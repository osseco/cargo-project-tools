use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};

use crate::lock;
use crate::process;
use crate::project::Project;
use crate::source::Checkout;

/// A tool's dependencies, copied out of cargo's shared caches into a directory of our own.
pub struct Vendored {
    pub dir: PathBuf,
    /// Arguments that point cargo at the vendor directory instead of crates.io and git.
    pub config_args: Vec<String>,
}

/// Vendor the dependencies of the tool in `checkout` (its lock already in place) into
/// `.project-tools/targets/<tool>/vendor`. Cargo checks every package against the lock, and
/// nothing is compiled. Builds then use these copies, which we can review and change, rather
/// than `~/.cargo/registry`, which we must not touch.
pub fn vendor(
    project: &Project,
    name: &str,
    checkout: &Checkout,
    offline: bool,
) -> Result<Vendored> {
    let target = project.target_dir(name);
    let dir = target.join("vendor");
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    fs::create_dir_all(&target)?;
    let lock = lock::load(&checkout.lock_path())?;
    let has_deps = lock.packages.iter().any(|p| p.source.is_some());
    // Not `--quiet`: it also drops the source config. The progress on stderr tells the user
    // to edit their config, which doesn't apply here, so it's shown only on failure.
    let mut cmd = process::cargo();
    cmd.args([
        "vendor",
        "--locked",
        "--versioned-dirs",
        "--respect-source-config",
        "--manifest-path",
    ])
    .arg(checkout.manifest_path())
    .arg(&dir);
    if offline {
        cmd.arg("--offline");
    }
    let snippet = process::output_quiet(&mut cmd).context("vendoring dependencies")?;
    let config_args = match source_config(&snippet, has_deps)? {
        Some(text) => {
            let path = target.join("vendor-config.toml");
            fs::write(&path, text)?;
            vec!["--config".into(), path.to_string_lossy().into_owned()]
        }
        None => Vec::new(),
    };
    Ok(Vendored { dir, config_args })
}

/// When every vendored file was last modified, as far as cargo can tell.
const SETTLED: Duration = Duration::from_secs(946_684_800); // 2000-01-01

/// Give every vendored file the same old modification time. Vendoring copies them afresh,
/// so build scripts that watch their files would otherwise run again on every install and
/// rebuild what depends on them. Content changes from the review clean the build instead.
pub fn settle(vendored: &Vendored) -> Result<()> {
    fn walk(dir: &Path, time: SystemTime) -> Result<()> {
        for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                walk(&path, time)?;
            } else if kind.is_file() {
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .and_then(|f| f.set_modified(time))
                    .with_context(|| format!("setting the time of {}", path.display()))?;
            }
        }
        // Build scripts also watch directories, whose time is when they were created.
        open_dir(dir)
            .and_then(|d| d.set_modified(time))
            .with_context(|| format!("setting the time of {}", dir.display()))
    }
    if !vendored.dir.exists() {
        return Ok(());
    }
    walk(&vendored.dir, SystemTime::UNIX_EPOCH + SETTLED)
}

/// A directory handle that can have its times set.
#[cfg(unix)]
fn open_dir(dir: &Path) -> std::io::Result<fs::File> {
    fs::File::open(dir)
}

#[cfg(windows)]
fn open_dir(dir: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    fs::File::options()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)
}

/// The source replacement `cargo vendor` prints, checked to be only that. `None` when the
/// tool has no dependencies to vendor; with `has_deps` there must be one, or the build would
/// quietly use cargo's caches instead.
fn source_config(snippet: &str, has_deps: bool) -> Result<Option<String>> {
    if snippet.trim().is_empty() {
        if has_deps {
            bail!("`cargo vendor` printed no source config");
        }
        return Ok(None);
    }
    let doc: toml::Table = snippet
        .parse()
        .context("reading the source config `cargo vendor` printed")?;
    if doc.keys().any(|k| k != "source") {
        bail!("`cargo vendor` printed more than source replacement:\n{snippet}");
    }
    if !doc
        .get("source")
        .and_then(|s| s.get("vendored-sources"))
        .is_some_and(|v| v.get("directory").is_some())
    {
        bail!("`cargo vendor` printed no vendored source:\n{snippet}");
    }
    Ok(Some(snippet.to_string()))
}

#[cfg(test)]
#[path = "vendor_test.rs"]
mod tests;
