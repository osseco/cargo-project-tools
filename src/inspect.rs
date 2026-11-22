//! `inspect`: what a tool builds with its features, what each other feature would add, and
//! the advisories and build-time code that come with it. The setup screen's model, without
//! the screen, for scripts and agents choosing features.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;
use semver::Version;
use serde::Serialize;

use crate::audit;
use crate::output::say;
use crate::sanitize::shown_line;
use crate::setup::Setup;

#[derive(Debug, Serialize, PartialEq)]
pub struct Inspection {
    pub tool: String,
    pub tool_version: String,
    /// The tool's features and what each enables.
    pub features: BTreeMap<String, Vec<String>>,
    pub default_features: bool,
    /// Features turned on besides the defaults.
    pub selected: Vec<String>,
    /// The targets the tree is for; all when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<String>>,
    /// The tool's source, repo-relative: its manifest and README say what features are for.
    pub source: String,
    /// The tool's binaries, the features each needs, and whether these features build it.
    pub bins: Vec<Bin>,
    /// What the tool builds with these features.
    pub packages: Vec<Package>,
    pub findings: Vec<Finding>,
    pub build_time: Vec<BuildTime>,
    /// What turning on each other feature adds, and turning off the defaults removes.
    pub feature_changes: Vec<Change>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Bin {
    pub name: String,
    /// Its `required-features`.
    pub required_features: Vec<String>,
    /// Built with the chosen features.
    pub built: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq, PartialOrd, Ord, Clone)]
pub struct Package {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, PartialEq, Clone)]
pub struct Finding {
    pub package: String,
    pub version: String,
    /// `vulnerability`, `unmaintained`, `unsound`, `notice`, `yanked` or `git-source`.
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub title: String,
    pub patched: Vec<String>,
    /// In the tool's `audit-ignore.toml`.
    pub ignored: bool,
    /// The command to run, as `audit` suggests it, for an open vulnerability or a yanked
    /// version.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// The platforms and functions the advisory is limited to, when the database says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected: Option<crate::output::Affected>,
}

#[derive(Debug, Serialize, PartialEq, Clone)]
pub struct BuildTime {
    /// `build-script`, `proc-macro`, or `build-dep` for a library build-time code runs.
    #[serde(rename = "type")]
    pub kind: String,
    pub package: String,
    pub version: String,
    /// The tool's `build-review.toml` has an entry for it (it may still be out of date).
    pub recorded: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Change {
    pub feature: String,
    /// `true`: what turning `feature` on adds. `false` (only for `default`): what
    /// `default-features = false` removes.
    pub enable: bool,
    pub packages: Vec<Package>,
    pub findings: Vec<Finding>,
    pub build_time: Vec<BuildTime>,
    /// The binaries it builds that weren't built before, or (for `enable: false`) that are
    /// no longer built.
    pub bins: Vec<String>,
}

/// What the current features build.
struct State {
    packages: BTreeSet<(String, Version)>,
    findings: Vec<Finding>,
    build_time: Vec<BuildTime>,
    bins: BTreeSet<String>,
}

fn state(
    setup: &mut Setup,
    code: &BTreeSet<(String, String, Version)>,
    recorded: &BTreeSet<(String, String)>,
) -> State {
    let packages: BTreeSet<(String, Version)> = setup.built.clone();
    let mut findings = Vec::new();
    for ((name, version), list) in setup.findings.clone() {
        // One command for every open vulnerability of a package, as `audit` gives it.
        let open: Vec<_> = list
            .iter()
            .filter(|f| f.is_vulnerability() && !f.ignored)
            .map(|f| f.patched.clone())
            .collect();
        let vuln_fix = (!open.is_empty()).then(|| setup.fix(&name, &version, &open));
        for f in list {
            let fix = match f.kind.as_str() {
                _ if f.ignored => None,
                "vulnerability" => vuln_fix.clone(),
                "yanked" => Some(audit::yanked_fix(&setup.name, &name, &version)),
                _ => None,
            };
            findings.push(Finding {
                package: name.clone(),
                version: version.to_string(),
                kind: f.kind,
                id: f.id,
                title: f.title,
                patched: f.patched.iter().map(ToString::to_string).collect(),
                ignored: f.ignored,
                fix,
                affected: f.affected,
            });
        }
    }
    let build_time = code
        .iter()
        .filter(|(_, name, version)| packages.contains(&(name.clone(), version.clone())))
        .map(|(kind, name, version)| BuildTime {
            kind: kind.clone(),
            package: name.clone(),
            version: version.to_string(),
            recorded: recorded.contains(&(kind.clone(), name.clone())),
        })
        .collect();
    State {
        packages,
        findings,
        build_time,
        bins: built_bins(setup),
    }
}

/// The binaries the setup's features build: those whose `required-features` are all on. A
/// required `dep/feature` counts by its first part.
fn built_bins(setup: &Setup) -> BTreeSet<String> {
    let start = setup
        .selected
        .iter()
        .map(String::as_str)
        .chain(setup.default_features.then_some("default"));
    let on = implied(&setup.features, start);
    setup
        .bins
        .iter()
        .filter(|(_, required)| {
            required
                .iter()
                .all(|r| on.contains(r.split('/').next().unwrap_or(r)))
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// What `to` has that `from` doesn't.
fn change(feature: &str, enable: bool, from: &State, to: &State) -> Change {
    let new = |name: &str, version: &str| {
        !from
            .packages
            .iter()
            .any(|(n, v)| n == name && v.to_string() == version)
    };
    Change {
        feature: feature.to_string(),
        enable,
        packages: to
            .packages
            .difference(&from.packages)
            .map(|(name, version)| Package {
                name: name.clone(),
                version: version.to_string(),
            })
            .collect(),
        findings: to
            .findings
            .iter()
            .filter(|f| new(&f.package, &f.version))
            .cloned()
            .collect(),
        build_time: to
            .build_time
            .iter()
            .filter(|b| new(&b.package, &b.version))
            .cloned()
            .collect(),
        bins: to.bins.difference(&from.bins).cloned().collect(),
    }
}

/// The features `start` turns on, itself included, following what each enables. Only a
/// value that names a feature counts: `dep:x` and `x?/y` turn on no feature of the tool,
/// `x/y` turns on feature `x` when there is one.
fn implied<'a>(
    features: &BTreeMap<String, Vec<String>>,
    start: impl Iterator<Item = &'a str>,
) -> BTreeSet<String> {
    let mut on = BTreeSet::new();
    let mut todo: Vec<String> = start.map(str::to_string).collect();
    while let Some(f) = todo.pop() {
        if !features.contains_key(&f) || !on.insert(f.clone()) {
            continue;
        }
        for value in &features[&f] {
            let name = match value.split_once('/') {
                _ if value.starts_with("dep:") => continue,
                Some((dep, _)) if dep.ends_with('?') => continue,
                Some((dep, _)) => dep,
                None => value.as_str(),
            };
            todo.push(name.to_string());
        }
    }
    on
}

/// Inspect the setup's current features, then try each other feature, and the defaults
/// off, against them. The setup ends with the features it started with. `recorded`: the
/// kinds and packages in the tool's `build-review.toml`.
pub fn inspect(
    setup: &mut Setup,
    recorded: &BTreeSet<(String, String)>,
    root: &Path,
) -> Result<Inspection> {
    let code = setup.build_time()?;
    let selected = setup.selected.clone();
    let defaults = setup.default_features;
    let base = state(setup, &code, recorded);

    // A feature the selection already turns on, directly or through others, adds nothing:
    // don't spend a `cargo tree` on it.
    let start = selected
        .iter()
        .map(String::as_str)
        .chain(defaults.then_some("default"));
    let on = implied(&setup.features, start);
    let candidates: Vec<String> = setup
        .features
        .keys()
        .filter(|f| *f != "default" && !on.contains(*f))
        .cloned()
        .collect();
    let mut changes = Vec::new();
    if !defaults && setup.features.contains_key("default") {
        setup.set_features(selected.clone(), true)?;
        changes.push(change(
            "default",
            true,
            &base,
            &state(setup, &code, recorded),
        ));
    }
    for feature in &candidates {
        let mut with = selected.clone();
        with.insert(feature.clone());
        setup.set_features(with, defaults)?;
        changes.push(change(feature, true, &base, &state(setup, &code, recorded)));
    }
    if defaults && setup.features.contains_key("default") {
        setup.set_features(selected.clone(), false)?;
        changes.push(change(
            "default",
            false,
            &state(setup, &code, recorded),
            &base,
        ));
    }
    setup.set_features(selected.clone(), defaults)?;

    Ok(Inspection {
        tool: setup.name.clone(),
        tool_version: setup.checkout.version.clone(),
        features: setup.features.clone(),
        default_features: defaults,
        selected: selected.into_iter().collect(),
        targets: setup.targets.clone(),
        source: crate::review::slash_rel(&setup.checkout.package_dir, root),
        bins: setup
            .bins
            .iter()
            .map(|(name, required)| Bin {
                name: name.clone(),
                required_features: required.clone(),
                built: base.bins.contains(name),
            })
            .collect(),
        packages: base
            .packages
            .iter()
            .map(|(name, version)| Package {
                name: name.clone(),
                version: version.to_string(),
            })
            .collect(),
        findings: base.findings,
        build_time: base.build_time,
        feature_changes: changes,
    })
}

fn count(build_time: &[BuildTime], kind: &str) -> usize {
    build_time.iter().filter(|b| b.kind == kind).count()
}

fn finding_line(f: &Finding) -> String {
    let ignored = if f.ignored { ", ignored" } else { "" };
    format!(
        "{} {}@{} ({}{ignored}): {}",
        f.id, f.package, f.version, f.kind, f.title
    )
}

/// `say!` with the line made safe to show: feature names, package names and advisory
/// titles come from manifests, locks and the advisory database.
macro_rules! safe {
    ($($arg:tt)*) => {
        say!("{}", shown_line(&format!($($arg)*)))
    };
}

/// The human report.
pub fn print(i: &Inspection) {
    let defaults = if i.default_features { "on" } else { "off" };
    safe!("{} {}", i.tool, i.tool_version);
    safe!(
        "  default features {defaults}; selected: {}",
        if i.selected.is_empty() {
            "none".to_string()
        } else {
            i.selected.join(", ")
        }
    );
    if let Some(t) = &i.targets {
        safe!("  targets: {}", t.join(", "));
    }
    safe!("  source: {}", i.source);
    safe!("features:");
    for (name, enables) in &i.features {
        safe!("  {name} = [{}]", enables.join(", "));
    }
    safe!("binaries:");
    for b in &i.bins {
        let needs = if b.required_features.is_empty() {
            String::new()
        } else {
            format!(" (needs {})", b.required_features.join(", "))
        };
        let state = if b.built { "built" } else { "not built" };
        safe!("  {}: {state}{needs}", b.name);
    }
    let open = |kind: bool| {
        i.findings
            .iter()
            .filter(|f| !f.ignored && (f.kind == "vulnerability") == kind)
            .count()
    };
    let unrecorded = i.build_time.iter().filter(|b| !b.recorded).count();
    safe!(
        "builds {} packages: {} vulnerabilities, {} warnings, {} build scripts, {} proc macros, {} build-time libraries ({unrecorded} not in build-review.toml)",
        i.packages.len(),
        open(true),
        open(false),
        count(&i.build_time, "build-script"),
        count(&i.build_time, "proc-macro"),
        count(&i.build_time, "build-dep"),
    );
    if !i.findings.is_empty() {
        safe!("advisories:");
        for f in &i.findings {
            safe!("  {}", finding_line(f));
            if let Some(fix) = &f.fix {
                safe!("    fix: {fix}");
            }
        }
    }
    if !i.build_time.is_empty() {
        safe!("build-time code:");
        for b in &i.build_time {
            let note = if b.recorded { "" } else { "  (not recorded)" };
            safe!("  {:<12} {}@{}{note}", b.kind, b.package, b.version);
        }
    }
    if !i.feature_changes.is_empty() {
        safe!("feature changes:");
    }
    for c in &i.feature_changes {
        let (sign, verb) = if c.enable {
            ("+", "adds")
        } else {
            ("-", "removes")
        };
        if c.packages.is_empty() && c.bins.is_empty() {
            safe!("  {sign}{}: {verb} nothing", c.feature);
            continue;
        }
        safe!(
            "  {sign}{}: {verb} {} packages, {} build scripts, {} proc macros, {} build-time libraries, {} advisories",
            c.feature,
            c.packages.len(),
            count(&c.build_time, "build-script"),
            count(&c.build_time, "proc-macro"),
            count(&c.build_time, "build-dep"),
            c.findings.len()
        );
        let names: Vec<String> = c
            .packages
            .iter()
            .map(|p| format!("{}@{}", p.name, p.version))
            .collect();
        safe!("      {}", names.join(" "));
        if !c.bins.is_empty() {
            safe!("      binaries: {}", c.bins.join(" "));
        }
        for f in &c.findings {
            safe!("      {}", finding_line(f));
        }
    }
}

#[cfg(test)]
#[path = "inspect_test.rs"]
mod tests;
