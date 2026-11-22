//! `--message-format`: human text, or JSON records on stdout with the human text moved to
//! stderr. One record per line, each with `"version"`, so consumers can pin the schema.

use std::cell::{Cell, RefCell};
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

/// `println!` for human text: on stderr in json mode, so stdout holds only records. Kept
/// for later while a full-screen view is up.
macro_rules! say {
    ($($arg:tt)*) => {{
        let json = $crate::output::json();
        if !$crate::output::keep(json, format!($($arg)*)) {
            if json {
                eprintln!($($arg)*)
            } else {
                println!($($arg)*)
            }
        }
    }};
}
pub(crate) use say;

thread_local! {
    /// On the thread that runs a full-screen view, which also runs what it starts.
    static HOLDING: Cell<bool> = const { Cell::new(false) };
    /// Lines held meanwhile, and whether each goes to stderr.
    static HELD: RefCell<Vec<(bool, String)>> = const { RefCell::new(Vec::new()) };
}

/// While the returned guard lives, human text is kept instead of printed over the screen,
/// and commands run through `process` have their output captured. Dropping it prints what
/// was kept.
pub fn hold() -> Hold {
    HOLDING.set(true);
    Hold
}

pub struct Hold;

impl Drop for Hold {
    fn drop(&mut self) {
        HOLDING.set(false);
        let lines = HELD.take();
        for (stderr, line) in lines {
            if stderr {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
        }
    }
}

pub fn holding() -> bool {
    HOLDING.get()
}

/// Keep `line` for later when holding; false when it should be printed now.
pub fn keep(stderr: bool, line: String) -> bool {
    if !holding() {
        return false;
    }
    HELD.with_borrow_mut(|held| held.push((stderr, line)));
    true
}

/// A note on stderr, kept for later while a full-screen view is up.
pub fn note(line: String) {
    if !keep(true, line.clone()) {
        eprintln!("{line}");
    }
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Record {
    /// Build-time code that isn't recorded in `file`.
    ReviewPending {
        tool: String,
        file: String,
        /// 1-based, where in `file` it is.
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        item: Box<ReviewItem>,
        /// What `review --approve`, `--strip` and `--override` take for it:
        /// `package@version=hash`, or `path=hash` for cargo config.
        spec: String,
        /// The entry to put in `file`, with the action it would get.
        entry: String,
    },
    /// An entry in `file` that matches nothing the tool builds.
    ReviewUnused {
        tool: String,
        file: String,
        /// 1-based, where in `file` it is.
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        #[serde(rename = "type")]
        kind: String,
        package: String,
        path: String,
    },
    TargetMissing {
        tool: String,
        file: String,
        /// 1-based, where in `file` it is.
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        targets: Vec<String>,
    },
    /// `[build].sandbox = "auto"` found no sandbox, so the build runs without one.
    SandboxMissing {
        tool: String,
        file: String,
        /// 1-based: the `sandbox` or `sandbox-os` setting that applied, when one did.
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        message: String,
    },
    /// A tool `install` built, and the sandbox it built in: `sandbox-exec`, `bwrap` or
    /// `none`.
    Built {
        tool: String,
        tool_version: String,
        sandbox: String,
        bins: Vec<String>,
    },
    Advisory {
        tool: String,
        file: String,
        /// 1-based, where in `file` it is.
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
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
        /// 1-based, where in `file` it is.
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        id: String,
    },
    IgnoreExpired {
        tool: String,
        file: String,
        /// 1-based, where in `file` it is.
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<usize>,
        id: String,
        expires: String,
    },
    /// What `inspect` found.
    Inspect(crate::inspect::Inspection),
    /// A dependency named without a version that matches several locked versions.
    AmbiguousDependency {
        tool: String,
        package: String,
        /// The `name@version` specs to choose from.
        candidates: Vec<String>,
    },
    /// `settings.sfw = "auto"` found no sfw, so downloads aren't filtered.
    SfwMissing {
        message: String,
    },
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The keys whose values changed: `sha256`, `checksum` or `commit`.
    pub changed: Vec<&'static str>,
    /// The version the entry was recorded for, when its copy was fetched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// That copy's build script or crate, repo-relative.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
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

/// Compact JSON that also writes as `\uXXXX` what serde_json leaves as is but a terminal
/// could act on or hide: DEL, C1 controls (U+009B starts an escape sequence), and the
/// characters `sanitize::hidden` flags (bidi controls, zero-width characters, line and
/// paragraph separators, …). The values stay the same.
struct Escaping;

impl serde_json::ser::Formatter for Escaping {
    fn write_string_fragment<W>(&mut self, writer: &mut W, fragment: &str) -> std::io::Result<()>
    where
        W: ?Sized + Write,
    {
        let mut start = 0;
        for (i, c) in fragment.char_indices() {
            let code = c as u32;
            if code == 0x7f || (0x80..0xa0).contains(&code) || crate::sanitize::hidden(c) {
                writer.write_all(&fragment.as_bytes()[start..i])?;
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    write!(writer, "\\u{u:04x}")?;
                }
                start = i + c.len_utf8();
            }
        }
        writer.write_all(&fragment.as_bytes()[start..])
    }
}

/// The JSON line for a record.
pub fn line(record: &Record) -> String {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, Escaping);
    Line {
        version: VERSION,
        record,
    }
    .serialize(&mut ser)
    .expect("records serialize");
    String::from_utf8(out).expect("JSON is UTF-8")
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

/// Print `error: …` on stderr, on one line and safe to show: errors quote paths and names
/// from sources.
pub fn print_error(e: &anyhow::Error) {
    eprintln!("error: {}", crate::sanitize::shown_line(&format!("{e:#}")));
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
