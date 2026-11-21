//! Reading files for the review screen, making their text safe to draw, and writing
//! overrides.

use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::UNREVIEWED;
use crate::review::{Item, Kind, safe_relative};

/// Whether `c` is drawn as an escape instead of itself. Code on the review screen is there
/// to be read, so anything that can hide, reorder or disguise text is shown by its code
/// point: format characters (bidi controls, zero-width characters, tag characters),
/// variation selectors and other characters that draw nothing, spaces other than U+0020,
/// line and paragraph separators, private-use characters and noncharacters. Control
/// characters are handled before this.
fn hidden(c: char) -> bool {
    let code = c as u32;
    matches!(code,
        // Format characters (general category Cf).
        0x00ad
        | 0x0600..=0x0605
        | 0x061c
        | 0x06dd
        | 0x070f
        | 0x0890..=0x0891
        | 0x08e2
        | 0x180e
        | 0x200b..=0x200f
        | 0x202a..=0x202e
        | 0x2060..=0x2064
        | 0x2066..=0x206f
        | 0xfeff
        | 0xfff9..=0xfffb
        | 0x110bd
        | 0x110cd
        | 0x13430..=0x1343f
        | 0x1bca0..=0x1bca3
        | 0x1d173..=0x1d17a
        | 0xe0001
        | 0xe0020..=0xe007f
        // Line and paragraph separators.
        | 0x2028..=0x2029
        // Spaces that look like U+0020.
        | 0x00a0
        | 0x1680
        | 0x2000..=0x200a
        | 0x202f
        | 0x205f
        | 0x3000
        // Characters that draw nothing or next to nothing.
        | 0x034f
        | 0x115f..=0x1160
        | 0x17b4..=0x17b5
        | 0x180b..=0x180d
        | 0x180f
        | 0x2800
        | 0x3164
        | 0xffa0
        // Variation selectors.
        | 0xfe00..=0xfe0f
        | 0xe0100..=0xe01ef
        // Private use.
        | 0xe000..=0xf8ff
        | 0xf0000..=0x10ffff
        // Noncharacters (those in planes 15 and 16 are private use above).
        | 0xfdd0..=0xfdef
    ) || code & 0xfffe == 0xfffe
}

/// Make untrusted text safe to draw: control characters become `\xNN`, and characters
/// that could hide or disguise text (see `hidden`) become `<U+XXXX>`. Returns the text and
/// how many characters were replaced.
pub fn sanitize(text: &str) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut replaced = 0;
    for c in text.chars() {
        let code = c as u32;
        if c == '\n' || c == '\t' {
            out.push(c);
        } else if code < 0x20 || (0x7f..0xa0).contains(&code) {
            out.push_str(&format!("\\x{code:02x}"));
            replaced += 1;
        } else if hidden(c) {
            out.push_str(&format!("<U+{code:04X}>"));
            replaced += 1;
        } else {
            out.push(c);
        }
    }
    (out, replaced)
}

/// `text` made safe to draw.
pub fn shown(text: &str) -> String {
    sanitize(text).0
}

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
pub(super) fn override_rel(item: &Item) -> Result<String> {
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
