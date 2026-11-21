use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use rustsec::database::Query;
use rustsec::{Collection, Database, Lockfile, SourceId};
use semver::{Version, VersionReq};

use crate::cli::SUBCOMMAND;
use crate::config::{AuditIgnore, IgnoreEntry, SELF_AUDIT_IGNORE};
use crate::graph::Built;
use crate::source::registry::{self, IndexCache};

/// Kind of the warning for an advisory matched on a git package by name and version only.
pub const GIT_SOURCE: &str = "git-source";

/// What one lock's audit found, before the ignore list is applied.
#[derive(Debug, Default)]
pub struct Report {
    pub vulnerabilities: Vec<Vulnerability>,
    /// By kind: `unmaintained`, `unsound`, `notice`, `yanked`, `git-source`.
    pub warnings: BTreeMap<String, Vec<Warning>>,
    /// Lock packages left out because the tool's features don't build them.
    pub skipped: usize,
}

#[derive(Debug)]
pub struct Vulnerability {
    pub advisory: Advisory,
    pub package: Package,
    pub patched: Vec<VersionReq>,
}

#[derive(Debug)]
pub struct Warning {
    pub kind: String,
    pub package: Package,
    pub advisory: Option<Advisory>,
}

#[derive(Debug)]
pub struct Advisory {
    pub id: String,
    pub title: String,
    pub aliases: Vec<String>,
}

#[derive(Debug)]
pub struct Package {
    pub name: String,
    pub version: Version,
}

impl From<&rustsec::Advisory> for Advisory {
    fn from(a: &rustsec::Advisory) -> Self {
        Advisory {
            id: a.metadata.id.to_string(),
            title: a.metadata.title.clone(),
            aliases: a.metadata.aliases.iter().map(ToString::to_string).collect(),
        }
    }
}

impl From<&rustsec::package::Package> for Package {
    fn from(p: &rustsec::package::Package) -> Self {
        Package {
            name: p.name.as_str().into(),
            version: p.version.clone(),
        }
    }
}

/// How a lock entry is matched against the database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// A crates.io package: matched as rustsec does.
    Registry,
    /// A git package: advisories are for crates.io releases, so a name and version match
    /// only means "possibly affected".
    Git,
}

/// The root package has no `source` in the lock, so it's placed by where the tool comes
/// from. Path packages and other registries aren't checked.
fn scope(pkg: &rustsec::package::Package, root: &str, git_tool: bool) -> Option<Scope> {
    match &pkg.source {
        Some(s) if s.is_default_registry() => Some(Scope::Registry),
        Some(s) if s.is_git() => Some(Scope::Git),
        None if pkg.name.as_str() == root => Some(if git_tool {
            Scope::Git
        } else {
            Scope::Registry
        }),
        _ => None,
    }
}

/// Advisories for one package version, like rustsec's crate scope: crates collection, no
/// withdrawn advisories, informational ones included so they can become warnings.
fn query<'a>(
    db: &'a Database,
    name: &str,
    version: &Version,
    scope: Scope,
) -> Vec<&'a rustsec::Advisory> {
    let Ok(name) = name.parse() else {
        return Vec::new();
    };
    let mut q = Query::new()
        .collection(Collection::Crates)
        .withdrawn(false)
        .package_name(name)
        .package_version(version.clone());
    if scope == Scope::Registry {
        // Advisories without a source are for crates.io, which is `SourceId::default()`.
        q = q.package_source(SourceId::default());
    }
    db.query(&q)
}

/// Match every checked package in the lock, the root included.
pub fn scan(db: &Database, lock: &Lockfile, root: &str, git_tool: bool) -> Report {
    let mut report = Report::default();
    for pkg in &lock.packages {
        let Some(scope) = scope(pkg, root, git_tool) else {
            continue;
        };
        for advisory in query(db, pkg.name.as_str(), &pkg.version, scope) {
            let kind = match (&advisory.metadata.informational, scope) {
                (_, Scope::Git) => GIT_SOURCE.to_string(),
                (None, Scope::Registry) => {
                    report.vulnerabilities.push(Vulnerability {
                        advisory: advisory.into(),
                        package: pkg.into(),
                        patched: advisory.versions.patched().to_vec(),
                    });
                    continue;
                }
                // Kinds rustsec has no warning for (`Other`) are skipped, as cargo-audit does.
                (Some(info), Scope::Registry) => match info.warning_kind() {
                    Some(kind) => kind.as_str().to_string(),
                    None => continue,
                },
            };
            report
                .warnings
                .entry(kind.clone())
                .or_default()
                .push(Warning {
                    kind,
                    package: pkg.into(),
                    advisory: Some(advisory.into()),
                });
        }
    }
    report
}

/// Advisories against this binary itself, as cargo-audit checks its own version.
pub fn self_check(db: &Database) -> Vec<(Package, Advisory)> {
    let mut out = Vec::new();
    for (name, version) in [
        (env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
        ("rustsec", rustsec::VERSION),
    ] {
        let Ok(version) = Version::parse(version) else {
            continue;
        };
        for a in query(db, name, &version, Scope::Registry) {
            if a.metadata.informational.is_none() {
                let package = Package {
                    name: name.to_string(),
                    version: version.clone(),
                };
                out.push((package, a.into()));
            }
        }
    }
    out
}

/// Print the self-check findings, applying `self-audit-ignore.toml`. Returns how many are
/// left.
pub fn evaluate_self(found: &[(Package, Advisory)], ignore: &AuditIgnore, today: &str) -> usize {
    let mut ignores = Ignores::new(ignore, today, &format!("cargo-{SUBCOMMAND}"));
    let mut open = 0;
    for (package, advisory) in found {
        if let Some(entry) = ignores.matches(advisory) {
            println!(
                "  ignored  {} {}@{}: {}",
                advisory.id, package.name, package.version, entry.reason
            );
            continue;
        }
        open += 1;
        println!(
            "  error    {} {}@{}: {}\n           fix: upgrade cargo-{SUBCOMMAND}, or ignore it in .project-tools/{SELF_AUDIT_IGNORE}",
            advisory.id, package.name, package.version, advisory.title
        );
    }
    for entry in ignores.unused() {
        println!(
            "  warning: unused ignore {} in {SELF_AUDIT_IGNORE}; remove it",
            entry.id
        );
    }
    open
}

/// Audits locks against one loaded database.
pub struct Auditor {
    db: Database,
    pub index: IndexCache,
}

impl Auditor {
    pub fn new(db: Database) -> Self {
        Auditor {
            db,
            index: IndexCache::default(),
        }
    }

    pub fn self_check(&self) -> Vec<(Package, Advisory)> {
        self_check(&self.db)
    }

    /// `root` is the tool's package name; `git_tool` whether it's installed from git. Only
    /// the lock's packages in `built` are checked.
    pub fn audit(
        &mut self,
        lock_path: &Path,
        root: &str,
        git_tool: bool,
        built: &Built,
    ) -> Result<Report> {
        let text = std::fs::read_to_string(lock_path)
            .with_context(|| format!("reading {}", lock_path.display()))?;
        let mut lock = crate::lock::parse(&text)?;
        let total = lock.packages.len();
        lock.packages
            .retain(|p| built.contains(&(p.name.as_str().to_string(), p.version.clone())));
        let mut report = scan(&self.db, &lock, root, git_tool);
        report.skipped = total - lock.packages.len();
        let yanked = yanked(&lock, root, git_tool, &mut self.index);
        if !yanked.is_empty() {
            report
                .warnings
                .entry("yanked".into())
                .or_default()
                .extend(yanked);
        }
        Ok(report)
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

    for v in &report.vulnerabilities {
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
                v.patched
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
            if w.kind == GIT_SOURCE
                && let Some(adv) = &w.advisory
            {
                println!(
                    "           possibly affected (git source, matched by name and version): {}",
                    adv.title
                );
            } else if w.kind == "yanked" {
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

/// One command fixing every open advisory of a package: `--precise` with the lowest version
/// that is patched for all of them, or a hint when no such version exists.
fn suggestion(
    tool: &str,
    name: &str,
    version: &Version,
    vulns: &[&Vulnerability],
    index: &mut IndexCache,
) -> String {
    if name == tool {
        return format!("cargo {SUBCOMMAND} update {tool} --latest");
    }
    let advisories: Vec<Vec<VersionReq>> = vulns.iter().map(|v| v.patched.clone()).collect();
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

/// crates.io packages in the lock whose exact version is yanked, the root of a registry tool
/// included.
pub fn yanked(lock: &Lockfile, root: &str, git_tool: bool, index: &mut IndexCache) -> Vec<Warning> {
    let pkgs: Vec<_> = lock
        .packages
        .iter()
        .filter(|p| scope(p, root, git_tool) == Some(Scope::Registry))
        .collect();
    index.prefetch(pkgs.iter().map(|p| p.name.as_str()));
    pkgs.into_iter()
        .filter(|p| index.is_yanked(p.name.as_str(), &p.version))
        .map(|p| Warning {
            kind: "yanked".into(),
            package: p.into(),
            advisory: None,
        })
        .collect()
}

#[cfg(test)]
#[path = "audit_test.rs"]
mod tests;
