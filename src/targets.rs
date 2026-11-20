use std::ffi::OsString;
use std::process::Command;

use anyhow::{Context, Result};

use crate::process;

/// The target triple of this machine, from `rustc -vV`.
pub fn host() -> Result<String> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let out = process::output(Command::new(rustc).arg("-vV"))?;
    out.lines()
        .find_map(|l| l.strip_prefix("host: "))
        .map(|h| h.trim().to_string())
        .context("`rustc -vV` printed no host")
}

/// The triples among `host` and `build_target` that `allowed` doesn't list. Build scripts
/// and proc macros run on the host, so it must be approved even when building for another
/// target. `None` allows everything.
pub fn missing(host: &str, build_target: Option<&str>, allowed: Option<&[String]>) -> Vec<String> {
    let Some(allowed) = allowed else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for t in std::iter::once(host).chain(build_target) {
        if !allowed.iter().any(|a| a == t) && !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

#[cfg(test)]
#[path = "targets_test.rs"]
mod tests;
