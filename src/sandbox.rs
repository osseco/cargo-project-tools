//! Runs a tool's build in an OS sandbox: no network, and writes only where the build puts
//! its output. `sandbox-exec` on macOS, `bwrap` on Linux, none elsewhere. Everything stays
//! readable: cargo and rustc need the toolchain and `CARGO_HOME`.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};

use crate::config::SandboxMode;
use crate::output::{self, Record};
use crate::process;

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Where the sandboxed build may write.
pub struct Writable {
    pub dirs: Vec<PathBuf>,
    /// Single files, created empty when missing.
    pub files: Vec<PathBuf>,
}

/// A working sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    SandboxExec,
    Bwrap(PathBuf),
}

impl Kind {
    /// As records name it.
    pub fn name(&self) -> &'static str {
        match self {
            Kind::SandboxExec => "sandbox-exec",
            Kind::Bwrap(_) => "bwrap",
        }
    }
}

/// The sandbox mode for one tool, and where it was set, for records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub mode: SandboxMode,
    /// `--sandbox` (or `--no-sandbox`) rather than `project-tools.toml`.
    pub from_cli: bool,
    /// The line in `project-tools.toml` of the setting that applied; none for the default.
    pub line: Option<usize>,
}

/// The sandbox to build `tool` in under `choice`: none when the mode is off, or when it's
/// auto and this machine has no working sandbox (with a note and a `sandbox-missing`
/// record). Fails under `require` without one. The probe runs once per run.
pub fn choose(tool: &str, choice: &Choice) -> Result<Option<Kind>> {
    if choice.mode == SandboxMode::Off {
        output::note(format!("note: not sandboxing {tool} (sandbox is off)"));
        return Ok(None);
    }
    decide(tool, choice, probe())
}

/// The sandbox this machine has, or why there's none; checked once per run.
pub fn probe() -> &'static std::result::Result<Kind, String> {
    static PROBE: OnceLock<std::result::Result<Kind, String>> = OnceLock::new();
    PROBE.get_or_init(available)
}

/// `choose` with the probe's result passed in.
fn decide(
    tool: &str,
    choice: &Choice,
    probe: &std::result::Result<Kind, String>,
) -> Result<Option<Kind>> {
    let why = match (probe, choice.mode) {
        (_, SandboxMode::Off) => return Ok(None),
        (Ok(kind), _) => return Ok(Some(kind.clone())),
        (Err(why), _) => why,
    };
    if choice.mode == SandboxMode::Require {
        let set = if choice.from_cli {
            "--sandbox require"
        } else {
            "sandbox = \"require\""
        };
        bail!("{tool}: a build sandbox is required ({set}), but {why}");
    }
    let message =
        format!("no build sandbox ({why}); build scripts run with network and write access");
    output::note(format!("note: {tool}: {message}"));
    output::emit(Record::SandboxMissing {
        tool: tool.to_string(),
        file: crate::project::CONFIG_FILE.to_string(),
        line: choice.line.filter(|_| !choice.from_cli),
        message,
    });
    Ok(None)
}

/// `cmd` to run in the sandbox `kind`, with `tmp` as its temp directory.
pub fn wrap(cmd: Command, kind: &Kind, writable: &Writable, tmp: &Path) -> Result<Command> {
    let writable = resolve(writable, tmp)?;
    let tmp = tmp.canonicalize()?;
    let mut out = match kind {
        Kind::SandboxExec => {
            let mut c = Command::new(SANDBOX_EXEC);
            c.arg("-p").arg(seatbelt_profile(&writable));
            c
        }
        Kind::Bwrap(bwrap) => {
            let mut c = Command::new(bwrap);
            c.args(bwrap_args(&writable)).arg("--");
            c
        }
    };
    out.arg(cmd.get_program()).args(cmd.get_args());
    for (key, value) in cmd.get_envs() {
        match value {
            Some(v) => out.env(key, v),
            None => out.env_remove(key),
        };
    }
    if let Some(dir) = cmd.get_current_dir() {
        out.current_dir(dir);
    }
    out.env("TMPDIR", tmp);
    Ok(out)
}

/// The writable places with links resolved, as the sandboxes compare real paths; the
/// directories and files are created first.
fn resolve(writable: &Writable, tmp: &Path) -> Result<Writable> {
    let mut dirs = Vec::new();
    for d in writable.dirs.iter().map(PathBuf::as_path).chain([tmp]) {
        fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
        dirs.push(d.canonicalize()?);
    }
    let mut files = Vec::new();
    for f in &writable.files {
        if !f.exists() {
            fs::write(f, "").with_context(|| format!("creating {}", f.display()))?;
        }
        files.push(f.canonicalize()?);
    }
    Ok(Writable { dirs, files })
}

/// The sandbox this machine has, after checking that it runs.
fn available() -> std::result::Result<Kind, String> {
    match std::env::consts::OS {
        "macos" => {
            if !Path::new(SANDBOX_EXEC).is_file() {
                return Err(format!("{SANDBOX_EXEC} isn't there"));
            }
            let mut probe = Command::new(SANDBOX_EXEC);
            probe.args(["-p", "(version 1)(allow default)", "/usr/bin/true"]);
            works(probe, "sandbox-exec").map(|()| Kind::SandboxExec)
        }
        "linux" => {
            let bwrap = process::which("bwrap").ok_or("bwrap isn't on PATH")?;
            let mut probe = Command::new(&bwrap);
            probe.args([
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--unshare-net",
                "--",
                "true",
            ]);
            works(probe, "bwrap").map(|()| Kind::Bwrap(bwrap))
        }
        os => Err(format!("there's none for {os}")),
    }
}

fn works(mut probe: Command, name: &str) -> std::result::Result<(), String> {
    match probe.output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "{name} doesn't work here: {}",
            process::error_line(&String::from_utf8_lossy(&out.stderr))
        )),
        Err(e) => Err(format!("{name} doesn't start: {e}")),
    }
}

/// A Seatbelt profile: everything allowed but the network and writes outside `writable`
/// (and `/dev`, for `/dev/null` and the terminal).
fn seatbelt_profile(writable: &Writable) -> String {
    let mut allow = vec![format!("(subpath {})", sbpl_string(Path::new("/dev")))];
    allow.extend(
        writable
            .dirs
            .iter()
            .map(|d| format!("(subpath {})", sbpl_string(d))),
    );
    allow.extend(
        writable
            .files
            .iter()
            .map(|f| format!("(literal {})", sbpl_string(f))),
    );
    format!(
        "(version 1)\n(allow default)\n(deny network*)\n(deny file-write*)\n(allow file-write*\n  {})\n",
        allow.join("\n  ")
    )
}

/// `path` as a Seatbelt string literal.
fn sbpl_string(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// bwrap's arguments: the whole system read-only, a fresh `/dev`, no network, and
/// `writable` bound writable in place.
fn bwrap_args(writable: &Writable) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--unshare-net",
        "--die-with-parent",
    ]
    .map(OsString::from)
    .to_vec();
    for p in writable.dirs.iter().chain(&writable.files) {
        args.extend([OsString::from("--bind"), p.into(), p.into()]);
    }
    args
}

#[cfg(test)]
#[path = "sandbox_test.rs"]
mod tests;
