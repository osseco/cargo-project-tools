use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use semver::{Version, VersionReq};
use serde::Deserialize;

use crate::cli::SUBCOMMAND;
use crate::config::{AuditIgnore, IgnoreEntry};
use crate::source::registry::{self, IndexCache};

/// The parts of cargo-audit's JSON report we use.
#[derive(Debug, Deserialize)]
pub struct Report {
    pub vulnerabilities: Vulnerabilities,
    #[serde(default)]
    pub warnings: BTreeMap<String, Vec<Warning>>,
}

#[derive(Debug, Deserialize)]
pub struct Vulnerabilities {
    #[serde(default)]
    pub list: Vec<Vulnerability>,
}

#[derive(Debug, Deserialize)]
pub struct Vulnerability {
    pub advisory: Advisory,
    pub package: Package,
    #[serde(default)]
    pub versions: Option<Versions>,
}

#[derive(Debug, Deserialize)]
pub struct Warning {
    pub kind: String,
    pub package: Package,
    pub advisory: Option<Advisory>,
}

#[derive(Debug, Deserialize)]
pub struct Advisory {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: Version,
}

#[derive(Debug, Default, Deserialize)]
pub struct Versions {
    #[serde(default)]
    pub patched: Vec<String>,
}

/// cargo-audit's own yanked check only works when it also fetches the DB, so it's turned
/// off and done from the sparse index instead, giving the same result on every run.
const AUDIT_TOML: &str = "[yanked]\nenabled = false\n";

/// Runs cargo-audit, fetching the advisory DB at most once per process.
pub struct Auditor {
    db: PathBuf,
    /// cargo-audit runs here so it picks up our `.cargo/audit.toml`.
    work_dir: PathBuf,
    fetch_pending: bool,
    pub index: IndexCache,
}

impl Auditor {
    /// Fetch on the first call when the DB is missing or `update_db` is set.
    pub fn new(db: PathBuf, work_dir: PathBuf, update_db: bool) -> Result<Self> {
        if crate::process::which("cargo-audit").is_none() {
            bail!(
                "cargo-audit is not installed; install it with `cargo install cargo-audit --locked`"
            );
        }
        std::fs::create_dir_all(work_dir.join(".cargo"))?;
        std::fs::write(work_dir.join(".cargo/audit.toml"), AUDIT_TOML)?;
        let fetch_pending = update_db || !db.exists();
        Ok(Auditor {
            db,
            work_dir,
            fetch_pending,
            index: IndexCache::default(),
        })
    }

    /// Audit lock text given on stdin.
    fn run(&mut self, lock: &str) -> Result<Report> {
        let mut cmd = crate::process::cargo();
        cmd.args(["audit", "--json", "--file", "-"])
            .arg("--db")
            .arg(&self.db)
            .current_dir(&self.work_dir);
        if !self.fetch_pending {
            cmd.args(["--no-fetch", "--stale"]);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .context("failed to start cargo audit")?;
        child
            .stdin
            .take()
            .context("no stdin")?
            .write_all(lock.as_bytes())?;
        let out = child.wait_with_output()?;
        // Non-zero exit just means something was found; only a missing report is an error.
        let report: Report = serde_json::from_slice(&out.stdout)
            .with_context(|| format!("cargo audit produced no report ({})", out.status))?;
        self.fetch_pending = false;
        Ok(report)
    }

    pub fn audit(&mut self, lock_path: &Path) -> Result<Report> {
        let lock = std::fs::read_to_string(lock_path)
            .with_context(|| format!("reading {}", lock_path.display()))?;
        let mut report = self.run(&lock)?;
        let yanked = yanked(&lock, &mut self.index)?;
        if !yanked.is_empty() {
            report
                .warnings
                .entry("yanked".into())
                .or_default()
                .extend(yanked);
        }
        Ok(report)
    }

    /// Fetch the DB by auditing an empty lock.
    pub fn update_db(&mut self) -> Result<()> {
        self.fetch_pending = true;
        self.run("version = 3\n").map(|_| ())
    }
}

/// Today as `YYYY-MM-DD` (UTC).
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to (year, month, day), from Howard Hinnant's date algorithms.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub vulnerabilities: usize,
    pub warnings: usize,
}

struct Ignores<'a> {
    active: Vec<&'a IgnoreEntry>,
    used: BTreeSet<&'a str>,
}

impl<'a> Ignores<'a> {
    fn new(file: &'a AuditIgnore, today: &str, tool: &str) -> Self {
        let mut active = Vec::new();
        for entry in &file.ignore {
            match &entry.expires {
                Some(exp) if exp.as_str() < today => {
                    println!(
                        "  warning: ignore for {} expired on {exp} ({tool})",
                        entry.id
                    );
                }
                _ => active.push(entry),
            }
        }
        Ignores {
            active,
            used: BTreeSet::new(),
        }
    }

    fn matches(&mut self, advisory: &Advisory) -> Option<&'a IgnoreEntry> {
        let hit = self
            .active
            .iter()
            .copied()
            .find(|e| e.id == advisory.id || advisory.aliases.contains(&e.id))?;
        self.used.insert(hit.id.as_str());
        Some(hit)
    }

    fn unused(&self) -> impl Iterator<Item = &&'a IgnoreEntry> {
        self.active
            .iter()
            .filter(|e| !self.used.contains(e.id.as_str()))
    }
}

/// Print the report for one tool, applying its ignore list. Returns what remains.
pub fn evaluate(
    tool: &str,
    report: &Report,
    ignore: &AuditIgnore,
    today: &str,
    index: &mut IndexCache,
) -> Outcome {
    let mut ignores = Ignores::new(ignore, today, tool);
    let mut outcome = Outcome::default();
    // (name, version) → advisories still open for it
    let mut open: BTreeMap<(&str, &Version), Vec<&Vulnerability>> = BTreeMap::new();

    for v in &report.vulnerabilities.list {
        if let Some(entry) = ignores.matches(&v.advisory) {
            println!(
                "  ignored  {} {}@{}: {}",
                v.advisory.id, v.package.name, v.package.version, entry.reason
            );
            continue;
        }
        outcome.vulnerabilities += 1;
        open.entry((&v.package.name, &v.package.version))
            .or_default()
            .push(v);
    }
    for ((name, version), vulns) in &open {
        for v in vulns {
            println!(
                "  error    {} {name}@{version}: {}\n           patched: {}",
                v.advisory.id,
                v.advisory.title,
                patched_reqs(v)
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" or ")
            );
        }
        println!(
            "           fix: {}",
            suggestion(tool, name, version, vulns, index)
        );
    }

    for (kind, list) in &report.warnings {
        for w in list {
            if let Some(adv) = &w.advisory
                && let Some(entry) = ignores.matches(adv)
            {
                println!(
                    "  ignored  {} {}@{} ({kind}): {}",
                    adv.id, w.package.name, w.package.version, entry.reason
                );
                continue;
            }
            outcome.warnings += 1;
            let id = w.advisory.as_ref().map_or("-", |a| a.id.as_str());
            println!(
                "  warning  {id} {}@{} ({})",
                w.package.name, w.package.version, w.kind
            );
            if w.kind == "yanked" {
                println!(
                    "           fix: cargo {SUBCOMMAND} update {tool} {}@{}",
                    w.package.name, w.package.version
                );
            }
        }
    }

    for entry in ignores.unused() {
        println!(
            "  warning: unused ignore {} in audit-ignore.toml ({tool}); remove it",
            entry.id
        );
    }
    outcome
}

fn patched_reqs(v: &Vulnerability) -> Vec<VersionReq> {
    v.versions
        .as_ref()
        .map(|x| {
            x.patched
                .iter()
                .filter_map(|r| VersionReq::parse(r).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// One command fixing every open advisory of a package: `--precise` with the lowest version
/// that is patched for all of them, or a hint when no such version exists.
fn suggestion(
    tool: &str,
    name: &str,
    version: &Version,
    vulns: &[&Vulnerability],
    index: &mut IndexCache,
) -> String {
    let advisories: Vec<Vec<VersionReq>> = vulns.iter().map(|v| patched_reqs(v)).collect();
    if advisories.iter().any(Vec::is_empty) {
        return "no patched version for every advisory; replace the dependency or ignore with a reason in audit-ignore.toml".to_string();
    }
    match registry::lowest_patched(index.get(name), version, &advisories) {
        Some(e) => format!(
            "cargo {SUBCOMMAND} update {tool} {name}@{version} --precise {}",
            e.vers
        ),
        None => format!("cargo {SUBCOMMAND} update {tool} {name}@{version}"),
    }
}

/// Registry packages in the lock whose exact version is yanked, as cargo-audit warnings.
pub fn yanked(lock_text: &str, index: &mut IndexCache) -> Result<Vec<Warning>> {
    let lock = crate::lock::parse(lock_text)?;
    let pkgs: Vec<_> = lock
        .packages
        .iter()
        .filter(|p| p.source.as_ref().is_some_and(|s| s.is_default_registry()))
        .collect();
    index.prefetch(pkgs.iter().map(|p| p.name.as_str()));
    Ok(pkgs
        .into_iter()
        .filter(|p| index.is_yanked(p.name.as_str(), &p.version))
        .map(|p| Warning {
            kind: "yanked".into(),
            package: Package {
                name: p.name.as_str().into(),
                version: p.version.clone(),
            },
            advisory: None,
        })
        .collect())
}

#[cfg(test)]
#[path = "audit_test.rs"]
mod tests;
