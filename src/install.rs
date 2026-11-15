use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cli::InstallArgs;
use crate::config::{Build, Tool};
use crate::lock;
use crate::process;
use crate::project::Project;
use crate::source::registry::sha256_hex as registry_hash;
use crate::source::{self, Dest, Origin};

const STAMP: &str = ".install-stamp.json";

/// What an installation was built from; a different stamp means rebuild.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Stamp {
    key: String,
    bins: Vec<String>,
}

/// `cargo install` arguments derived from the tool, its build settings and CLI flags.
pub fn cargo_args(tool: &Tool, build: &Build, args: &InstallArgs) -> Vec<String> {
    let mut out = Vec::new();
    for f in &tool.features {
        out.extend(["--features".into(), f.clone()]);
    }
    if tool.default_features == Some(false) {
        out.push("--no-default-features".into());
    }
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
    if let Some(flags) = &build.rustflags {
        let arr: toml::Value = flags.clone().into();
        out.extend(["--config".into(), format!("build.rustflags={arr}")]);
    }
    if let Some(t) = args.target.as_ref().or(build.target.as_ref()) {
        out.extend(["--target".into(), t.clone()]);
    }
    for c in &args.config {
        out.extend(["--config".into(), c.clone()]);
    }
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

fn stamp_key(name: &str, tool: &Tool, lock_text: &str, cargo_args: &[String]) -> String {
    let mut parts = vec![
        name.to_string(),
        tool.version.clone(),
        tool.git.clone().unwrap_or_default(),
        tool.rev.clone().unwrap_or_default(),
        tool.tag.clone().unwrap_or_default(),
        tool.branch.clone().unwrap_or_default(),
        registry_hash(lock_text.as_bytes()),
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
    args: &InstallArgs,
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
    let key = stamp_key(name, tool, &lock_text, &cargo_args);
    let target_dir = project.target_dir(name);
    let stamp_path = target_dir.join(STAMP);
    if !args.force
        && let Some(stamp) = read_stamp(&stamp_path)
        && stamp.key == key
        && !stamp.bins.is_empty()
        && stamp
            .bins
            .iter()
            .all(|b| project.bin_dir().join(b).is_file())
    {
        println!("{name} {} is up to date", tool.version);
        return Ok(());
    }

    println!("Installing {name} {}", tool.version);
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
            .args(&cargo_args),
    )?;

    let bins = installed_bins(project, name, &tool.version);
    fs::write(
        &stamp_path,
        serde_json::to_string_pretty(&Stamp { key, bins })?,
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "install_test.rs"]
mod tests;
