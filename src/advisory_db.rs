use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::net::{self, Fetched};
use crate::output::say;
use crate::source::registry;

/// A `git archive` of the default branch. codeload answers `If-None-Match` with 304.
const URL: &str = "https://codeload.github.com/rustsec/advisory-db/tar.gz/refs/heads/main";
const PREFIX: &str = "advisory-db-main";
const STATE: &str = ".fetch.json";

/// What we know about the copy on disk, kept next to it.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub etag: Option<String>,
    /// When we last checked with the server (unix seconds).
    pub fetched_at: u64,
    /// Commit of the archive, from its pax global header.
    pub commit: Option<String>,
    /// Commit time of the archive (unix seconds).
    pub updated_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Fetch when missing, revalidate when older than the max age.
    Auto,
    /// Revalidate now.
    Force,
    /// Never touch the network.
    Offline,
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Use {
        stale: bool,
    },
    /// `required`: a failure is an error rather than a warning.
    Fetch {
        required: bool,
    },
}

fn decide(state: Option<&State>, policy: Policy, max_age: Duration, now: u64) -> Result<Action> {
    let Some(state) = state else {
        if policy == Policy::Offline {
            bail!(
                "no advisory database yet, and it can't be fetched offline; run `cargo {} audit-db update` first, or pass --no-audit",
                crate::cli::SUBCOMMAND
            );
        }
        return Ok(Action::Fetch { required: true });
    };
    let stale = now.saturating_sub(state.fetched_at) > max_age.as_secs();
    Ok(match policy {
        Policy::Force => Action::Fetch { required: true },
        Policy::Auto if stale => Action::Fetch { required: false },
        _ => Action::Use { stale },
    })
}

/// Open the database at `path`, fetching or revalidating it first as `policy` and
/// `max_age` say.
pub fn ensure(path: &Path, policy: Policy, max_age: Duration) -> Result<rustsec::Database> {
    ensure_with(path, policy, max_age, now_secs(), |etag| {
        net::get_conditional(URL, etag)
    })
}

/// `ensure` with the clock and the conditional GET passed in.
fn ensure_with(
    path: &Path,
    policy: Policy,
    max_age: Duration,
    now: u64,
    get: impl FnOnce(Option<&str>) -> Result<Fetched>,
) -> Result<rustsec::Database> {
    let state = read_state(path);
    let mut fetched = None;
    match decide(state.as_ref(), policy, max_age, now)? {
        Action::Use { stale } => {
            if stale {
                eprintln!("warning: advisory database is older than the configured max age");
            }
        }
        Action::Fetch { required } => match fetch(path, state.as_ref(), now, get) {
            Ok(db) => fetched = db,
            Err(e) if !required => {
                eprintln!("warning: could not refresh the advisory database: {e:#}")
            }
            Err(e) => return Err(e),
        },
    }
    if let Some(state) = read_state(path) {
        say!("advisory-db: {}", describe(&state, now));
    }
    match fetched {
        Some(db) => Ok(db),
        None => rustsec::Database::open(path)
            .with_context(|| format!("opening advisory database {}", path.display())),
    }
}

/// Conditional GET. Returns the new database when the server sent one.
fn fetch(
    path: &Path,
    state: Option<&State>,
    now: u64,
    get: impl FnOnce(Option<&str>) -> Result<Fetched>,
) -> Result<Option<rustsec::Database>> {
    match get(state.and_then(|s| s.etag.as_deref()))? {
        Fetched::NotModified => {
            say!("advisory-db: unchanged on the server");
            let mut state = state.cloned().unwrap_or_default();
            state.fetched_at = now;
            write_state(path, &state)?;
            Ok(None)
        }
        Fetched::Body { bytes, etag } => {
            say!("advisory-db: downloaded {} KiB", bytes.len() / 1024);
            let (commit, updated_at) = archive_info(&bytes)?;
            let parent = path
                .parent()
                .context("advisory database path has no parent")?;
            fs::create_dir_all(parent)?;
            let tmp = tempfile::Builder::new()
                .prefix(".advisory-db-")
                .tempdir_in(parent)?;
            let new = tmp.path().join("db");
            registry::extract_prefixed(tar_gz(&bytes), PREFIX, &new)?;
            // Check it loads before it replaces the copy we have.
            let db =
                rustsec::Database::open(&new).context("fetched advisory database is invalid")?;
            write_state(
                &new,
                &State {
                    etag,
                    fetched_at: now,
                    commit,
                    updated_at,
                },
            )?;
            if path.exists() {
                fs::rename(path, tmp.path().join("old"))?;
            }
            fs::rename(&new, path)
                .with_context(|| format!("moving advisory database to {}", path.display()))?;
            Ok(Some(db))
        }
    }
}

fn tar_gz(bytes: &[u8]) -> tar::Archive<flate2::read::GzDecoder<&[u8]>> {
    tar::Archive::new(flate2::read::GzDecoder::new(bytes))
}

/// Commit id from the pax global header that `git archive` writes, and the commit time from
/// the first real entry's mtime.
fn archive_info(bytes: &[u8]) -> Result<(Option<String>, Option<u64>)> {
    let mut commit = None;
    let mut ar = tar_gz(bytes);
    for item in ar.entries()? {
        let mut item = item?;
        if item.header().entry_type().is_pax_global_extensions() {
            if let Some(exts) = item.pax_extensions()? {
                for ext in exts {
                    let ext = ext?;
                    if ext.key() == Ok("comment") {
                        commit = ext.value().ok().map(str::to_string);
                    }
                }
            }
            continue;
        }
        return Ok((commit, item.header().mtime().ok()));
    }
    Ok((commit, None))
}

fn read_state(path: &Path) -> Option<State> {
    // A state file without the advisories means a half-written copy: start over.
    if !path.join("crates").is_dir() {
        return None;
    }
    serde_json::from_str(&fs::read_to_string(path.join(STATE)).ok()?).ok()
}

fn write_state(path: &Path, state: &State) -> Result<()> {
    fs::write(path.join(STATE), serde_json::to_string_pretty(state)?)
        .with_context(|| format!("writing {}", path.join(STATE).display()))
}

fn describe(state: &State, now: u64) -> String {
    let commit = state
        .commit
        .as_deref()
        .map_or("unknown commit", |c| &c[..c.len().min(12)]);
    let mut s = commit.to_string();
    if let Some(updated) = state.updated_at {
        s.push_str(&format!(
            ", updated {} ago",
            age(now.saturating_sub(updated))
        ));
    }
    s.push_str(&format!(
        ", checked {} ago",
        age(now.saturating_sub(state.fetched_at))
    ));
    s
}

fn age(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "advisory_db_test.rs"]
mod tests;
