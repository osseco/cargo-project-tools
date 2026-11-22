use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::config::SfwMode;
use crate::output::{self, Record};
use crate::process;

/// Set in the re-executed process so it doesn't wrap itself again.
const MARKER: &str = "CARGO_PROJECT_TOOLS_UNDER_SFW";

/// Whether this process already runs under sfw: either we re-executed ourselves, or the user
/// wrapped the command (`sfw cargo project-tools …`). sfw exports a proxy plus its own CA
/// bundle, named `socketFirewallCaBundle…`, in `SSL_CERT_FILE`.
fn under_sfw() -> bool {
    if std::env::var_os(MARKER).is_some() {
        return true;
    }
    let ca = std::env::var_os("SSL_CERT_FILE").unwrap_or_default();
    std::env::var_os("HTTPS_PROXY").is_some() && ca.to_string_lossy().contains("socketFirewall")
}

/// Re-run this process under `sfw` when configured. Returns the child's exit code when it
/// did, or `None` when the current process should carry on.
pub fn reexec(mode: SfwMode) -> Result<Option<i32>> {
    if mode == SfwMode::Off {
        eprintln!("note: not using sfw (sfw is off)");
        return Ok(None);
    }
    if under_sfw() {
        if std::env::var_os(MARKER).is_none() {
            eprintln!("note: already running under sfw");
        }
        return Ok(None);
    }
    let Some(sfw) = process::which("sfw") else {
        if mode == SfwMode::Require {
            bail!("sfw is required (settings.sfw = \"require\") but not found on PATH");
        }
        let message = "sfw not found on PATH; downloads are not filtered";
        eprintln!("note: {message}");
        output::emit(Record::SfwMissing {
            message: message.to_string(),
        });
        return Ok(None);
    };
    let setting = match mode {
        SfwMode::Require => "require",
        _ => "auto",
    };
    eprintln!("note: running through sfw (settings.sfw = \"{setting}\")");
    let exe = std::env::current_exe().context("locating own executable")?;
    let status = Command::new(sfw)
        .arg(exe)
        .args(std::env::args_os().skip(1))
        .env(MARKER, "1")
        .status()
        .context("failed to start sfw")?;
    Ok(Some(status.code().unwrap_or(1)))
}
