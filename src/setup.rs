//! What the interactive setup works on: a tool's checkout and lock, the features chosen,
//! the dependency tree they build, and what the advisory database says about it. Every
//! cargo run here captures its output, so a full-screen view stays intact; a failure comes
//! back as cargo's message.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::process::Command;

use anyhow::{Context, Result, bail};
use semver::{Version, VersionReq};
use serde::Deserialize;

use crate::audit::{self, Auditor, Report};
use crate::config::AuditIgnore;
use crate::graph;
use crate::install;
use crate::lock;
use crate::process;
use crate::source::Checkout;
use crate::source::registry;

/// A package in the dependency tree. The tree holds one group per target: first what every
/// target builds, then for each target only what it adds. Each group starts with the tool at
/// depth 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dep {
    pub depth: usize,
    pub name: String,
    pub version: Version,
    /// What cargo shows in parentheses: a path or git source, or `proc-macro`; empty for
    /// crates.io.
    pub source: String,
    /// Listed before in its group; its dependencies are shown there.
    pub repeat: bool,
    /// A build dependency of the package above it.
    pub build: bool,
    /// In a target's group: on the way to what the target adds, and built for every target.
    pub shared: bool,
    /// `None` for what every target builds, else the one target this group is for.
    pub target: Option<String>,
}

/// A package as the tree knows it: name, version, source.
type Pkg = (String, Version, String);
/// What each package depends on: (build dependency, package).
type Edges = BTreeMap<Pkg, BTreeSet<(bool, Pkg)>>;

/// What the advisory database says about a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// `vulnerability`, or a warning kind: `unmaintained`, `unsound`, `notice`, `yanked`,
    /// `git-source`.
    pub kind: String,
    pub id: String,
    pub title: String,
    /// Versions with a fix, for a vulnerability.
    pub patched: Vec<VersionReq>,
    /// In the tool's `audit-ignore.toml`.
    pub ignored: bool,
}

impl Finding {
    pub fn is_vulnerability(&self) -> bool {
        self.kind == "vulnerability"
    }
}

/// A version a package could move to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub version: Version,
    /// Semver-compatible with the current version.
    pub compatible: bool,
    /// The lowest version that fixes its advisories.
    pub suggested: bool,
    pub current: bool,
    /// The version in the lock when the setup started.
    pub original: bool,
    /// A pre-release (`1.0.0-rc.1`).
    pub prerelease: bool,
}

pub struct Setup {
    pub name: String,
    pub checkout: Checkout,
    /// Installed from git, which the audit flags.
    pub git: bool,
    /// The tool's features and what each enables.
    pub features: BTreeMap<String, Vec<String>>,
    /// Features turned on besides the defaults.
    pub selected: BTreeSet<String>,
    pub default_features: bool,
    pub targets: Option<Vec<String>>,
    pub tree: Vec<Dep>,
    /// Every package built, for any of the targets.
    pub built: graph::Built,
    pub findings: BTreeMap<(String, Version), Vec<Finding>>,
    /// Each package's versions in the lock when the setup started, the tool's included.
    pub original: BTreeMap<String, BTreeSet<Version>>,
    auditor: Auditor,
    ignore: AuditIgnore,
    today: String,
    /// Extra arguments for every cargo run (tests point cargo at an offline registry).
    cargo_args: Vec<String>,
}

impl Setup {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        checkout: Checkout,
        git: bool,
        selected: BTreeSet<String>,
        default_features: bool,
        targets: Option<Vec<String>>,
        auditor: Auditor,
        ignore: AuditIgnore,
        cargo_args: Vec<String>,
    ) -> Result<Setup> {
        let mut setup = Setup {
            name: name.to_string(),
            checkout,
            git,
            features: BTreeMap::new(),
            selected,
            default_features,
            targets,
            tree: Vec::new(),
            built: graph::Built::new(),
            findings: BTreeMap::new(),
            original: BTreeMap::new(),
            auditor,
            ignore,
            today: audit::today(),
            cargo_args,
        };
        for p in lock::load(&setup.checkout.lock_path())?.packages {
            setup
                .original
                .entry(p.name.as_str().to_string())
                .or_default()
                .insert(p.version);
        }
        setup.load_features()?;
        setup.refresh()?;
        Ok(setup)
    }

    /// The feature flags for cargo.
    pub fn feature_args(&self) -> Vec<String> {
        let selected: Vec<String> = self.selected.iter().cloned().collect();
        install::feature_flags(&selected, self.default_features)
    }

    fn cargo(&self, sub: &str) -> Command {
        let mut cmd = process::cargo();
        // Plain text: the output is parsed and shown on the screen, whatever
        // `CARGO_TERM_COLOR` says.
        cmd.env("CARGO_TERM_COLOR", "never")
            .arg(sub)
            .arg("--manifest-path")
            .arg(self.checkout.manifest_path());
        cmd
    }

    /// Run with output captured; a failure is cargo's message.
    fn run(&self, mut cmd: Command) -> Result<String> {
        cmd.args(&self.cargo_args);
        let out = cmd.output().context("starting cargo")?;
        if !out.status.success() {
            bail!(
                "{}",
                process::error_line(&String::from_utf8_lossy(&out.stderr))
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// The tool's features, from `cargo metadata`. Selected features the tool no longer has
    /// are dropped.
    fn load_features(&mut self) -> Result<()> {
        #[derive(Deserialize)]
        struct Metadata {
            packages: Vec<Package>,
        }
        #[derive(Deserialize)]
        struct Package {
            name: String,
            features: BTreeMap<String, Vec<String>>,
        }
        let mut cmd = self.cargo("metadata");
        cmd.args(["--format-version", "1", "--no-deps"]);
        let meta: Metadata = serde_json::from_str(&self.run(cmd)?)?;
        let pkg = meta
            .packages
            .into_iter()
            .find(|p| p.name == self.name)
            .with_context(|| format!("{} isn't in its own checkout", self.name))?;
        self.features = pkg.features;
        self.selected.retain(|f| self.features.contains_key(f));
        Ok(())
    }

    /// The tree and findings for the current features, targets and lock. `cargo tree` runs
    /// once per target: given several, it prints a tree for each without saying which.
    fn refresh(&mut self) -> Result<()> {
        let targets: Vec<Option<String>> = match &self.targets {
            Some(list) if !list.is_empty() => list.iter().cloned().map(Some).collect(),
            _ => vec![None],
        };
        let mut graphs = Vec::new();
        for target in targets {
            let mut cmd = self.cargo("tree");
            cmd.args([
                "--locked",
                "--edges",
                "normal,build",
                "--prefix",
                "indent",
                "--charset",
                "ascii",
                "--format",
                "{p}",
            ])
            .args(self.feature_args())
            .args(graph::target_args(
                target.as_ref().map(std::slice::from_ref),
            ));
            let lines = parse_tree(&self.run(cmd)?)?;
            graphs.push((target.unwrap_or_default(), edges(&lines)));
        }
        self.built = graphs
            .iter()
            .flat_map(|(_, (_, e))| e.keys())
            .map(|(name, version, _)| (name.clone(), version.clone()))
            .collect();
        self.tree = layout(&graphs);
        let report = self.auditor.audit(
            &self.checkout.lock_path(),
            &self.name,
            self.git,
            &self.built,
        )?;
        self.findings = findings(&report, &self.ignore, &self.today);
        Ok(())
    }

    /// Turn features on or off, and see what that builds.
    pub fn set_features(
        &mut self,
        selected: BTreeSet<String>,
        default_features: bool,
    ) -> Result<()> {
        self.selected = selected;
        self.default_features = default_features;
        self.refresh()
    }

    /// Move a dependency to `to`. When cargo can't resolve that, the lock is left as it was
    /// and cargo's message is the error.
    pub fn set_version(&mut self, name: &str, from: &Version, to: &Version) -> Result<()> {
        let lock_path = self.checkout.lock_path();
        let backup = fs::read(&lock_path)?;
        let mut cmd = self.cargo("update");
        cmd.args([
            "-p",
            &format!("{name}@{from}"),
            "--precise",
            &to.to_string(),
        ]);
        if let Err(e) = self.run(cmd) {
            fs::write(&lock_path, backup)?;
            return Err(e);
        }
        // Cargo can succeed without making the move (it ignores `--precise` for a replaced
        // source, for one); don't report a move that didn't happen.
        let moved = lock::load(&lock_path)?
            .packages
            .iter()
            .any(|p| p.name.as_str() == name && p.version == *to);
        if !moved {
            fs::write(&lock_path, backup)?;
            bail!("cargo didn't move {name} to {to}");
        }
        if let Err(e) = self.refresh() {
            fs::write(&lock_path, backup)?;
            self.refresh()?;
            return Err(e);
        }
        Ok(())
    }

    /// Switch to another checkout of the tool (another version), keeping the features it
    /// still has.
    pub fn set_checkout(&mut self, checkout: Checkout) -> Result<()> {
        self.checkout = checkout;
        self.load_features()?;
        self.refresh()
    }

    /// Which features pull in `name@version`: the inverted `cargo tree -e features`.
    pub fn why(&self, name: &str, version: &Version) -> Result<Vec<String>> {
        let mut cmd = self.cargo("tree");
        cmd.args([
            "--locked",
            "-i",
            &format!("{name}@{version}"),
            "-e",
            "features",
        ])
        .args(self.feature_args())
        .args(graph::target_args(self.targets.as_deref()));
        Ok(self.run(cmd)?.lines().map(str::to_string).collect())
    }

    /// The command `audit` suggests for the open advisories of `name@version`, given the
    /// patched ranges of each.
    pub fn fix(&mut self, name: &str, version: &Version, patched: &[Vec<VersionReq>]) -> String {
        crate::audit::fix_for(&self.name, name, version, patched, &mut self.auditor.index)
    }

    /// Versions `name` could move to from `current`, newest first, yanked ones left out. The
    /// suggested one is the lowest that every open advisory with a patched range calls fixed;
    /// there's none when a vulnerability has no fix at all.
    pub fn versions(&mut self, name: &str, current: &Version) -> Vec<Choice> {
        let open: Vec<&Finding> = self
            .findings
            .get(&(name.to_string(), current.clone()))
            .into_iter()
            .flatten()
            .filter(|f| !f.ignored)
            .collect();
        let unfixable = open
            .iter()
            .any(|f| f.is_vulnerability() && f.patched.is_empty());
        let patched: Vec<Vec<VersionReq>> = open
            .iter()
            .filter(|f| !f.patched.is_empty())
            .map(|f| f.patched.clone())
            .collect();
        let entries = self.auditor.index.get(name);
        let suggested = if patched.is_empty() || unfixable {
            None
        } else {
            registry::lowest_patched(entries, current, &patched).map(|e| e.vers.clone())
        };
        let mut out: Vec<Choice> = entries
            .iter()
            .filter(|e| !e.yanked || e.vers == *current)
            .map(|e| Choice {
                compatible: lock::compatible(&e.vers, current),
                suggested: suggested.as_ref() == Some(&e.vers),
                current: e.vers == *current,
                prerelease: !e.vers.pre.is_empty(),
                original: self.original.get(name).is_some_and(|o| o.contains(&e.vers)),
                version: e.vers.clone(),
            })
            .collect();
        out.sort_by(|a, b| b.version.cmp(&a.version));
        out.dedup_by(|a, b| a.version == b.version);
        out
    }

    /// What `name` was when the setup started, when `version` isn't one of those: its
    /// original versions that are no longer built.
    pub fn was(&self, name: &str, version: &Version) -> Vec<Version> {
        let Some(original) = self.original.get(name) else {
            return Vec::new();
        };
        if original.contains(version) {
            return Vec::new();
        }
        let now: BTreeSet<&Version> = self
            .built
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v)
            .collect();
        original
            .iter()
            .filter(|v| !now.contains(v))
            .cloned()
            .collect()
    }

    /// How many packages are built, and how many have open vulnerabilities and warnings.
    pub fn counts(&self) -> (usize, usize, usize) {
        let open = |vuln: bool| {
            self.findings
                .values()
                .filter(|fs| {
                    fs.iter()
                        .any(|f| !f.ignored && f.is_vulnerability() == vuln)
                })
                .count()
        };
        (self.built.len(), open(true), open(false))
    }

    /// The packages in the lock with build-time code, by kind (`build-script`, `proc-macro`,
    /// or `build-dep` for a library build-time code runs), from `cargo metadata`. Whether
    /// one is built depends on the features.
    pub fn build_time(&self) -> Result<BTreeSet<(String, String, Version)>> {
        #[derive(Deserialize)]
        struct Metadata {
            packages: Vec<Package>,
        }
        #[derive(Deserialize)]
        struct Package {
            name: String,
            version: Version,
            targets: Vec<Target>,
        }
        #[derive(Deserialize)]
        struct Target {
            kind: Vec<String>,
        }
        // Every feature of the tool, so it lists whatever any choice of them could build.
        let mut cmd = self.cargo("metadata");
        cmd.args(["--format-version", "1", "--locked", "--all-features"]);
        let json = self.run(cmd)?;
        let meta: Metadata = serde_json::from_str(&json)?;
        let mut out = BTreeSet::new();
        // With every feature on, this can name more than one choice of features runs at
        // build time; the review itself resolves the choice made.
        for (name, version) in crate::review::build_deps_in(&json)? {
            if let Ok(version) = Version::parse(&version) {
                out.insert(("build-dep".to_string(), name, version));
            }
        }
        for p in meta.packages {
            for kind in p.targets.iter().flat_map(|t| &t.kind) {
                let kind = match kind.as_str() {
                    "custom-build" => "build-script",
                    "proc-macro" => "proc-macro",
                    _ => continue,
                };
                out.insert((kind.to_string(), p.name.clone(), p.version.clone()));
            }
        }
        Ok(out)
    }

    /// The advisory with this id, if the database has it.
    pub fn advisory(&self, id: &str) -> Option<&rustsec::Advisory> {
        self.auditor.advisory(id)
    }

    /// The lock as it is now.
    pub fn lock_path(&self) -> std::path::PathBuf {
        self.checkout.lock_path()
    }
}

/// Parse `cargo tree --prefix indent --charset ascii --format {p}` lines:
/// `|   |-- name v1.2.3 (source) (*)`, four columns a level. A `[build-dependencies]` line
/// marks the dependencies after it, at the next level, as build dependencies.
fn parse_tree(out: &str) -> Result<Vec<Dep>> {
    let mut deps = Vec::new();
    // Whether the dependencies at each depth are build dependencies, so far.
    let mut section: Vec<bool> = Vec::new();
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        let indent = line
            .chars()
            .take_while(|c| matches!(c, '|' | '`' | '-' | ' '))
            .count();
        // The prefix is ASCII, one byte a character.
        let rest = &line[indent..];
        let depth = indent / 4;
        if let Some(header) = rest.strip_prefix('[') {
            let level = depth + 1;
            section.truncate(level);
            section.resize(level + 1, false);
            section[level] = header.starts_with("build-dependencies]");
            continue;
        }
        let build = section.get(depth).copied().unwrap_or(false);
        // What follows at the next level starts as normal dependencies.
        section.truncate(depth + 1);
        let (rest, repeat) = match rest.strip_suffix(" (*)") {
            Some(r) => (r, true),
            None => (rest, false),
        };
        let mut parts = rest.splitn(3, ' ');
        let (Some(name), Some(version)) = (parts.next(), parts.next()) else {
            bail!("unexpected `cargo tree` line: {line}");
        };
        let version = version
            .strip_prefix('v')
            .and_then(|v| Version::parse(v).ok())
            .with_context(|| format!("unexpected `cargo tree` line: {line}"))?;
        let source = parts
            .next()
            .unwrap_or_default()
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .to_string();
        deps.push(Dep {
            depth,
            name: name.to_string(),
            version,
            source,
            repeat,
            build,
            shared: false,
            target: None,
        });
    }
    if deps.is_empty() {
        bail!("`cargo tree` listed nothing");
    }
    if deps[0].depth != 0 || deps[1..].iter().any(|d| d.depth == 0) {
        bail!("`cargo tree` listed more than one root");
    }
    Ok(deps)
}

fn pkg(d: &Dep) -> Pkg {
    (d.name.clone(), d.version.clone(), d.source.clone())
}

/// The root and what each package depends on, from one `cargo tree`. Cargo lists a package's
/// dependencies only where it first shows it; a package it shows more than once, with other
/// features, gets what any of them depends on.
fn edges(lines: &[Dep]) -> (Pkg, Edges) {
    let mut edges = Edges::new();
    let mut path: Vec<Pkg> = Vec::new();
    for d in lines {
        let p = pkg(d);
        edges.entry(p.clone()).or_default();
        path.truncate(d.depth);
        if let Some(parent) = path.last() {
            edges
                .entry(parent.clone())
                .or_default()
                .insert((d.build, p.clone()));
        }
        path.push(p);
    }
    (pkg(&lines[0]), edges)
}

/// The tree to show: what every target builds, then per target only the way to what it
/// adds. With one target (or `--target all`) there's only the first group.
fn layout(graphs: &[(String, (Pkg, Edges))]) -> Vec<Dep> {
    let Some((_, (root, first))) = graphs.first() else {
        return Vec::new();
    };
    // The dependencies every target has.
    let mut common = Edges::new();
    for (p, children) in first {
        let mut shared = children.clone();
        for (_, (_, other)) in &graphs[1..] {
            match other.get(p) {
                Some(c) => shared.retain(|e| c.contains(e)),
                None => shared.clear(),
            }
        }
        common.insert(p.clone(), shared);
    }
    let mut out = Vec::new();
    let mut walk = Walk {
        out: &mut out,
        edges: &common,
        common: &common,
        target: None,
        seen: BTreeSet::new(),
        adds: BTreeMap::new(),
    };
    walk.push(root, 0, false, false);
    if graphs.len() > 1 {
        for (target, (root, edges)) in graphs {
            let mut walk = Walk {
                out: &mut out,
                edges,
                common: &common,
                target: Some(target),
                seen: BTreeSet::new(),
                adds: BTreeMap::new(),
            };
            if walk.adds(root) {
                walk.push(root, 0, false, false);
            }
        }
    }
    out
}

/// One group of the tree, depth first.
struct Walk<'a> {
    out: &'a mut Vec<Dep>,
    edges: &'a Edges,
    common: &'a Edges,
    /// `None` for the group of what every target builds.
    target: Option<&'a String>,
    seen: BTreeSet<Pkg>,
    /// Whether a package leads to something only this target builds.
    adds: BTreeMap<Pkg, bool>,
}

impl Walk<'_> {
    fn is_common(&self, parent: &Pkg, edge: &(bool, Pkg)) -> bool {
        self.common.get(parent).is_some_and(|c| c.contains(edge))
    }

    fn adds(&mut self, p: &Pkg) -> bool {
        if let Some(&known) = self.adds.get(p) {
            return known;
        }
        // Dependency graphs have no cycles, but don't loop if one did.
        self.adds.insert(p.clone(), false);
        let edges = self.edges;
        let found = edges
            .get(p)
            .into_iter()
            .flatten()
            .any(|e| !self.is_common(p, e) || self.adds(&e.1));
        self.adds.insert(p.clone(), found);
        found
    }

    fn push(&mut self, p: &Pkg, depth: usize, build: bool, shared: bool) {
        let repeat = !self.seen.insert(p.clone());
        self.out.push(Dep {
            depth,
            name: p.0.clone(),
            version: p.1.clone(),
            source: p.2.clone(),
            repeat,
            build,
            shared,
            target: self.target.cloned(),
        });
        if repeat {
            return;
        }
        let edges = self.edges;
        for e in edges.get(p).into_iter().flatten() {
            let common = self.target.is_some() && self.is_common(p, e);
            if common && !self.adds(&e.1) {
                continue;
            }
            self.push(&e.1, depth + 1, e.0, common);
        }
    }
}

/// Whether the tool's ignore file covers `id` (or one of its aliases) today.
fn ignored(ignore: &AuditIgnore, today: &str, id: &str, aliases: &[String]) -> bool {
    ignore.ignore.iter().any(|e| {
        e.expires.as_deref().is_none_or(|exp| exp >= today)
            && (e.id == id || aliases.contains(&e.id))
    })
}

/// The report's findings by package.
fn findings(
    report: &Report,
    ignore: &AuditIgnore,
    today: &str,
) -> BTreeMap<(String, Version), Vec<Finding>> {
    let mut out: BTreeMap<(String, Version), Vec<Finding>> = BTreeMap::new();
    for v in &report.vulnerabilities {
        out.entry((v.package.name.clone(), v.package.version.clone()))
            .or_default()
            .push(Finding {
                kind: "vulnerability".into(),
                id: v.advisory.id.clone(),
                title: v.advisory.title.clone(),
                patched: v.patched.clone(),
                ignored: ignored(ignore, today, &v.advisory.id, &v.advisory.aliases),
            });
    }
    for (kind, warnings) in &report.warnings {
        for w in warnings {
            let (id, title, ign) = match &w.advisory {
                Some(a) => (
                    a.id.clone(),
                    a.title.clone(),
                    ignored(ignore, today, &a.id, &a.aliases),
                ),
                None => (
                    kind.clone(),
                    String::new(),
                    ignored(ignore, today, kind, &[]),
                ),
            };
            out.entry((w.package.name.clone(), w.package.version.clone()))
                .or_default()
                .push(Finding {
                    kind: kind.clone(),
                    id,
                    title,
                    patched: w.patched.clone(),
                    ignored: ign,
                });
        }
    }
    out
}

#[cfg(test)]
#[path = "setup_test.rs"]
pub(crate) mod tests;
