//! The version a changed piece of build-time code was recorded for: fetched next to the
//! tool's build, so the change can be read, and shown as a diff by `review --diff`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{Action, Entry, Item, Kind, Pin, git_commit, is_registry, load, pulled_by, slash};
use crate::build_files;
use crate::output::{self, say};
use crate::project::Project;
use crate::sanitize::shown_line;
use crate::source::{Checkout, git, registry};

/// A copy of the crate an entry was recorded for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OldCopy {
    pub version: String,
    /// Its package directory.
    pub base: PathBuf,
}

impl OldCopy {
    /// What to read in it: the build script, or the whole crate.
    pub fn location(&self, item: &Item) -> PathBuf {
        match item.kind {
            Kind::BuildScript => self.base.join(&item.path),
            _ => self.base.clone(),
        }
    }
}

/// Fetch the crate each changed piece's entry was recorded for into
/// `targets/<tool>/previous`, and set the piece's `old`. Best effort: what can't be fetched
/// gets a note, and nothing is fetched offline.
pub fn fetch(
    project: &Project,
    name: &str,
    checkout: &Checkout,
    items: &mut [Item],
    offline: bool,
) -> Result<()> {
    if offline {
        return Ok(());
    }
    let entries = load(&project.review_path(name))?;
    let dir = project.target_dir(name).join("previous");
    for item in items.iter_mut() {
        let Some(e) = entries
            .iter()
            .find(|e| item.same_place(e) && !item.matches(e) && e.action != Action::Pending)
        else {
            continue;
        };
        let Some(old) = old_crate(item, e) else {
            continue;
        };
        match fetch_one(project, checkout, item, &old, &dir) {
            Ok(copy) => item.old = Some(copy),
            Err(err) => output::note(format!(
                "note: no previous copy of {}: {}",
                shown_line(&item.describe()),
                shown_line(&format!("{err:#}"))
            )),
        }
    }
    Ok(())
}

/// The crate `e` was recorded for, when it's another one than the piece's: a build script's
/// pin, or a whole crate's checksum or commit. A package of the tool's own workspace is
/// pinned by its files, which name no crate to fetch.
fn old_crate(item: &Item, e: &Entry) -> Option<Pin> {
    let (old, now) = match item.kind {
        Kind::BuildScript => (e.pin.clone(), item.pin.clone()),
        Kind::ProcMacro | Kind::BuildDep => {
            let source = item.source.as_deref()?;
            if is_registry(source) {
                (
                    Pin::Checksum(e.hash.clone()),
                    Pin::Checksum(item.hash.clone()),
                )
            } else if git_commit(source).is_some() {
                (Pin::Commit(e.hash.clone()), Pin::Commit(item.hash.clone()))
            } else {
                return None;
            }
        }
        Kind::CargoConfig => return None,
    };
    (old != Pin::None && old != now).then_some(old)
}

fn fetch_one(
    project: &Project,
    checkout: &Checkout,
    item: &Item,
    old: &Pin,
    dir: &Path,
) -> Result<OldCopy> {
    match old {
        Pin::Checksum(sum) => {
            let entries = registry::fetch_index(&item.package)?;
            let entry = entries
                .iter()
                .find(|e| e.cksum == *sum)
                .context("no published version has the recorded checksum")?;
            let file = registry::download(entry, &project.cache_dir())?;
            let dest = dir.join(format!("{}-{}", entry.name, entry.vers));
            registry::extract(&file, entry, &dest)?;
            Ok(OldCopy {
                version: entry.vers.to_string(),
                base: dest,
            })
        }
        Pin::Commit(commit) => {
            if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("recorded commit `{}` isn't a full hash", shown_line(commit));
            }
            let url = match item.source.as_deref() {
                Some(source) => git_url(source).context("not a git source")?,
                // A package of the tool's own clone.
                None => git::origin_url(&checkout.root)?,
            };
            let dest = dir.join(format!("{}-{}", item.package, &commit[..12]));
            let reference = git::GitRef {
                rev: Some(commit.clone()),
                ..Default::default()
            };
            let clone = git::clone(&url, &reference, &item.package, &dest)?;
            Ok(OldCopy {
                version: clone.version,
                base: clone.package_dir,
            })
        }
        Pin::None => bail!("nothing recorded to fetch"),
    }
}

/// The repository of a git source in cargo's notation: `git+<url>?<ref>#<commit>`.
fn git_url(source: &str) -> Option<String> {
    let url = source.strip_prefix("git+")?;
    let end = url.find(['?', '#']).unwrap_or(url.len());
    Some(url[..end].to_string())
}

/// Print a unified diff of what each changed piece's hash covers against its previous copy.
pub fn print_diffs(name: &str, items: &[Item]) -> Result<()> {
    let mut any = false;
    for item in items {
        let Some(old) = &item.old else { continue };
        any = true;
        say!(
            "{} (was {})",
            shown_line(&item.describe()),
            shown_line(&old.version)
        );
        for line in diff(item, old)?.lines() {
            say!("{}", shown_line(line));
        }
    }
    if !any {
        say!("{name}: no changed build-time code with a previous version to compare");
    }
    Ok(())
}

/// The unified diff, file by file, of what the piece's hash covers in `old` and now.
fn diff(item: &Item, old: &OldCopy) -> Result<String> {
    let mut paths = covered(item.kind, &item.base, &item.location, &item.pulled)?;
    let old_location = old.location(item);
    let old_pulled = match item.kind {
        Kind::BuildScript if old_location.is_file() => pulled_by(&old.base, &old_location)?,
        _ => Default::default(),
    };
    paths.extend(covered(item.kind, &old.base, &old_location, &old_pulled)?);
    let mut out = String::new();
    for rel in paths {
        let before = read(&old.base.join(&rel))?;
        let after = read(&item.base.join(&rel))?;
        if before == after {
            continue;
        }
        let (Ok(a), Ok(b)) = (
            String::from_utf8(before.clone().unwrap_or_default()),
            String::from_utf8(after.clone().unwrap_or_default()),
        ) else {
            out.push_str(&format!("Binary files a/{rel} and b/{rel} differ\n"));
            continue;
        };
        let a_name = before.map_or("/dev/null".to_string(), |_| format!("a/{rel}"));
        let b_name = after.map_or("/dev/null".to_string(), |_| format!("b/{rel}"));
        out.push_str(
            &similar::TextDiff::from_lines(&a, &b)
                .unified_diff()
                .header(&a_name, &b_name)
                .to_string(),
        );
    }
    Ok(out)
}

fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// The files, relative to `base`, that a piece's hash covers, as `script_hash` and
/// `dir_hash` pick them.
fn covered(
    kind: Kind,
    base: &Path,
    location: &Path,
    pulled: &build_files::Pulled,
) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    match kind {
        Kind::BuildScript if pulled.whole_package.is_none() => {
            match location.parent() {
                Some(dir) if dir != base => files(base, dir, &mut out)?,
                _ if location.is_file() => {
                    out.insert(slash(location.strip_prefix(base)?));
                }
                _ => {}
            }
            for f in &pulled.files {
                out.insert(slash(f.strip_prefix(base)?));
            }
        }
        Kind::CargoConfig => {
            if location.is_file() {
                out.insert(slash(location.strip_prefix(base)?));
            }
        }
        _ => files(base, base, &mut out)?,
    }
    Ok(out)
}

/// Every file under `dir`, relative to `base`. Links, `.git` and cargo's vendoring checksums
/// are left out: they aren't code to read.
fn files(base: &Path, dir: &Path, out: &mut BTreeSet<String>) -> Result<()> {
    let Ok(read) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in read {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        let name = entry.file_name();
        if kind.is_dir() && name != ".git" {
            files(base, &path, out)?;
        } else if kind.is_file() && name != ".cargo-checksum.json" {
            out.insert(slash(path.strip_prefix(base)?));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "review_previous_test.rs"]
mod tests;
