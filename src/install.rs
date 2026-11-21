use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::checksum;
use crate::cli::BuildArgs;
use crate::config::{Build, Tool};
use crate::graph;
use crate::lock;
use crate::output::say;
use crate::process;
use crate::project::Project;
use crate::review;
use crate::source::registry::sha256_hex as registry_hash;
use crate::source::{self, Dest, Origin};
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

/// How `install` treats a tool beyond its build settings.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    /// Rebuild even when the stamp is up to date.
    pub force: bool,
    /// Record new or changed build-time code in the review file.
    pub approve_build: bool,
    /// Stop after checking the build review: build and write nothing.
    pub check: bool,
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
    // Approving reviews now, even when the binaries are current.
    if !opts.force
        && !opts.approve_build
        && let Some(stamp) = read_stamp(&stamp_path)
        && stamp.key == key
        && !stamp.bins.is_empty()
        && stamp
            .bins
            .iter()
            .all(|b| project.bin_dir().join(b).is_file())
        && checksum::check(project, name).is_ok_and(|m| m.is_empty())
    {
        say!("{name} {} is up to date", tool.version);
        return Ok(());
    }

    if opts.check {
        say!("Checking {name} {}", tool.version);
    } else {
        say!("Installing {name} {}", tool.version);
    }
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
    if opts.check {
        return review::check_only(project, name, selection, &checkout, &vendored, args.offline);
    }
    let changes = review::review(
        project,
        name,
        selection,
        &checkout,
        &vendored,
        args.offline,
        opts.approve_build,
    )?;
    clean_if_changed(&target_dir, &changes)?;
    vendor::settle(&vendored)?;
    // `--approve-build` may have just written the review file.
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
    process::run(
        process::cargo()
            .args(["install", "--locked", "--force"])
            .arg("--path")
            .arg(&checkout.package_dir)
            .arg("--root")
            .arg(project.install_root())
            .arg("--target-dir")
            .arg(&target_dir)
            .args(&vendored.config_args)
            .args(&cargo_args),
    )?;

    let bins = installed_bins(project, name, &tool.version);
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
