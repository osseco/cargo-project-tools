use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::config::Tool;
use crate::install;
use crate::process;
use crate::project::Project;
use crate::source::registry::sha256_hex;
use crate::source::{self, Origin};

/// Packages, by name and version, that cargo builds for a tool.
pub type Built = BTreeSet<(String, Version)>;

/// Which part of a tool's lock is built: its feature flags, and the targets (all when
/// `None`).
#[derive(Debug, Clone, Copy)]
pub struct Selection<'a> {
    pub features: &'a [String],
    pub targets: Option<&'a [String]>,
}

/// A remembered `built_packages` result, kept next to the install stamp.
#[derive(Serialize, Deserialize)]
struct Cached {
    key: String,
    packages: Vec<(String, String)>,
}

/// What a `built_packages` result depends on: the tool's source, its lock, its features and
/// the targets.
fn key(name: &str, tool: &Tool, lock_text: &str, selection: Selection) -> String {
    let targets = selection
        .targets
        .map_or_else(|| "all".to_string(), |t| t.join(","));
    let parts = [
        name,
        &tool.version,
        tool.git.as_deref().unwrap_or_default(),
        tool.rev.as_deref().unwrap_or_default(),
        &sha256_hex(lock_text.as_bytes()),
        &selection.features.join(" "),
        &targets,
    ];
    sha256_hex(parts.join("\0").as_bytes())
}

/// The remembered result at `path` when it's for `key` and lists the root; anything else is
/// recomputed rather than trusted.
fn read_cached(path: &Path, key: &str, root: &str) -> Option<Built> {
    let cached: Cached = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let built = cached
        .packages
        .into_iter()
        .map(|(n, v)| Some((n, Version::parse(&v).ok()?)))
        .collect::<Option<Built>>()?;
    (cached.key == key && built.iter().any(|(n, _)| n == root)).then_some(built)
}

fn write_cached(path: &Path, key: &str, built: &Built) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let cached = Cached {
        key: key.to_string(),
        packages: built
            .iter()
            .map(|(n, v)| (n.clone(), v.to_string()))
            .collect(),
    };
    fs::write(path, serde_json::to_string_pretty(&cached)?)
        .with_context(|| format!("writing {}", path.display()))
}

/// What a tool's lock builds with its features: normal and build dependencies, for each of
/// `targets` (every target when `None`), so one answer holds on all of them. Runs `cargo
/// tree` on a scratch copy of the tool, which resolves features without compiling anything
/// or running build scripts, but downloads the dependencies. The result is remembered in
/// `bin/.stamps`, so it only runs again when the lock, features or targets change.
pub fn built_packages(
    project: &Project,
    name: &str,
    tool: &Tool,
    targets: Option<&[String]>,
    offline: bool,
) -> Result<Built> {
    let lock_path = project.lock_path(name);
    let Ok(lock_text) = fs::read_to_string(&lock_path) else {
        bail!(
            "reading {}; add the tool with `cargo project-tools add`",
            lock_path.display()
        );
    };
    let features = install::feature_args(tool);
    let selection = Selection {
        features: &features,
        targets,
    };
    let key = key(name, tool, &lock_text, selection);
    let cache = project.built_path(name);
    if let Some(built) = read_cached(&cache, &key, name) {
        return Ok(built);
    }
    let scratch = source::scratch(project, name, &Origin::of(tool))?;
    scratch.put_lock(&lock_path)?;
    let built = tree(&scratch.manifest_path(), selection, &[], offline, name)?;
    write_cached(&cache, &key, &built)?;
    Ok(built)
}

/// `cargo tree` for the package at `manifest`; `extra` goes to cargo as is (e.g. a vendored
/// source config).
pub fn tree(
    manifest: &Path,
    selection: Selection,
    extra: &[String],
    offline: bool,
    root: &str,
) -> Result<Built> {
    let mut cmd = process::cargo();
    cmd.args([
        "tree",
        "--locked",
        "--quiet",
        "--edges",
        "normal,build",
        "--prefix",
        "none",
        "--format",
        "{p}",
        "--manifest-path",
    ])
    .arg(manifest)
    .args(selection.features)
    .args(target_args(selection.targets))
    .args(extra);
    if offline {
        cmd.arg("--offline");
    }
    let out = process::output(&mut cmd).context("listing the packages cargo builds")?;
    parse_tree(&out, root)
}

/// `--target` for each triple, or `--target all`.
fn target_args(targets: Option<&[String]>) -> Vec<String> {
    match targets {
        Some(list) => list
            .iter()
            .flat_map(|t| ["--target".to_string(), t.clone()])
            .collect(),
        None => vec!["--target".into(), "all".into()],
    }
}

/// Parse `cargo tree --prefix none --format {p}` lines (`name v1.2.3 (source) (*)`). Fails
/// rather than return a set that may miss packages.
fn parse_tree(out: &str, root: &str) -> Result<Built> {
    let mut built = Built::new();
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        let mut parts = line.split_whitespace();
        let (Some(name), Some(version)) = (parts.next(), parts.next()) else {
            bail!("unexpected `cargo tree` line: {line}");
        };
        let version = version
            .strip_prefix('v')
            .and_then(|v| Version::parse(v).ok())
            .with_context(|| format!("unexpected `cargo tree` line: {line}"))?;
        built.insert((name.to_string(), version));
    }
    if !built.iter().any(|(name, _)| name == root) {
        bail!("`cargo tree` did not list {root}");
    }
    Ok(built)
}

#[cfg(test)]
#[path = "graph_test.rs"]
mod tests;
