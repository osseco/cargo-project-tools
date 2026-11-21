use super::*;

use std::fs;
use std::path::Path;

use crate::audit::Auditor;
use crate::config::AuditIgnore;
use crate::lock;
use crate::source::Checkout;
use crate::source::registry::IndexEntry;
use crate::test_registry::{Crate, Registry, tool_with};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// `tool` (which has a build script) with `dep`, an optional `opt` with a build script and
/// an advisory behind its default feature `extra`, and an optional proc macro `mac` behind
/// `derive`.
struct Fixture {
    _dir: tempfile::TempDir,
    _registry: Registry,
    setup: Setup,
}

fn fixture(default_features: bool) -> Fixture {
    let registry = Registry::with(vec![
        Crate::new("dep", "1.0.0"),
        Crate::new("opt", "0.1.0").build("fn main() {}\n"),
        Crate::new("mac", "1.0.0").proc_macro(),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let manifest = tool_with(
        &src,
        "1.0.0",
        "[dependencies]\ndep = \"1\"\nopt = { version = \"0.1\", optional = true }\nmac = { version = \"1\", optional = true }\n\n[features]\ndefault = [\"extra\"]\nextra = [\"dep:opt\"]\nderive = [\"dep:mac\"]\n",
    );
    lock::shipped_or_generated(&manifest, &src.join("Cargo.lock"), &registry.cargo_args()).unwrap();
    let db = dir.path().join("db");
    write(
        &db.join("crates/opt/RUSTSEC-2024-0002.md"),
        "```toml\n[advisory]\nid = \"RUSTSEC-2024-0002\"\npackage = \"opt\"\ndate = \"2024-01-01\"\n\n[versions]\npatched = [\">= 0.2.0\"]\n```\n\n# Opt is broken\n\nDescription.\n",
    );
    let mut auditor = Auditor::new(rustsec::Database::open(&db).unwrap());
    for (name, version) in [("dep", "1.0.0"), ("opt", "0.1.0"), ("mac", "1.0.0")] {
        auditor.index.insert(
            name,
            vec![IndexEntry {
                name: name.into(),
                vers: Version::parse(version).unwrap(),
                cksum: String::new(),
                yanked: false,
            }],
        );
    }
    auditor.index.insert("tool", Vec::new());
    let setup = Setup::new(
        "tool",
        Checkout::at(src.clone(), src.clone(), src),
        false,
        BTreeSet::new(),
        default_features,
        None,
        auditor,
        AuditIgnore::default(),
        registry.cargo_args(),
    )
    .unwrap();
    Fixture {
        _dir: dir,
        _registry: registry,
        setup,
    }
}

fn names(packages: &[Package]) -> Vec<&str> {
    packages.iter().map(|p| p.name.as_str()).collect()
}

fn code(build_time: &[BuildTime]) -> Vec<String> {
    build_time
        .iter()
        .map(|b| format!("{} {} {}", b.kind, b.package, b.recorded))
        .collect()
}

#[test]
fn each_feature_shows_what_it_adds() {
    let mut f = fixture(true);
    let recorded = BTreeSet::from([("build-script".to_string(), "tool".to_string())]);
    let found = inspect(&mut f.setup, &recorded).unwrap();

    assert_eq!(names(&found.packages), ["dep", "opt", "tool"]);
    assert_eq!(
        code(&found.build_time),
        ["build-script opt false", "build-script tool true"]
    );
    assert_eq!(found.findings.len(), 1);
    assert_eq!(found.findings[0].id, "RUSTSEC-2024-0002");

    // `extra` is on through the defaults, so only `derive` and the defaults are tried.
    let changes: Vec<(&str, bool)> = found
        .feature_changes
        .iter()
        .map(|c| (c.feature.as_str(), c.enable))
        .collect();
    assert_eq!(changes, [("derive", true), ("default", false)]);

    let derive = &found.feature_changes[0];
    assert_eq!(names(&derive.packages), ["mac"]);
    assert_eq!(code(&derive.build_time), ["proc-macro mac false"]);
    assert!(derive.findings.is_empty());

    let no_defaults = &found.feature_changes[1];
    assert_eq!(names(&no_defaults.packages), ["opt"]);
    assert_eq!(code(&no_defaults.build_time), ["build-script opt false"]);
    assert_eq!(no_defaults.findings[0].package, "opt");

    // The setup is back where it started.
    assert!(f.setup.default_features);
    assert!(f.setup.tree.iter().any(|d| d.name == "opt"));
}

#[test]
fn without_defaults_turning_them_on_is_tried() {
    let mut f = fixture(false);
    let found = inspect(&mut f.setup, &BTreeSet::new()).unwrap();
    assert_eq!(names(&found.packages), ["dep", "tool"]);
    let changes: Vec<(&str, bool, Vec<&str>)> = found
        .feature_changes
        .iter()
        .map(|c| (c.feature.as_str(), c.enable, names(&c.packages)))
        .collect();
    assert_eq!(
        changes,
        [
            ("default", true, vec!["opt"]),
            ("derive", true, vec!["mac"]),
            ("extra", true, vec!["opt"]),
        ]
    );
}

#[test]
fn the_record_is_tagged_inspect() {
    let mut f = fixture(true);
    let found = inspect(&mut f.setup, &BTreeSet::new()).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&crate::output::line(&crate::output::Record::Inspect(found))).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["kind"], "inspect");
    assert_eq!(json["tool"], "tool");
    assert_eq!(json["tool_version"], "1.0.0");
    assert_eq!(json["features"]["derive"][0], "dep:mac");
    assert_eq!(json["feature_changes"][0]["packages"][0]["name"], "mac");
}
