use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use cargo_lock::Lockfile;

use crate::process;

pub fn load(path: &Path) -> Result<Lockfile> {
    Lockfile::load(path).with_context(|| format!("reading {}", path.display()))
}

pub fn parse(text: &str) -> Result<Lockfile> {
    text.parse().context("parsing Cargo.lock")
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
        println!("  (no changes)");
    }
    for c in changes {
        println!("{c}");
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
fn compatible(a: &semver::Version, b: &semver::Version) -> bool {
    match (a.major, b.major) {
        (0, 0) => match (a.minor, b.minor) {
            (0, 0) => a.patch == b.patch,
            (x, y) => x == y,
        },
        (x, y) => x == y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK_A: &str = r#"
version = 3

[[package]]
name = "tool"
version = "1.0.0"
dependencies = ["dep", "gone"]

[[package]]
name = "dep"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "gone"
version = "0.1.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    const LOCK_B: &str = r#"
version = 3

[[package]]
name = "tool"
version = "1.0.0"
dependencies = ["dep", "new"]

[[package]]
name = "dep"
version = "1.0.5"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "new"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    #[test]
    fn root_check() {
        let lock = parse(LOCK_A).unwrap();
        check_root(&lock, "tool", "1.0.0").unwrap();
        assert!(check_root(&lock, "tool", "1.0.1").is_err());
        assert!(check_root(&lock, "other", "1.0.0").is_err());
    }

    #[test]
    fn diffs_locks() {
        let changes = diff(&parse(LOCK_A).unwrap(), &parse(LOCK_B).unwrap());
        assert_eq!(
            changes,
            vec![
                Change::Updated(
                    "dep".into(),
                    "1.0.0".parse().unwrap(),
                    "1.0.5".parse().unwrap()
                ),
                Change::Removed("gone".into(), "0.1.0".parse().unwrap()),
                Change::Added("new".into(), "2.0.0".parse().unwrap()),
            ]
        );
    }

    #[test]
    fn carry_forward_candidates() {
        let out = newer_than(&parse(LOCK_B).unwrap(), &parse(LOCK_A).unwrap());
        assert_eq!(
            out,
            vec![(
                "dep".into(),
                "1.0.0".parse().unwrap(),
                "1.0.5".parse().unwrap()
            )]
        );
    }

    #[test]
    fn semver_compat() {
        let v = |s: &str| semver::Version::parse(s).unwrap();
        assert!(compatible(&v("1.0.0"), &v("1.9.0")));
        assert!(!compatible(&v("0.1.0"), &v("0.2.0")));
        assert!(compatible(&v("0.2.1"), &v("0.2.9")));
    }
}
