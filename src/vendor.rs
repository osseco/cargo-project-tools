use std::fs;

use anyhow::{Context, Result, bail};

use crate::lock;
use crate::process;
use crate::project::Project;
use crate::source::Checkout;

/// A tool's dependencies, copied out of cargo's shared caches into a directory of our own.
pub struct Vendored {
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
    Ok(Vendored { config_args })
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
