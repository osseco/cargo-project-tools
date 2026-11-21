//! Reading files for the review screen, made safe to draw, and writing overrides.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::UNREVIEWED;
use crate::review::{Item, Kind, safe_relative};
use crate::sanitize::sanitize;

/// What reading a file gave.
pub(super) enum Content {
    /// Sanitized text and how many characters had to be replaced.
    Text(String, usize),
    Binary(u64),
    Link(PathBuf),
}

pub(super) fn read_content(path: &Path) -> Result<Content> {
    let meta = path
        .symlink_metadata()
        .with_context(|| format!("reading {}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Ok(Content::Link(fs::read_link(path)?));
    }
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(match String::from_utf8(bytes) {
        Ok(text) => {
            let (text, replaced) = sanitize(&text);
            Content::Text(text, replaced)
        }
        Err(e) => Content::Binary(e.into_bytes().len() as u64),
    })
}

/// A file that can't be shown as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Warn {
    None,
    Unsafe,
    Binary,
}

impl Warn {
    pub(super) fn of(path: &Path) -> Warn {
        match read_content(path) {
            Ok(Content::Text(_, 0)) => Warn::None,
            Ok(Content::Text(..)) => Warn::Unsafe,
            _ => Warn::Binary,
        }
    }

    pub(super) fn mark(self) -> &'static str {
        match self {
            Warn::None => "",
            Warn::Unsafe => " ⚠",
            Warn::Binary => " (binary)",
        }
    }
}

/// Where the override of `item` goes, relative to the tool's crate directory.
pub fn override_rel(item: &Item) -> Result<String> {
    let rel = match item.kind {
        Kind::BuildScript => format!("overrides/{}/{}", item.package, item.path),
        Kind::CargoConfig => format!("overrides/cargo-config/{}", item.path),
        Kind::ProcMacro => bail!("a proc macro can't be overridden"),
    };
    safe_relative(&rel)?;
    Ok(rel)
}

/// `bytes` with every line commented out under the UNREVIEWED header, so it does nothing
/// until someone edits it.
pub(super) fn commented_copy(bytes: &[u8], rust: bool) -> Vec<u8> {
    let prefix: &[u8] = if rust { b"// " } else { b"# " };
    let mut out = Vec::with_capacity(bytes.len() * 2);
    out.extend_from_slice(prefix);
    out.extend_from_slice(UNREVIEWED.as_bytes());
    out.push(b'\n');
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        out.extend_from_slice(prefix);
        out.extend_from_slice(line);
    }
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    out
}

/// Fail when `path` is a link or isn't a directory; create it when it's missing.
fn plain_dir(path: &Path) -> Result<()> {
    match path.symlink_metadata() {
        Ok(m) if m.file_type().is_symlink() => {
            bail!("{} is a link; refusing to write through it", path.display())
        }
        Ok(m) if !m.is_dir() => bail!("{} isn't a directory", path.display()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).with_context(|| format!("creating {}", path.display()))
        }
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Write the override `rel` under `crate_dir`. No link is followed anywhere on the way, and
/// the file is written to a temporary file next to it and renamed into place, so it's never
/// left half written. Returns where it went.
pub(super) fn write_override(crate_dir: &Path, rel: &str, bytes: &[u8]) -> Result<PathBuf> {
    safe_relative(rel)?;
    if let Some(parent) = crate_dir.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    plain_dir(crate_dir)?;
    let rel = Path::new(rel);
    let mut dir = crate_dir.to_path_buf();
    for c in rel.parent().into_iter().flat_map(Path::components) {
        let Component::Normal(name) = c else {
            bail!("unsafe override path `{}`", rel.display());
        };
        dir.push(name);
        plain_dir(&dir)?;
    }
    let name = rel
        .file_name()
        .context("override path without a file name")?;
    let dest = dir.join(name);
    match dest.symlink_metadata() {
        Ok(m) if m.file_type().is_symlink() => {
            bail!("{} is a link; refusing to write through it", dest.display())
        }
        Ok(m) if !m.is_file() => bail!("{} isn't a plain file", dest.display()),
        _ => {}
    }
    let mut tmp = tempfile::NamedTempFile::new_in(&dir)
        .with_context(|| format!("creating a temporary file in {}", dir.display()))?;
    tmp.write_all(bytes)?;
    tmp.persist(&dest)
        .with_context(|| format!("writing {}", dest.display()))?;
    Ok(dest)
}

#[cfg(test)]
#[path = "text_test.rs"]
mod tests;
