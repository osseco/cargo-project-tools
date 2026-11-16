//! `bin/<tool>.sha256`: the sha256 of every binary `install` built for a tool, in
//! `sha256sum` format so `sha256sum -c` can check it too.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::project::Project;

#[derive(Debug, PartialEq)]
pub enum Mismatch {
    Missing(String),
    Differs(String),
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mismatch::Missing(bin) => write!(f, "{bin} is missing"),
            Mismatch::Differs(bin) => write!(f, "{bin} does not match its checksum"),
        }
    }
}

pub fn path(project: &Project, tool: &str) -> PathBuf {
    project.bin_dir().join(format!("{tool}.sha256"))
}

pub fn file_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Hash `bins` (names in `bin/`) and record them for `tool`.
pub fn write(project: &Project, tool: &str, bins: &[String]) -> Result<()> {
    let mut lines = Vec::new();
    for bin in bins {
        check_name(bin)?;
        lines.push(format!(
            "{}  {bin}\n",
            file_sha256(&project.bin_dir().join(bin))?
        ));
    }
    lines.sort();
    let path = path(project, tool);
    fs::write(&path, lines.concat()).with_context(|| format!("writing {}", path.display()))
}

/// The recorded `(bin, sha256)` pairs for `tool`.
pub fn read(project: &Project, tool: &str) -> Result<Vec<(String, String)>> {
    let path = path(project, tool);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("no checksum for {tool}; run `cargo project-tools install {tool}`")
        }
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let (hash, bin) = line
            .split_once("  ")
            .with_context(|| format!("malformed line in {}: {line}", path.display()))?;
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("malformed checksum in {}: {line}", path.display());
        }
        check_name(bin)?;
        out.push((bin.to_string(), hash.to_ascii_lowercase()));
    }
    if out.is_empty() {
        bail!("{} lists no binaries", path.display());
    }
    Ok(out)
}

/// Binaries of `tool` that are missing or differ from the recorded checksum.
pub fn check(project: &Project, tool: &str) -> Result<Vec<Mismatch>> {
    let mut out = Vec::new();
    for (bin, expected) in read(project, tool)? {
        let file = project.bin_dir().join(&bin);
        if !file.is_file() {
            out.push(Mismatch::Missing(bin));
        } else if file_sha256(&file)? != expected {
            out.push(Mismatch::Differs(bin));
        }
    }
    Ok(out)
}

/// The checksum file may come from a cache, so its names must stay inside `bin/`.
fn check_name(bin: &str) -> Result<()> {
    if bin.is_empty() || bin.contains(['/', '\\']) || bin == "." || bin == ".." {
        bail!("invalid binary name `{bin}`");
    }
    Ok(())
}

#[cfg(test)]
#[path = "checksum_test.rs"]
mod tests;
