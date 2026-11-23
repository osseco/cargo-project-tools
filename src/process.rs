use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// The cargo binary: `$CARGO` when run as a cargo subcommand, otherwise `cargo` on PATH.
/// Cargo fetches git dependencies with the git command, limited to https like our own
/// clones; its built-in git would take any transport a manifest names.
pub fn cargo() -> Command {
    let mut cmd =
        Command::new(std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")));
    cmd.env("CARGO_NET_GIT_FETCH_WITH_CLI", "true")
        .env("GIT_ALLOW_PROTOCOL", "https")
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// Keeps git, as started by a build script, from looking for a repository in `dir` or above
/// it. A script in a source without its own `.git` would otherwise find the project's
/// repository. A ceiling already in the environment is kept.
pub fn git_ceiling(cmd: &mut Command, dir: &Path) -> Result<()> {
    // git compares the ceiling with the real path of the working directory.
    let dir = dir
        .canonicalize()
        .with_context(|| format!("resolving {}", dir.display()))?;
    let mut ceilings = vec![without_verbatim(dir)];
    if let Some(old) = std::env::var_os("GIT_CEILING_DIRECTORIES") {
        ceilings.extend(std::env::split_paths(&old));
    }
    cmd.env("GIT_CEILING_DIRECTORIES", std::env::join_paths(ceilings)?);
    Ok(())
}

/// `path` without the `\\?\` prefix `canonicalize` gives it on Windows, which git doesn't
/// read: `\\?\C:\x` becomes `C:\x`, and `\\?\UNC\host\share` becomes `\\host\share`.
fn without_verbatim(path: PathBuf) -> PathBuf {
    let Some(rest) = path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) else {
        return path;
    };
    match rest.strip_prefix(r"UNC\") {
        Some(unc) => PathBuf::from(format!(r"\\{unc}")),
        None => PathBuf::from(rest),
    }
}

fn describe(cmd: &Command) -> String {
    let mut s = cmd.get_program().to_string_lossy().into_owned();
    for arg in cmd.get_args() {
        s.push(' ');
        s.push_str(&arg.to_string_lossy());
    }
    s
}

/// Run with inherited stdio; error on non-zero exit. With `--message-format json` its
/// stdout goes to stderr, so stdout holds only records. While a full-screen view is up
/// (`output::hold`), its output is captured instead, and a failure carries its stderr.
pub fn run(cmd: &mut Command) -> Result<()> {
    if crate::output::holding() {
        return output_quiet(cmd).map(|_| ());
    }
    if crate::output::json() {
        cmd.stdout(std::io::stderr());
    }
    let status = cmd
        .status()
        .with_context(|| format!("failed to start `{}`", describe(cmd)))?;
    if !status.success() {
        return Err(Failed {
            command: describe(cmd),
            status: status.to_string(),
            code: status.code(),
        }
        .into());
    }
    Ok(())
}

/// A command that `run` started exited unsuccessfully.
#[derive(Debug)]
pub struct Failed {
    command: String,
    status: String,
    /// The exit code; none when a signal ended it.
    pub code: Option<i32>,
}

impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` failed ({})", self.command, self.status)
    }
}

impl std::error::Error for Failed {}

/// Run capturing stdout (stderr is passed through, or captured while a full-screen view is
/// up); error on non-zero exit.
pub fn output(cmd: &mut Command) -> Result<String> {
    if crate::output::holding() {
        return output_quiet(cmd);
    }
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("failed to start `{}`", describe(cmd)))?;
    if !out.status.success() {
        bail!("`{}` failed ({})", describe(cmd), out.status);
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// Run capturing stdout and stderr; stderr is printed only when the command fails, or put
/// in the error while a full-screen view is up.
pub fn output_quiet(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .output()
        .with_context(|| format!("failed to start `{}`", describe(cmd)))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if crate::output::holding() {
            bail!(
                "`{}` failed ({}): {}",
                describe(cmd),
                out.status,
                error_line(&stderr)
            );
        }
        eprint!("{stderr}");
        bail!("`{}` failed ({})", describe(cmd), out.status);
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// A command's error on one line: from its first `error` line on, or all of it.
pub fn error_line(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with("error"))
        .unwrap_or(0);
    let msg = lines.get(start..).unwrap_or_default().join(" ");
    if msg.is_empty() {
        "cargo failed".into()
    } else {
        msg
    }
}

/// Find an executable on PATH.
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
#[path = "process_test.rs"]
mod tests;
