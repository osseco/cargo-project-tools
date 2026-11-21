use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use cargo_lock::Lockfile;

use crate::output::say;
use crate::process;

pub fn load(path: &Path) -> Result<Lockfile> {
    let lock = Lockfile::load(path).with_context(|| format!("reading {}", path.display()))?;
    check_sources(&lock).with_context(|| format!("checking {}", path.display()))?;
    Ok(lock)
}

pub fn parse(text: &str) -> Result<Lockfile> {
    let lock = text.parse().context("parsing Cargo.lock")?;
    check_sources(&lock)?;
    Ok(lock)
}

/// Every source in the lock must be fetched over https: `git+ssh://`, `git://`, `file://`
/// and plain http are refused, as for our own downloads and clones.
pub fn check_sources(lock: &Lockfile) -> Result<()> {
    for p in &lock.packages {
        let Some(source) = &p.source else { continue };
        let text = source.to_string();
        let https = text
            .split_once('+')
            .is_some_and(|(_, url)| url.starts_with("https://"));
        if !https {
            bail!(
                "{} {} comes from {text}; only https sources are allowed",
                p.name,
                p.version
            );
        }
    }
    Ok(())
}

/// The lock must belong to `name` at `version`: the package without a source.
pub fn check_root(lock: &Lockfile, name: &str, version: &str) -> Result<()> {
    let roots: Vec<_> = lock
        .packages
        .iter()
        .filter(|p| p.source.is_none() && p.name.as_str() == name)
        .collect();
    match roots.as_slice() {
        [p] if p.version.to_string() == version => Ok(()),
        [p] => bail!(
            "Cargo.lock is for {name} {}, but project-tools.toml wants {version}",
            p.version
        ),
        [] => bail!("Cargo.lock does not contain {name} as a local package"),
        _ => bail!("Cargo.lock contains {name} more than once"),
    }
}

/// name → versions
fn versions(lock: &Lockfile) -> BTreeMap<String, BTreeSet<semver::Version>> {
    let mut map: BTreeMap<String, BTreeSet<semver::Version>> = BTreeMap::new();
    for p in &lock.packages {
        map.entry(p.name.as_str().to_string())
            .or_default()
            .insert(p.version.clone());
    }
    map
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Added(String, semver::Version),
    Removed(String, semver::Version),
    Updated(String, semver::Version, semver::Version),
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Change::Added(n, v) => write!(f, "  + {n} {v}"),
            Change::Removed(n, v) => write!(f, "  - {n} {v}"),
            Change::Updated(n, a, b) => write!(f, "  ~ {n} {a} -> {b}"),
        }
    }
}

pub fn diff(old: &Lockfile, new: &Lockfile) -> Vec<Change> {
    let (old, new) = (versions(old), versions(new));
    let names: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    let empty = BTreeSet::new();
    let mut out = Vec::new();
    for name in names {
        let a = old.get(name).unwrap_or(&empty);
        let b = new.get(name).unwrap_or(&empty);
        let gone: Vec<_> = a.difference(b).collect();
        let came: Vec<_> = b.difference(a).collect();
        if let ([from], [to]) = (gone.as_slice(), came.as_slice()) {
            out.push(Change::Updated(
                name.clone(),
                (*from).clone(),
                (*to).clone(),
            ));
            continue;
        }
        out.extend(
            gone.into_iter()
                .map(|v| Change::Removed(name.clone(), v.clone())),
        );
        out.extend(
            came.into_iter()
                .map(|v| Change::Added(name.clone(), v.clone())),
        );
    }
    out
}

pub fn print_diff(changes: &[Change]) {
    if changes.is_empty() {
        say!("  (no changes)");
    }
    for c in changes {
        say!("{c}");
    }
}

/// Run `cargo update` in the project at `manifest`.
pub fn cargo_update(manifest: &Path, args: &[String]) -> Result<()> {
    process::run(
        process::cargo()
            .arg("update")
            .arg("--manifest-path")
            .arg(manifest)
            .args(args),
    )
}

/// Where a new tool's lock came from.
#[derive(Debug, PartialEq, Eq)]
pub enum Start {
    Shipped,
    Generated,
}

/// The lock a new tool starts from: the one at `lock_path` if the source ships it, otherwise a
/// fresh one from `cargo generate-lockfile`. That only reads the index: nothing is compiled
/// and no build script runs. `cargo_args` go to cargo as is.
pub fn shipped_or_generated(
    manifest: &Path,
    lock_path: &Path,
    cargo_args: &[String],
) -> Result<(String, Start)> {
    let start = if lock_path.is_file() {
        Start::Shipped
    } else {
        process::run(
            process::cargo()
                .args(["generate-lockfile", "--quiet", "--manifest-path"])
                .arg(manifest)
                .args(cargo_args),
        )?;
        Start::Generated
    };
    let text = std::fs::read_to_string(lock_path)
        .with_context(|| format!("reading {}", lock_path.display()))?;
    Ok((text, start))
}

/// Resolve the lock at `lock_path` for the project at `manifest`, starting from `ours`: cargo
/// keeps every locked version that still fits and only resolves what the manifest now needs.
pub fn reseed(manifest: &Path, lock_path: &Path, ours: &Path, cargo_args: &[String]) -> Result<()> {
    std::fs::copy(ours, lock_path)
        .with_context(|| format!("copying {} to {}", ours.display(), lock_path.display()))?;
    let mut args = vec!["--quiet".to_string(), "--workspace".to_string()];
    args.extend_from_slice(cargo_args);
    cargo_update(manifest, &args)
}

/// Versions of registry packages in `lock` that are newer than all versions of the same
/// package in `new`, and semver compatible with one of them: candidates to carry forward.
pub fn newer_than(
    old: &Lockfile,
    new: &Lockfile,
) -> Vec<(String, semver::Version, semver::Version)> {
    let newv = versions(new);
    let mut out = Vec::new();
    for p in old.packages.iter().filter(|p| p.source.is_some()) {
        let Some(in_new) = newv.get(p.name.as_str()) else {
            continue;
        };
        if in_new.contains(&p.version) {
            continue;
        }
        if let Some(lower) = in_new
            .iter()
            .filter(|v| **v < p.version && compatible(v, &p.version))
            .max()
        {
            out.push((
                p.name.as_str().to_string(),
                lower.clone(),
                p.version.clone(),
            ));
        }
    }
    out
}

/// Cargo's notion of semver compatibility (same left-most non-zero component).
pub fn compatible(a: &semver::Version, b: &semver::Version) -> bool {
    match (a.major, b.major) {
        (0, 0) => match (a.minor, b.minor) {
            (0, 0) => a.patch == b.patch,
            (x, y) => x == y,
        },
        (x, y) => x == y,
    }
}

#[cfg(test)]
#[path = "lock_test.rs"]
mod tests;
