use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::checksum;
use crate::cli::BuildArgs;
use crate::config::{Build, SandboxMode, Tool};
use crate::graph;
use crate::lock;
use crate::output::{self, Record, say};
use crate::process;
use crate::project::Project;
use crate::review;
use crate::sandbox;
use crate::source::registry::sha256_hex as registry_hash;
use crate::source::{self, Dest, Origin};
use crate::toml_line;
use crate::vendor;

/// Where stamps lived before they moved to `bin/.stamps`.
const OLD_STAMP: &str = ".install-stamp.json";

/// What an installation was built from; a different stamp means rebuild.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Stamp {
    key: String,
    bins: Vec<String>,
}

/// What to do with `bin/<tool>.sha256` after building.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Checksums {
    /// Record the new binaries.
    Write,
    /// The new binaries must match what's recorded (`verify --repair`).
    Expect,
}

/// How far `install` goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Review, then build and install.
    Install,
    /// Check the build review, then stop: build and write nothing (`check`).
    Check,
    /// Review, then stop before building (`review`).
    Review,
    /// Show how changed build-time code differs from what was recorded, then stop
    /// (`review --diff`).
    Diff,
}

/// How `install` treats a tool beyond its build settings.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    /// Rebuild even when the stamp is up to date.
    pub force: bool,
    /// Record new or changed build-time code in the review file.
    pub auto_approve: bool,
    /// Decisions named on the command line (`review --approve` and the like).
    pub picks: &'a [review::Pick],
    /// Decisions carried over from other review files (`review --from`).
    pub from: &'a review::KnownMap,
    pub stage: Stage,
    pub checksums: Checksums,
    /// The targets build-time code is reviewed for; all when `None`.
    pub targets: Option<&'a [String]>,
}

/// Cargo only fingerprints files of path packages, so after an override or strip in vendored
/// sources changes it would reuse stale builds. Remove the build output when the applied
/// changes differ from last time; `src`, `vendor` and its config stay.
fn clean_if_changed(target_dir: &Path, changes: &str) -> Result<()> {
    let marker = target_dir.join("review-changes");
    let previous = fs::read_to_string(&marker).unwrap_or_default();
    if previous == changes {
        return Ok(());
    }
    for entry in fs::read_dir(target_dir)? {
        let entry = entry?;
        let keep = matches!(
            entry.file_name().to_str(),
            Some("src" | "vendor" | "vendor-config.toml")
        );
        if !keep {
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }
    }
    fs::write(marker, changes)?;
    Ok(())
}

/// The tool's feature selection as cargo arguments.
pub fn feature_args(tool: &Tool) -> Vec<String> {
    feature_flags(&tool.features, tool.default_features != Some(false))
}

/// `features` and `default_features` as cargo arguments.
pub fn feature_flags(features: &[String], default_features: bool) -> Vec<String> {
    let mut out = Vec::new();
    for f in features {
        out.extend(["--features".into(), f.clone()]);
    }
    if !default_features {
        out.push("--no-default-features".into());
    }
    out
}

/// `cargo install` arguments derived from the tool, its build settings and CLI flags.
/// The build's `--config` values that can change what cargo resolves and builds:
/// `build.rustflags` (a `--cfg` turns on platform-specific dependencies) and the user's own
/// `--config`. The build review and the audit resolve with them too, so they see what the
/// build builds.
pub fn resolve_config(build: &Build, extra: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(flags) = &build.rustflags {
        let arr: toml::Value = flags.clone().into();
        out.extend(["--config".into(), format!("build.rustflags={arr}")]);
    }
    for c in extra {
        out.extend(["--config".into(), c.clone()]);
    }
    out
}

pub fn cargo_args(tool: &Tool, build: &Build, args: &BuildArgs) -> Vec<String> {
    let mut out = feature_args(tool);
    for b in &tool.bins {
        out.extend(["--bin".into(), b.clone()]);
    }

    let profile = if args.debug {
        "dev".to_string()
    } else {
        args.profile
            .clone()
            .or_else(|| build.profile.clone())
            .unwrap_or_else(|| "release".into())
    };
    out.extend(["--profile".into(), profile.clone()]);
    for (key, value) in &build.profile_settings {
        out.extend([
            "--config".into(),
            format!("profile.{profile}.{key}={value}"),
        ]);
    }
    if let Some(t) = args.target.as_ref().or(build.target.as_ref()) {
        out.extend(["--target".into(), t.clone()]);
    }
    out.extend(resolve_config(build, &args.config));
    if let Some(j) = &args.jobs {
        out.extend(["--jobs".into(), j.clone()]);
    }
    for (on, flag) in [
        (args.offline, "--offline"),
        (args.frozen, "--frozen"),
        (args.quiet, "--quiet"),
        (args.keep_going, "--keep-going"),
        (args.timings, "--timings"),
    ] {
        if on {
            out.push(flag.into());
        }
    }
    for _ in 0..args.verbose {
        out.push("--verbose".into());
    }
    if let Some(c) = &args.color {
        out.extend(["--color".into(), c.clone()]);
    }
    out
}

/// Flags that don't change the output and so stay out of the stamp.
fn affects_output(arg: &str) -> bool {
    !matches!(
        arg,
        "--offline" | "--frozen" | "--quiet" | "--keep-going" | "--timings" | "--verbose"
    )
}

/// `review` is `review::file_digest`: a changed decision rebuilds the tool.
fn stamp_key(
    name: &str,
    tool: &Tool,
    lock_text: &str,
    review: &str,
    cargo_args: &[String],
    targets: Option<&[String]>,
) -> String {
    stamp_key_on(
        name,
        tool,
        lock_text,
        review,
        cargo_args,
        targets,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

/// The stamp sits in `bin/`, which may be restored from a cache built elsewhere, so the
/// platform is part of the key.
#[allow(clippy::too_many_arguments)]
fn stamp_key_on(
    name: &str,
    tool: &Tool,
    lock_text: &str,
    review: &str,
    cargo_args: &[String],
    targets: Option<&[String]>,
    os: &str,
    arch: &str,
) -> String {
    let mut parts = vec![
        name.to_string(),
        os.to_string(),
        arch.to_string(),
        tool.version.clone(),
        tool.git.clone().unwrap_or_default(),
        tool.rev.clone().unwrap_or_default(),
        tool.tag.clone().unwrap_or_default(),
        tool.branch.clone().unwrap_or_default(),
        registry_hash(lock_text.as_bytes()),
        review.to_string(),
        // What the review covered: changing it must review again.
        targets.map_or_else(|| "all".to_string(), |t| t.join(",")),
    ];
    let mut skip_next = false;
    for a in cargo_args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if a == "--jobs" || a == "--color" {
            skip_next = true;
            continue;
        }
        if affects_output(a) {
            parts.push(a.clone());
        }
    }
    registry_hash(parts.join("\0").as_bytes())
}

fn read_stamp(path: &Path) -> Option<Stamp> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Binaries cargo recorded for `name` in `.crates2.json`.
fn installed_bins(project: &Project, name: &str, version: &str) -> Vec<String> {
    #[derive(Deserialize)]
    struct Crates2 {
        installs: BTreeMap<String, Install>,
    }
    #[derive(Deserialize)]
    struct Install {
        bins: Vec<String>,
    }
    let path = project.install_root().join(".crates2.json");
    let Some(c2) = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Crates2>(&s).ok())
    else {
        return Vec::new();
    };
    let prefix = format!("{name} {version} ");
    c2.installs
        .into_iter()
        .filter(|(k, _)| k.starts_with(&prefix))
        .flat_map(|(_, v)| v.bins)
        .collect()
}

pub fn install(
    project: &Project,
    name: &str,
    tool: &Tool,
    build: &Build,
    args: &BuildArgs,
    opts: Options,
) -> Result<()> {
    let lock_path = project.lock_path(name);
    let lock_text = fs::read_to_string(&lock_path).with_context(|| {
        format!(
            "reading {}; add the tool with `cargo project-tools add`",
            lock_path.display()
        )
    })?;
    lock::check_root(&lock::parse(&lock_text)?, name, &tool.version)?;

    let cargo_args = cargo_args(tool, build, args);
    let review_digest = review::file_digest(project, name)?;
    let key = stamp_key(
        name,
        tool,
        &lock_text,
        &review_digest,
        &cargo_args,
        opts.targets,
    );
    let target_dir = project.target_dir(name);
    let stamp_path = project.stamp_path(name);
    let choice = sandbox_choice(project, name, tool, build, args);
    // `check` probes the sandbox even for a tool that's up to date: CI that requires one
    // learns it's missing before it has anything to build.
    if opts.stage == Stage::Check {
        sandbox::choose(name, &choice)?;
    }
    // Approving reviews now, even when the binaries are current.
    if !opts.force
        && !opts.auto_approve
        && opts.picks.is_empty()
        && opts.from.is_empty()
        && let Some(stamp) = read_stamp(&stamp_path)
        && stamp.key == key
        && !stamp.bins.is_empty()
        && stamp
            .bins
            .iter()
            .all(|b| project.bin_dir().join(b).is_file())
        && checksum::check(project, name).is_ok_and(|m| m.is_empty())
    {
        if matches!(opts.stage, Stage::Review | Stage::Diff) {
            // The stamp covers the lock and the review file it was built with.
            say!("{name} {} is up to date; nothing to review", tool.version);
        } else {
            say!("{name} {} is up to date", tool.version);
            // So CI can tell a cache hit built nothing, without comparing file times.
            output::emit(Record::UpToDate {
                tool: name.to_string(),
                tool_version: tool.version.clone(),
                bins: stamp.bins,
            });
        }
        return Ok(());
    }

    match opts.stage {
        Stage::Install => say!("Installing {name} {}", tool.version),
        Stage::Check => say!("Checking {name} {}", tool.version),
        Stage::Review => say!("Reviewing {name} {}", tool.version),
        Stage::Diff => say!("Comparing {name} {}", tool.version),
    }
    // Before anything is downloaded, so `require` without a sandbox fails early.
    let sandbox = match opts.stage {
        Stage::Install => sandbox::choose(name, &choice)?,
        _ => None,
    };
    let checkout = source::checkout(
        project,
        name,
        &Origin::of(tool),
        Dest::Fixed(target_dir.join("src")),
    )?;
    anyhow::ensure!(
        checkout.version == tool.version,
        "source of {name} is version {}, but project-tools.toml wants {}",
        checkout.version,
        tool.version
    );
    checkout.put_lock(&lock_path)?;
    let vendored = vendor::vendor(project, name, &checkout, args.offline, &[])?;
    let features = feature_args(tool);
    let config = resolve_config(build, &args.config);
    let selection = graph::Selection {
        features: &features,
        targets: opts.targets,
        config: &config,
    };
    if opts.stage == Stage::Diff {
        return review::diff(project, name, selection, &checkout, &vendored, args.offline);
    }
    if opts.stage == Stage::Check {
        return review::check_only(project, name, selection, &checkout, &vendored, args.offline);
    }
    let changes = review::review(
        project,
        name,
        selection,
        &checkout,
        &vendored,
        args.offline,
        review::Approval::new(
            opts.auto_approve,
            opts.picks,
            opts.from,
            opts.stage == Stage::Review,
        ),
    )?;
    clean_if_changed(&target_dir, &changes)?;
    vendor::settle(&vendored)?;
    if opts.stage == Stage::Review {
        say!("{name} {}: build review done", tool.version);
        return Ok(());
    }
    // `--auto-approve` may have just written the review file.
    let key = stamp_key(
        name,
        tool,
        &lock_text,
        &review::file_digest(project, name)?,
        &cargo_args,
        opts.targets,
    );

    // The tracking key contains the source path, so binaries from an earlier install are
    // owned by a "different" package: --force lets cargo replace them. Our stamp decides
    // whether to build at all.
    let cmd = build_command(
        project,
        &target_dir,
        &checkout.package_dir,
        &vendored.config_args,
        &cargo_args,
    )?;
    // What `cargo install` writes: the build, the binaries and its records of them.
    let writable = sandbox::Writable {
        dirs: vec![target_dir.clone(), project.bin_dir()],
        files: [".crates.toml", ".crates2.json"]
            .map(|f| project.install_root().join(f))
            .to_vec(),
    };
    let mut cmd = match &sandbox {
        Some(kind) => {
            say!("Building {name} {} in {}", tool.version, kind.name());
            sandbox::wrap(cmd, kind, &writable, &target_dir.join("tmp"))?
        }
        None => {
            say!("Building {name} {} without a sandbox", tool.version);
            cmd
        }
    };
    process::run(&mut cmd)?;

    let bins = installed_bins(project, name, &tool.version);
    say!(
        "{name}: {} in {}",
        bins.join(", "),
        project
            .bin_dir()
            .strip_prefix(&project.root)
            .unwrap_or(&project.bin_dir())
            .display()
    );
    output::emit(Record::Built {
        tool: name.to_string(),
        tool_version: tool.version.clone(),
        sandbox: sandbox
            .as_ref()
            .map_or("none", sandbox::Kind::name)
            .to_string(),
        bins: bins.clone(),
    });
    match opts.checksums {
        Checksums::Write => checksum::write(project, name, &bins)?,
        Checksums::Expect => expect_recorded(project, name, &bins)?,
    }
    if let Some(dir) = stamp_path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(
        &stamp_path,
        serde_json::to_string_pretty(&Stamp { key, bins })?,
    )?;
    let old = target_dir.join(OLD_STAMP);
    if old.exists() {
        fs::remove_file(&old).with_context(|| format!("removing {}", old.display()))?;
    }
    Ok(())
}

/// The `cargo install` that builds the tool in `package_dir`.
fn build_command(
    project: &Project,
    target_dir: &Path,
    package_dir: &Path,
    vendor_args: &[String],
    cargo_args: &[String],
) -> Result<Command> {
    let mut cmd = process::cargo();
    process::git_ceiling(&mut cmd, target_dir)?;
    cmd.args(["install", "--locked", "--force"])
        .arg("--path")
        .arg(package_dir)
        .arg("--root")
        .arg(project.install_root())
        .arg("--target-dir")
        .arg(target_dir)
        .args(vendor_args)
        .args(cargo_args);
    // The build uses only the vendored sources: it never needs the network, so cargo is
    // told so, sandbox or not.
    if !cargo_args.iter().any(|a| a == "--offline") {
        cmd.arg("--offline");
    }
    // Cargo warns when the install root's `bin` isn't on PATH. Appended, so nothing there
    // takes the place of a system binary a build script runs.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<_> = std::env::split_paths(&path).collect();
    paths.push(project.bin_dir());
    cmd.env("PATH", std::env::join_paths(paths)?);
    Ok(cmd)
}

/// The sandbox mode for building `name`: `--sandbox` (or `--no-sandbox`), else the merged
/// `[build]` setting for this OS, with the line in project-tools.toml that set it.
fn sandbox_choice(
    project: &Project,
    name: &str,
    tool: &Tool,
    build: &Build,
    args: &BuildArgs,
) -> sandbox::Choice {
    let cli = if args.no_sandbox {
        Some(SandboxMode::Off)
    } else {
        args.sandbox
    };
    if let Some(mode) = cli {
        return sandbox::Choice {
            mode,
            from_cli: true,
            line: None,
        };
    }
    let os = std::env::consts::OS;
    let own = tool.build.as_ref();
    // As `Build::sandbox_for` picks: an OS entry first, the tool's own before the global one.
    let (path, key): (Vec<&str>, &str) = if own.is_some_and(|b| b.sandbox_os.contains_key(os)) {
        (vec!["tool", name, "build", "sandbox-os"], os)
    } else if build.sandbox_os.contains_key(os) {
        (vec!["build", "sandbox-os"], os)
    } else if own.is_some_and(|b| b.sandbox.is_some()) {
        (vec!["tool", name, "build"], "sandbox")
    } else {
        (vec!["build"], "sandbox")
    };
    let line = fs::read_to_string(project.config_path())
        .ok()
        .and_then(|text| toml_line::key_line(&text, &path, key));
    sandbox::Choice {
        mode: build.sandbox_for(os),
        from_cli: false,
        line,
    }
}

/// Fails, removing the tool's binaries so nothing unverified stays on PATH, when the
/// rebuilt binaries aren't the recorded ones.
fn expect_recorded(project: &Project, name: &str, bins: &[String]) -> Result<()> {
    let recorded = checksum::read(project, name)?;
    let mut problems: Vec<String> = checksum::check(project, name)?
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut built = bins.to_vec();
    built.sort();
    let mut expected: Vec<String> = recorded.iter().map(|(b, _)| b.clone()).collect();
    expected.sort();
    if built != expected {
        problems.push(format!(
            "built {} but {} were recorded",
            built.join(", "),
            expected.join(", ")
        ));
    }
    if problems.is_empty() {
        return Ok(());
    }
    for bin in built.iter().chain(&expected) {
        let path = project.bin_dir().join(bin);
        if path.is_file() {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    anyhow::bail!(
        "rebuild of {name} does not match its recorded checksum ({}); its binaries were removed",
        problems.join("; ")
    )
}

#[cfg(test)]
#[path = "install_test.rs"]
mod tests;
