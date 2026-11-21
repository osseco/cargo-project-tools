use super::*;

use std::path::Path;

use crate::config::IgnoreEntry;
use crate::source::registry::IndexEntry;
use crate::test_registry::{Registry, tool_with};

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

fn entry(name: &str, version: &str, yanked: bool) -> IndexEntry {
    IndexEntry {
        name: name.into(),
        vers: v(version),
        cksum: String::new(),
        yanked,
    }
}

/// `tool` with an optional `opt` behind its default feature `extra`, and `dep` = "1".
/// `dep` 1.0.0 has an advisory patched in >= 1.1.0; the lock starts on 1.0.0.
struct Fixture {
    _dir: tempfile::TempDir,
    _registry: Registry,
    setup: Setup,
}

fn fixture(ignore: AuditIgnore) -> Fixture {
    let registry = Registry::new(&[
        ("dep", "1.0.0"),
        ("dep", "1.1.0"),
        ("dep", "1.2.0"),
        ("dep", "2.0.0"),
        ("opt", "0.1.0"),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let manifest = tool_with(
        &src,
        "1.0.0",
        "[dependencies]\ndep = \"1\"\nopt = { version = \"0.1\", optional = true }\n\n[features]\ndefault = [\"extra\"]\nextra = [\"dep:opt\"]\nunused = []\n",
    );
    // Locked against a registry that only has dep 1.0.0; the setup sees them all.
    let first = Registry::new(&[("dep", "1.0.0"), ("opt", "0.1.0")]);
    let lock_path = src.join("Cargo.lock");
    lock::shipped_or_generated(&manifest, &lock_path, &first.cargo_args()).unwrap();

    let db = dir.path().join("db");
    write(
        &db.join("crates/dep/RUSTSEC-2024-0001.md"),
        "```toml\n[advisory]\nid = \"RUSTSEC-2024-0001\"\npackage = \"dep\"\ndate = \"2024-01-01\"\n\n[versions]\npatched = [\">= 1.1.0\"]\n```\n\n# Dep is broken\n\nDescription.\n",
    );
    let mut auditor = Auditor::new(rustsec::Database::open(&db).unwrap());
    auditor.index.insert(
        "dep",
        vec![
            entry("dep", "1.0.0", false),
            entry("dep", "1.1.0", false),
            entry("dep", "1.2.0", true),
            entry("dep", "2.0.0", false),
        ],
    );
    auditor
        .index
        .insert("opt", vec![entry("opt", "0.1.0", false)]);
    auditor.index.insert("tool", Vec::new());

    let checkout = Checkout::at(src.clone(), src.clone(), src);
    let setup = Setup::new(
        "tool",
        checkout,
        false,
        BTreeSet::new(),
        true,
        None,
        auditor,
        ignore,
        registry.cargo_args(),
    )
    .unwrap();
    Fixture {
        _dir: dir,
        _registry: registry,
        setup,
    }
}

fn names(setup: &Setup) -> Vec<&str> {
    setup.tree.iter().map(|d| d.name.as_str()).collect()
}

#[test]
fn features_come_from_the_manifest() {
    let f = fixture(AuditIgnore::default());
    let features: Vec<&str> = f.setup.features.keys().map(String::as_str).collect();
    assert_eq!(features, ["default", "extra", "unused"]);
    assert_eq!(f.setup.features["extra"], ["dep:opt"]);
}

#[test]
fn features_change_the_tree() {
    let mut f = fixture(AuditIgnore::default());
    assert_eq!(names(&f.setup), ["tool", "dep", "opt"]);
    assert_eq!(f.setup.tree[1].depth, 1);
    f.setup.set_features(BTreeSet::new(), false).unwrap();
    assert_eq!(names(&f.setup), ["tool", "dep"]);
    assert_eq!(f.setup.feature_args(), ["--no-default-features"]);
    f.setup
        .set_features(BTreeSet::from(["extra".to_string()]), false)
        .unwrap();
    assert_eq!(names(&f.setup), ["tool", "dep", "opt"]);
}

#[test]
fn why_names_the_feature() {
    let f = fixture(AuditIgnore::default());
    let why = f.setup.why("opt", &v("0.1.0")).unwrap().join("\n");
    assert!(why.contains("tool feature \"extra\""), "{why}");
    assert!(why.contains("tool feature \"default\""), "{why}");
}

#[test]
fn advisories_are_found_and_ignores_respected() {
    let f = fixture(AuditIgnore::default());
    let found = &f.setup.findings[&("dep".to_string(), v("1.0.0"))];
    assert_eq!(found.len(), 1);
    assert!(found[0].is_vulnerability());
    assert_eq!(found[0].id, "RUSTSEC-2024-0001");
    assert_eq!(found[0].title, "Dep is broken");
    assert!(!found[0].ignored);
    assert_eq!(f.setup.counts(), (3, 1, 0));

    let f = fixture(AuditIgnore {
        ignore: vec![IgnoreEntry {
            id: "RUSTSEC-2024-0001".into(),
            reason: "not reachable".into(),
            expires: None,
        }],
    });
    assert!(f.setup.findings[&("dep".to_string(), v("1.0.0"))][0].ignored);
    assert_eq!(f.setup.counts(), (3, 0, 0));
}

#[test]
fn versions_are_newest_first_with_the_fix_suggested() {
    let mut f = fixture(AuditIgnore::default());
    let choices = f.setup.versions("dep", &v("1.0.0"));
    let summary: Vec<(String, bool, bool, bool)> = choices
        .iter()
        .map(|c| (c.version.to_string(), c.compatible, c.suggested, c.current))
        .collect();
    // 1.2.0 is yanked.
    assert_eq!(
        summary,
        [
            ("2.0.0".into(), false, false, false),
            ("1.1.0".into(), true, true, false),
            ("1.0.0".into(), true, false, true),
        ]
    );
}

#[test]
fn a_move_cargo_does_not_make_leaves_the_lock() {
    // Offline, cargo ignores `--precise` for the replaced test registry and still succeeds:
    // that must not count as a move. (A real move needs crates.io.)
    let mut f = fixture(AuditIgnore::default());
    let before = fs::read_to_string(f.setup.lock_path()).unwrap();
    let err = f
        .setup
        .set_version("dep", &v("1.0.0"), &v("1.1.0"))
        .unwrap_err()
        .to_string();
    assert_eq!(err, "cargo didn't move dep to 1.1.0");
    assert_eq!(fs::read_to_string(f.setup.lock_path()).unwrap(), before);
    assert_eq!(f.setup.counts(), (3, 1, 0));
}

#[test]
fn a_move_cargo_refuses_is_its_message() {
    let mut f = fixture(AuditIgnore::default());
    let before = fs::read_to_string(f.setup.lock_path()).unwrap();
    let err = f
        .setup
        .set_version("nonesuch", &v("1.0.0"), &v("1.1.0"))
        .unwrap_err()
        .to_string();
    assert!(err.starts_with("error"), "{err}");
    assert!(err.contains("nonesuch"), "{err}");
    assert!(!err.contains('\n'));
    assert_eq!(fs::read_to_string(f.setup.lock_path()).unwrap(), before);
}

#[test]
fn cargo_errors_on_one_line() {
    assert_eq!(
        cargo_error("    Updating index\nerror: no matching package\n\n  dep@1.2.0\n"),
        "error: no matching package dep@1.2.0"
    );
    assert_eq!(cargo_error("odd failure\n"), "odd failure");
    assert_eq!(cargo_error(""), "cargo failed");
}

#[test]
fn tree_lines_parse() {
    let deps = parse_tree(
        "0tool v1.0.0 (/src)\n1dep v1.0.0\n2inner v0.1.0 (proc-macro)\n1dep v1.0.0 (*)\n10deep v1.0.0-rc.1\n",
    )
    .unwrap();
    assert_eq!(deps.len(), 5);
    assert_eq!(deps[0].source, "/src");
    assert_eq!(deps[2].source, "proc-macro");
    assert!(deps[3].repeat);
    assert!(!deps[1].repeat);
    assert_eq!(deps[4].depth, 10);
    assert_eq!(deps[4].version, v("1.0.0-rc.1"));
    assert!(parse_tree("").is_err());
    assert!(parse_tree("xtool v1\n").is_err());
}

#[test]
fn another_checkout_keeps_the_features_it_still_has() {
    let mut f = fixture(AuditIgnore::default());
    f.setup
        .set_features(
            BTreeSet::from(["extra".to_string(), "unused".to_string()]),
            true,
        )
        .unwrap();
    // Version 1.1.0 drops `unused` and `opt`.
    let registry = Registry::new(&[("dep", "1.1.0")]);
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    let manifest = tool_with(
        &src,
        "1.1.0",
        "[dependencies]\ndep = \"1\"\n\n[features]\ndefault = [\"extra\"]\nextra = []\n",
    );
    lock::shipped_or_generated(&manifest, &src.join("Cargo.lock"), &registry.cargo_args()).unwrap();
    f.setup.cargo_args = registry.cargo_args();
    f.setup
        .auditor
        .index
        .insert("dep", vec![entry("dep", "1.1.0", false)]);
    f.setup
        .set_checkout(Checkout::at(src.clone(), src.clone(), src))
        .unwrap();
    assert_eq!(f.setup.selected, BTreeSet::from(["extra".to_string()]));
    assert_eq!(names(&f.setup), ["tool", "dep"]);
    assert_eq!(f.setup.counts(), (2, 0, 0));
}
