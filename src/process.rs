use std::ffi::OsString;
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

fn describe(cmd: &Command) -> String {
    let mut s = cmd.get_program().to_string_lossy().into_owned();
    for arg in cmd.get_args() {
        s.push(' ');
        s.push_str(&arg.to_string_lossy());
    }
    s
}

/// Run with inherited stdio; error on non-zero exit. With `--message-format json` its
/// stdout goes to stderr, so stdout holds only records.
pub fn run(cmd: &mut Command) -> Result<()> {
    if crate::output::json() {
        cmd.stdout(std::io::stderr());
    }
    let status = cmd
        .status()
        .with_context(|| format!("failed to start `{}`", describe(cmd)))?;
    if !status.success() {
        bail!("`{}` failed ({status})", describe(cmd));
    }
    Ok(())
}

/// Run capturing stdout (stderr is passed through); error on non-zero exit.
pub fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("failed to start `{}`", describe(cmd)))?;
    if !out.status.success() {
        bail!("`{}` failed ({})", describe(cmd), out.status);
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// Run capturing stdout and stderr; stderr is printed only when the command fails.
pub fn output_quiet(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .output()
        .with_context(|| format!("failed to start `{}`", describe(cmd)))?;
    if !out.status.success() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        bail!("`{}` failed ({})", describe(cmd), out.status);
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// Find an executable on PATH.
pub fn which(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}
