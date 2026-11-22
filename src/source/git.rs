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

/// Not a directory, so git finds no hooks in it.
#[cfg(windows)]
const NO_HOOKS: &str = "NUL";
#[cfg(not(windows))]
const NO_HOOKS: &str = "/dev/null";

fn git() -> Command {
    let mut cmd = Command::new("git");
    // Never prompt for credentials and refuse non-https transports, including in submodules.
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "https");
    // Run no command a repository's config names: no hooks, no fsmonitor.
    cmd.args(["-c", "core.fsmonitor=false", "-c"])
        .arg(format!("core.hooksPath={NO_HOOKS}"));
    cmd
}

/// Next to `dest`: which clone `fresh_clone` made there. Outside the work tree, so nothing
/// in the source can write it.
fn marker(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".clone");
    dest.with_file_name(name)
}

fn marker_text(url: &str, rev: &str) -> String {
    format!("{url}\n{rev}\n")
}

fn reject_option_like(value: &str) -> Result<()> {
    if value.starts_with('-') {
        bail!("invalid git ref `{value}`");
    }
    Ok(())
}

/// Clone `url` into `dest` (replaced), check out the requested ref and find package `name`.
/// A `dest` already at the pinned `rev` is reused instead.
pub fn clone(url: &str, reference: &GitRef, name: &str, dest: &Path) -> Result<Clone> {
    ensure_https(url)?;
    if !reference
        .rev
        .as_deref()
        .is_some_and(|rev| reuse(dest, url, rev))
    {
        fresh_clone(url, reference, dest)?;
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

/// Reset an existing clone that's already at commit `rev`, rewriting only files that differ
/// from it, so unchanged files keep their mtimes and cargo's fingerprints stay fresh. Only a
/// clone `fresh_clone` made of `url` at `rev` is reused: a `.git` that came some other way
/// (inside a crate archive, say) could hold config that runs commands. False when there's
/// nothing to reuse or git fails.
fn reuse(dest: &Path, url: &str, rev: &str) -> bool {
    if rev.len() != 40 || !rev.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let ours = fs::read_to_string(marker(dest)).is_ok_and(|m| m == marker_text(url, rev));
    let real_dir = dest
        .join(".git")
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_dir());
    if !ours || !real_dir {
        return false;
    }
    let at_rev = process::output(git().arg("-C").arg(dest).args(["rev-parse", "HEAD"]))
        .is_ok_and(|head| head.trim().eq_ignore_ascii_case(rev));
    at_rev
        && process::output(
            git()
                .arg("-C")
                .arg(dest)
                .args(["reset", "--quiet", "--hard"]),
        )
        .is_ok()
        && process::output(git().arg("-C").arg(dest).args(["clean", "-ffdxq"])).is_ok()
}

fn fresh_clone(url: &str, reference: &GitRef, dest: &Path) -> Result<()> {
    let marker = marker(dest);
    if marker.exists() {
        fs::remove_file(&marker)?;
    }
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
        fs::write(&marker, marker_text(url, rev))?;
    }
    Ok(())
}

/// The url `dir`, a clone of ours, was cloned from.
pub fn origin_url(dir: &Path) -> Result<String> {
    let url = process::output(
        git()
            .arg("-C")
            .arg(dir)
            .args(["remote", "get-url", "origin"]),
    )?;
    let url = url.trim().to_string();
    ensure_https(&url)?;
    Ok(url)
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

#[cfg(test)]
#[path = "git_test.rs"]
mod tests;
