use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::config::ensure_https;
use crate::process;

#[derive(Debug, Clone, Default)]
pub struct GitRef {
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub rev: Option<String>,
}

pub struct Clone {
    pub commit: String,
    pub workspace_root: PathBuf,
    pub package_dir: PathBuf,
    pub version: String,
}

fn git() -> Command {
    let mut cmd = Command::new("git");
    // Never prompt for credentials and refuse non-https transports, including in submodules.
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "https");
    cmd
}

fn reject_option_like(value: &str) -> Result<()> {
    if value.starts_with('-') {
        bail!("invalid git ref `{value}`");
    }
    Ok(())
}

/// Clone `url` into `dest` (replaced), check out the requested ref and find package `name`.
pub fn clone(url: &str, reference: &GitRef, name: &str, dest: &Path) -> Result<Clone> {
    ensure_https(url)?;
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    let mut cmd = git();
    cmd.args(["clone", "--quiet"]);
    let named = reference.branch.as_ref().or(reference.tag.as_ref());
    if reference.rev.is_none() {
        cmd.args(["--depth", "1"]);
        if let Some(r) = named {
            reject_option_like(r)?;
            cmd.arg("--branch").arg(r);
        }
    }
    cmd.arg("--").arg(url).arg(dest);
    process::run(&mut cmd)?;

    if let Some(rev) = &reference.rev {
        reject_option_like(rev)?;
        process::run(
            git()
                .arg("-C")
                .arg(dest)
                .args(["checkout", "--quiet", "--detach"])
                .arg(rev),
        )?;
    }
    let commit = process::output(git().arg("-C").arg(dest).args(["rev-parse", "HEAD"]))?
        .trim()
        .to_string();

    let (workspace_root, package_dir, version) = locate_package(dest, name)?;
    Ok(Clone {
        commit,
        workspace_root,
        package_dir,
        version,
    })
}

#[derive(Deserialize)]
struct Metadata {
    workspace_root: PathBuf,
    packages: Vec<MetaPackage>,
}

#[derive(Deserialize)]
struct MetaPackage {
    name: String,
    version: String,
    manifest_path: PathBuf,
}

fn locate_package(dir: &Path, name: &str) -> Result<(PathBuf, PathBuf, String)> {
    let manifest = dir.join("Cargo.toml");
    let json = process::output(
        process::cargo()
            .args([
                "metadata",
                "--no-deps",
                "--format-version",
                "1",
                "--offline",
            ])
            .arg("--manifest-path")
            .arg(&manifest),
    )
    .with_context(|| format!("reading workspace metadata in {}", dir.display()))?;
    let meta: Metadata = serde_json::from_str(&json)?;
    let pkg = meta
        .packages
        .into_iter()
        .find(|p| p.name == name)
        .with_context(|| format!("package `{name}` not found in repository"))?;
    let package_dir = pkg
        .manifest_path
        .parent()
        .context("bad manifest path")?
        .to_path_buf();
    Ok((meta.workspace_root, package_dir, pkg.version))
}
