use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::process;

/// Whether `s` looks like a target triple (`arch-vendor-os[-env]`): parts of ASCII letters,
/// digits, `_` and `.`, joined by `-`. Enough to keep option-like or path-like values out.
pub fn is_triple(s: &str) -> bool {
    s.contains('-')
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        })
}

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

/// A target triple offered at `init`, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub triple: String,
    /// Where it was seen: "this machine" or workflow files.
    pub sources: Vec<String>,
}

/// The triple a GitHub-hosted runner label builds on natively, if it's one we know.
fn runner_triple(label: &str) -> Option<&'static str> {
    const LINUX: &str = "x86_64-unknown-linux-gnu";
    const LINUX_ARM: &str = "aarch64-unknown-linux-gnu";
    const MAC: &str = "aarch64-apple-darwin";
    const MAC_INTEL: &str = "x86_64-apple-darwin";
    const WINDOWS: &str = "x86_64-pc-windows-msvc";
    const WINDOWS_ARM: &str = "aarch64-pc-windows-msvc";
    let is_version = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.');

    if let Some(rest) = label.strip_prefix("ubuntu-") {
        let (version, arm) = match rest.strip_suffix("-arm") {
            Some(v) => (v, true),
            None => (rest, false),
        };
        if version == "latest" || is_version(version) {
            return Some(if arm { LINUX_ARM } else { LINUX });
        }
    } else if let Some(rest) = label.strip_prefix("macos-") {
        let (version, size) = rest.split_once('-').unwrap_or((rest, ""));
        let intel = match size {
            "" | "xlarge" => version.parse::<u32>().is_ok_and(|v| v <= 13),
            "large" | "intel" => true,
            _ => return None,
        };
        if version == "latest" || is_version(version) {
            return Some(if intel { MAC_INTEL } else { MAC });
        }
    } else if let Some(rest) = label.strip_prefix("windows-") {
        if rest == "11-arm" {
            return Some(WINDOWS_ARM);
        }
        if rest == "latest" || is_version(rest) {
            return Some(WINDOWS);
        }
    }
    None
}

/// Triples of the runners the project's GitHub workflows use, found as runner labels
/// anywhere in `.github/workflows/*.yml`: in `runs-on`, matrix lists and `include` alike.
/// The workflows are looked for from `root` up to the git repository's top.
fn from_workflows(root: &Path) -> Vec<Suggestion> {
    let mut found = None;
    for d in root.ancestors() {
        if d.join(".github/workflows").is_dir() {
            found = Some(d.join(".github/workflows"));
            break;
        }
        if d.join(".git").exists() {
            break;
        }
    }
    let Some(dir) = found else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "yml" || x == "yaml"))
        .collect();
    files.sort();
    let mut out: Vec<Suggestion> = Vec::new();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let name = file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let tokens = text.split(|c: char| !(c.is_ascii_alphanumeric() || "._-".contains(c)));
        for triple in tokens.filter_map(runner_triple) {
            add(&mut out, triple, &name);
        }
    }
    out
}

fn add(list: &mut Vec<Suggestion>, triple: &str, source: &str) {
    match list.iter_mut().find(|s| s.triple == triple) {
        Some(s) if !s.sources.iter().any(|x| x == source) => s.sources.push(source.into()),
        Some(_) => {}
        None => list.push(Suggestion {
            triple: triple.into(),
            sources: vec![source.into()],
        }),
    }
}

/// This machine first, then the runners of the project's workflows.
pub fn suggestions(root: &Path) -> Vec<Suggestion> {
    let mut out = Vec::new();
    match host() {
        Ok(h) => add(&mut out, &h, "this machine"),
        Err(e) => eprintln!("note: couldn't tell this machine's target: {e:#}"),
    }
    for s in from_workflows(root) {
        for source in &s.sources {
            add(&mut out, &s.triple, source);
        }
    }
    out
}

/// Read an answer to the targets prompt: numbers of `suggestions` and triples, separated by
/// commas or spaces; empty takes every suggestion; `all` means every target (`None`).
pub fn parse_choice(answer: &str, suggestions: &[Suggestion]) -> Result<Option<Vec<String>>> {
    let answer = answer.trim();
    if answer.eq_ignore_ascii_case("all") {
        return Ok(None);
    }
    if answer.is_empty() {
        return Ok(Some(suggestions.iter().map(|s| s.triple.clone()).collect()));
    }
    let mut out: Vec<String> = Vec::new();
    for part in answer
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|p| !p.is_empty())
    {
        let triple = match part.parse::<usize>() {
            Ok(n) => suggestions
                .get(n.wrapping_sub(1))
                .map(|s| s.triple.clone())
                .with_context(|| format!("no suggestion {n}"))?,
            Err(_) if is_triple(part) => part.to_string(),
            Err(_) => bail!("`{part}` is neither a number from the list nor a target triple"),
        };
        if !out.contains(&triple) {
            out.push(triple);
        }
    }
    Ok(Some(out))
}

#[cfg(test)]
#[path = "targets_test.rs"]
mod tests;
