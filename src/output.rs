//! `--message-format`: human text, or JSON records on stdout with the human text moved to
//! stderr. One record per line, each with `"version"`, so consumers can pin the schema.

use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde::Serialize;

/// Bumped when a record changes incompatibly.
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Format {
    #[default]
    Human,
    Json,
}

static FORMAT: OnceLock<Format> = OnceLock::new();
static SUMMARY: AtomicBool = AtomicBool::new(false);
static VULNERABILITIES: AtomicUsize = AtomicUsize::new(0);
static WARNINGS: AtomicUsize = AtomicUsize::new(0);
static PENDING_REVIEWS: AtomicUsize = AtomicUsize::new(0);
static MISSING_TARGETS: AtomicUsize = AtomicUsize::new(0);

/// Set once, before anything is printed. `summary`: end with a `summary` record.
pub fn init(format: Format, summary: bool) {
    let _ = FORMAT.set(format);
    SUMMARY.store(summary, Ordering::Relaxed);
}

/// This process wrote nothing itself (it re-ran under sfw): no summary at exit.
pub fn skip_summary() {
    SUMMARY.store(false, Ordering::Relaxed);
}

pub fn json() -> bool {
    FORMAT.get() == Some(&Format::Json)
}

/// `println!` for human text: on stderr in json mode, so stdout holds only records.
macro_rules! say {
    ($($arg:tt)*) => {
        if $crate::output::json() {
            eprintln!($($arg)*)
        } else {
            println!($($arg)*)
        }
    };
}
pub(crate) use say;

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Record {
    /// Build-time code that isn't recorded in `file`.
    ReviewPending {
        tool: String,
        file: String,
        item: ReviewItem,
        /// The entry to put in `file`, with the action it would get.
        entry: String,
    },
    /// An entry in `file` that matches nothing the tool builds.
    ReviewUnused {
        tool: String,
        file: String,
        #[serde(rename = "type")]
        kind: String,
        package: String,
        path: String,
    },
    TargetMissing {
        tool: String,
        file: String,
        targets: Vec<String>,
    },
    Advisory {
        tool: String,
        file: String,
        id: String,
        /// `vulnerability`, or the warning: `unmaintained`, `unsound`, `notice`, `yanked`,
        /// `git-source`.
        #[serde(rename = "type")]
        kind: String,
        package: String,
        /// Not `version`, which is the schema's.
        package_version: String,
        title: String,
        patched: Vec<String>,
        severity: Severity,
        #[serde(skip_serializing_if = "Option::is_none")]
        fix: Option<String>,
    },
    IgnoreUnused {
        tool: String,
        file: String,
        id: String,
    },
    IgnoreExpired {
        tool: String,
        file: String,
        id: String,
        expires: String,
    },
    /// What `inspect` found.
    Inspect(crate::inspect::Inspection),
    Error {
        message: String,
    },
    Summary {
        ok: bool,
        vulnerabilities: usize,
        warnings: usize,
        pending_reviews: usize,
        missing_targets: usize,
    },
}

#[derive(Debug, Serialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ReviewItem {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub package: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// A proc macro's crate checksum, or the crate a build script comes with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    /// The git commit a build script comes with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The file or directory to read, repo-relative.
    pub location: String,
    /// The recorded entry this one replaces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<Previous>,
    /// The decision another tool's review file records for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub known: Option<Known>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Previous {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Known {
    pub tool: String,
    pub action: String,
}

#[derive(Serialize)]
struct Line<'a> {
    version: u32,
    #[serde(flatten)]
    record: &'a Record,
}

/// The JSON line for a record.
pub fn line(record: &Record) -> String {
    serde_json::to_string(&Line {
        version: VERSION,
        record,
    })
    .expect("records serialize")
}

/// Count the record for the summary, and in json mode write it to stdout.
pub fn emit(record: Record) {
    let counter = match &record {
        Record::ReviewPending { .. } => Some(&PENDING_REVIEWS),
        Record::TargetMissing { targets, .. } => {
            MISSING_TARGETS.fetch_add(targets.len(), Ordering::Relaxed);
            None
        }
        Record::Advisory {
            severity: Severity::Error,
            ..
        } => Some(&VULNERABILITIES),
        Record::Advisory { .. } => Some(&WARNINGS),
        _ => None,
    };
    if let Some(c) = counter {
        c.fetch_add(1, Ordering::Relaxed);
    }
    if json() {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{}", line(&record));
        let _ = out.flush();
    }
}

/// At exit: an `error` record for a failure, then the `summary` when the command has one.
pub fn finish(error: Option<&anyhow::Error>, ok: bool) {
    if let Some(e) = error {
        emit(Record::Error {
            message: format!("{e:#}"),
        });
    }
    if SUMMARY.load(Ordering::Relaxed) {
        emit(Record::Summary {
            ok,
            vulnerabilities: VULNERABILITIES.load(Ordering::Relaxed),
            warnings: WARNINGS.load(Ordering::Relaxed),
            pending_reviews: PENDING_REVIEWS.load(Ordering::Relaxed),
            missing_targets: MISSING_TARGETS.load(Ordering::Relaxed),
        });
    }
}

#[cfg(test)]
#[path = "output_test.rs"]
mod tests;
