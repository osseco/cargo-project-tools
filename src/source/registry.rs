use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use semver::{Version, VersionReq};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::net;

const INDEX: &str = "https://index.crates.io";
const DOWNLOAD: &str = "https://static.crates.io/crates";

#[derive(Debug, Clone, Deserialize)]
pub struct IndexEntry {
    pub name: String,
    pub vers: Version,
    pub cksum: String,
    #[serde(default)]
    pub yanked: bool,
}

/// Path of a crate's file in the sparse index.
pub fn index_path(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    match n.len() {
        1 => format!("1/{n}"),
        2 => format!("2/{n}"),
        3 => format!("3/{}/{n}", &n[..1]),
        _ => format!("{}/{}/{n}", &n[..2], &n[2..4]),
    }
}

fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        bail!("invalid crate name `{name}`");
    }
    Ok(())
}

pub fn fetch_index(name: &str) -> Result<Vec<IndexEntry>> {
    validate_name(name)?;
    let body = net::get_bytes(&format!("{INDEX}/{}", index_path(name)))
        .with_context(|| format!("looking up `{name}` on crates.io"))?;
    let text = String::from_utf8(body)?;
    let mut entries = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        // Skip entries this parser doesn't understand rather than failing the lookup.
        if let Ok(entry) = serde_json::from_str::<IndexEntry>(line) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// Parse `1.2.3` as an exact requirement and anything else as a semver requirement.
pub fn parse_req(s: &str) -> Result<VersionReq> {
    if let Ok(v) = Version::parse(s) {
        return Ok(VersionReq::parse(&format!("={v}"))?);
    }
    VersionReq::parse(s).with_context(|| format!("invalid version requirement `{s}`"))
}

/// Highest non-yanked version matching `req`; without a requirement, the highest stable one.
pub fn select<'a>(entries: &'a [IndexEntry], req: Option<&VersionReq>) -> Option<&'a IndexEntry> {
    entries
        .iter()
        .filter(|e| !e.yanked)
        .filter(|e| match req {
            Some(r) => r.matches(&e.vers),
            None => e.vers.pre.is_empty(),
        })
        .max_by(|a, b| a.vers.cmp(&b.vers))
}

/// Lowest non-yanked version above `current` that, for every advisory, matches one of its
/// patched requirements.
pub fn lowest_patched<'a>(
    entries: &'a [IndexEntry],
    current: &Version,
    advisories: &[Vec<VersionReq>],
) -> Option<&'a IndexEntry> {
    entries
        .iter()
        .filter(|e| !e.yanked && e.vers > *current)
        .filter(|e| {
            advisories
                .iter()
                .all(|patched| patched.iter().any(|r| r.matches(&e.vers)))
        })
        .min_by(|a, b| a.vers.cmp(&b.vers))
}

/// Index lookups shared across a run, fetched a few at a time in parallel.
#[derive(Default)]
pub struct IndexCache {
    entries: BTreeMap<String, Vec<IndexEntry>>,
}

impl IndexCache {
    /// Fetch every name not yet cached. Lookups that fail are cached as empty.
    pub fn prefetch<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        let missing: Vec<&str> = names
            .into_iter()
            .filter(|n| !self.entries.contains_key(*n))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if missing.is_empty() {
            return;
        }
        let chunk = missing.len().div_ceil(8);
        let fetched: Vec<(String, Vec<IndexEntry>)> = std::thread::scope(|s| {
            let handles: Vec<_> = missing
                .chunks(chunk)
                .map(|names| {
                    s.spawn(move || {
                        names
                            .iter()
                            .map(|n| (n.to_string(), fetch_index(n).unwrap_or_default()))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });
        self.entries.extend(fetched);
    }

    /// Seed a lookup, so tests never reach the index.
    #[cfg(test)]
    pub fn insert(&mut self, name: &str, entries: Vec<IndexEntry>) {
        self.entries.insert(name.to_string(), entries);
    }

    pub fn get(&mut self, name: &str) -> &[IndexEntry] {
        self.prefetch([name]);
        &self.entries[name]
    }

    /// Whether the index marks this exact version as yanked.
    pub fn is_yanked(&mut self, name: &str, version: &Version) -> bool {
        self.get(name)
            .iter()
            .any(|e| e.vers == *version && e.yanked)
    }
}

/// Download a `.crate` (or reuse the cached copy) and verify its sha256 against the index.
pub fn download(entry: &IndexEntry, cache_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(cache_dir)?;
    let file = cache_dir.join(format!("{}-{}.crate", entry.name, entry.vers));
    if let Ok(bytes) = fs::read(&file)
        && sha256_hex(&bytes) == entry.cksum
    {
        return Ok(file);
    }
    let url = format!(
        "{DOWNLOAD}/{n}/{n}-{v}.crate",
        n = entry.name,
        v = entry.vers
    );
    let bytes = net::get_bytes(&url)?;
    let actual = sha256_hex(&bytes);
    if actual != entry.cksum {
        bail!(
            "checksum mismatch for {}-{}: index says {}, download is {actual}",
            entry.name,
            entry.vers,
            entry.cksum
        );
    }
    let tmp = file.with_extension("crate.part");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &file)?;
    Ok(file)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn archive(crate_file: &Path) -> Result<tar::Archive<flate2::read::GzDecoder<fs::File>>> {
    let file =
        fs::File::open(crate_file).with_context(|| format!("opening {}", crate_file.display()))?;
    Ok(tar::Archive::new(flate2::read::GzDecoder::new(file)))
}

/// Relative path inside the `<prefix>/` directory, rejecting anything that could escape it.
fn inner_path(path: &Path, prefix: &str) -> Result<Option<PathBuf>> {
    let mut comps = path.components();
    match comps.next() {
        Some(Component::Normal(first)) if first == prefix => {}
        _ => bail!("unexpected path in archive: {}", path.display()),
    }
    let rest: PathBuf = comps.collect();
    if rest
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("unsafe path in archive: {}", path.display());
    }
    // A `.git` in the source can hold config that makes git run commands. `cargo package`
    // never ships one. Any case, for case-insensitive file systems.
    if rest
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
    {
        bail!("crate archive contains .git: {}", path.display());
    }
    Ok((!rest.as_os_str().is_empty()).then_some(rest))
}

/// Extract the whole crate into `dest` (which is replaced). Returns `dest`.
pub fn extract(crate_file: &Path, entry: &IndexEntry, dest: &Path) -> Result<PathBuf> {
    let prefix = format!("{}-{}", entry.name, entry.vers);
    extract_prefixed(archive(crate_file)?, &prefix, dest)?;
    Ok(dest.to_path_buf())
}

/// Extract everything under `<prefix>/` into `dest` (which is replaced). Only files and
/// directories are written; links and specials are skipped.
pub fn extract_prefixed<R: Read>(mut ar: tar::Archive<R>, prefix: &str, dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;
    for item in ar.entries()? {
        let mut item = item?;
        let kind = item.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        let Some(rel) = inner_path(&item.path()?, prefix)? else {
            continue;
        };
        let target = dest.join(&rel);
        if kind.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        item.unpack(&target)
            .with_context(|| format!("extracting {}", rel.display()))?;
    }
    Ok(())
}

/// Read a single top-level file (e.g. `Cargo.toml`) from a crate archive.
pub fn read_file(crate_file: &Path, entry: &IndexEntry, file: &str) -> Result<Option<String>> {
    let prefix = format!("{}-{}", entry.name, entry.vers);
    let mut ar = archive(crate_file)?;
    for item in ar.entries()? {
        let mut item = item?;
        if !item.header().entry_type().is_file() {
            continue;
        }
        if inner_path(&item.path()?, &prefix)?.as_deref() == Some(Path::new(file)) {
            let mut s = String::new();
            item.read_to_string(&mut s)?;
            return Ok(Some(s));
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "registry_test.rs"]
mod tests;
