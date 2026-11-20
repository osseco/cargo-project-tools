use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use semver::Version;

use crate::config::Tool;
use crate::install;
use crate::process;
use crate::project::Project;
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

/// What a tool's lock builds with its features: normal and build dependencies, for each of
/// `targets` (every target when `None`), so one answer holds on all of them. Runs `cargo
/// tree` on a scratch copy of the tool, which resolves features without compiling anything
/// or running build scripts.
pub fn built_packages(
    project: &Project,
    name: &str,
    tool: &Tool,
    targets: Option<&[String]>,
    offline: bool,
) -> Result<Built> {
    let lock_path = project.lock_path(name);
    if !lock_path.is_file() {
        bail!(
            "reading {}; add the tool with `cargo project-tools add`",
            lock_path.display()
        );
    }
    let scratch = source::scratch(project, name, &Origin::of(tool))?;
    scratch.put_lock(&lock_path)?;
    let features = install::feature_args(tool);
    let selection = Selection {
        features: &features,
        targets,
    };
    tree(&scratch.manifest_path(), selection, &[], offline, name)
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
