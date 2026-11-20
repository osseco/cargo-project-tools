use std::fs;

use super::*;

const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

/// An advisory file as found in `advisory-db/crates/<package>/`.
fn advisory(id: &str, package: &str, extra: &str, patched: &str) -> String {
    format!(
        "```toml\n[advisory]\nid = \"{id}\"\npackage = \"{package}\"\ndate = \"2020-01-01\"\n{extra}\n\n[versions]\npatched = [{patched}]\n```\n\n# Title of {id}\n\nDescription.\n"
    )
}

fn db(advisories: &[(&str, &str, &str, &str)]) -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    for (id, package, extra, patched) in advisories {
        let pkg_dir = dir.path().join("crates").join(package);
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join(format!("{id}.md")),
            advisory(id, package, extra, patched),
        )
        .unwrap();
    }
    let db = Database::open(dir.path()).unwrap();
    (dir, db)
}

fn lock() -> Lockfile {
    let text = format!(
        r#"version = 3

[[package]]
name = "mytool"
version = "1.0.0"

[[package]]
name = "smallvec"
version = "1.6.0"
source = "{REGISTRY}"

[[package]]
name = "ansi_term"
version = "0.12.1"
source = "{REGISTRY}"

[[package]]
name = "gitdep"
version = "1.0.0"
source = "git+https://github.com/example/gitdep?rev=abcdef0#abcdef0123456789abcdef0123456789abcdef01"

[[package]]
name = "localdep"
version = "1.0.0"
"#
    );
    crate::lock::parse(&text).unwrap()
}

fn fixture() -> (tempfile::TempDir, Database) {
    db(&[
        ("RUSTSEC-2020-0001", "mytool", "", "\">= 1.0.1\""),
        (
            "RUSTSEC-2020-0002",
            "smallvec",
            "aliases = [\"CVE-2021-25900\"]",
            "\">= 1.6.1\"",
        ),
        (
            "RUSTSEC-2020-0003",
            "ansi_term",
            "informational = \"unmaintained\"",
            "",
        ),
        ("RUSTSEC-2020-0004", "gitdep", "", "\">= 2.0.0\""),
        (
            "RUSTSEC-2020-0005",
            "smallvec",
            "withdrawn = \"2021-01-01\"",
            "\">= 9.0.0\"",
        ),
        ("RUSTSEC-2020-0006", "localdep", "", "\">= 2.0.0\""),
    ])
}

fn ids<'a>(list: impl IntoIterator<Item = &'a Option<Advisory>>) -> Vec<&'a str> {
    list.into_iter()
        .map(|a| a.as_ref().unwrap().id.as_str())
        .collect()
}

#[test]
fn scan_registry_tool() {
    let (_dir, db) = fixture();
    let r = scan(&db, &lock(), "mytool", false);
    let vulns: Vec<_> = r
        .vulnerabilities
        .iter()
        .map(|v| (v.advisory.id.as_str(), v.package.name.as_str()))
        .collect();
    // The root package is checked too; the withdrawn advisory and the path package are not.
    assert_eq!(
        vulns,
        vec![
            ("RUSTSEC-2020-0001", "mytool"),
            ("RUSTSEC-2020-0002", "smallvec")
        ]
    );
    assert_eq!(
        r.vulnerabilities[1].advisory.aliases,
        vec!["CVE-2021-25900".to_string()]
    );
    assert_eq!(
        r.vulnerabilities[1].patched,
        vec![VersionReq::parse(">=1.6.1").unwrap()]
    );
    assert_eq!(
        ids(r.warnings["unmaintained"].iter().map(|w| &w.advisory)),
        vec!["RUSTSEC-2020-0003"]
    );
    assert_eq!(
        ids(r.warnings[GIT_SOURCE].iter().map(|w| &w.advisory)),
        vec!["RUSTSEC-2020-0004"]
    );
}

#[test]
fn scan_git_tool_root_is_a_warning() {
    let (_dir, db) = fixture();
    let r = scan(&db, &lock(), "mytool", true);
    assert_eq!(r.vulnerabilities.len(), 1);
    assert_eq!(
        ids(r.warnings[GIT_SOURCE].iter().map(|w| &w.advisory)),
        vec!["RUSTSEC-2020-0001", "RUSTSEC-2020-0004"]
    );
}

#[test]
fn root_fix_is_an_upgrade() {
    let (_dir, db) = fixture();
    let r = scan(&db, &lock(), "mytool", false);
    let v = &r.vulnerabilities[0];
    let fix = suggestion(
        "mytool",
        &v.package.name,
        &v.package.version,
        &[v],
        &mut IndexCache::default(),
    );
    assert_eq!(fix, format!("cargo {SUBCOMMAND} upgrade mytool"));
}

fn ignores(toml_text: &str) -> AuditIgnore {
    toml::from_str(toml_text).unwrap()
}

#[test]
fn ignore_by_alias_and_unused() {
    let (_dir, db) = fixture();
    let r = scan(&db, &lock(), "mytool", false);
    let file = ignores(
        r#"
        [[ignore]]
        id = "CVE-2021-25900"
        reason = "not used"
        [[ignore]]
        id = "RUSTSEC-2020-0003"
        reason = "fine"
        [[ignore]]
        id = "RUSTSEC-1999-0001"
        reason = "stale"
        "#,
    );
    let mut ig = Ignores::new(&file, "2026-01-01", "t");
    assert!(ig.matches(&r.vulnerabilities[1].advisory).is_some());
    assert!(
        ig.matches(r.warnings["unmaintained"][0].advisory.as_ref().unwrap())
            .is_some()
    );
    let unused: Vec<_> = ig.unused().map(|e| e.id.as_str()).collect();
    assert_eq!(unused, vec!["RUSTSEC-1999-0001"]);
}

#[test]
fn expired_ignore_does_not_apply() {
    let (_dir, db) = fixture();
    let r = scan(&db, &lock(), "mytool", false);
    let file = ignores(
        "[[ignore]]\nid = \"RUSTSEC-2020-0002\"\nreason = \"x\"\nexpires = \"2025-01-01\"\n",
    );
    let mut ig = Ignores::new(&file, "2026-01-01", "t");
    assert!(ig.matches(&r.vulnerabilities[1].advisory).is_none());
}

#[test]
fn civil_dates() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    assert_eq!(civil_from_days(19_782), (2024, 2, 29));
}

fn entry(name: &str, vers: &str, yanked: bool) -> registry::IndexEntry {
    registry::IndexEntry {
        name: name.into(),
        vers: vers.parse().unwrap(),
        cksum: String::new(),
        yanked,
    }
}

fn found(r: &Report) -> Vec<String> {
    let mut out: Vec<String> = r
        .vulnerabilities
        .iter()
        .map(|v| format!("{} {}@{}", v.advisory.id, v.package.name, v.package.version))
        .collect();
    for (kind, list) in &r.warnings {
        for w in list {
            let id = w.advisory.as_ref().map_or("-", |a| a.id.as_str());
            out.push(format!(
                "{kind} {id} {}@{}",
                w.package.name, w.package.version
            ));
        }
    }
    out
}

fn lock_of(packages: &[(&str, &str, Option<&str>)]) -> Lockfile {
    let mut text = "version = 3\n".to_string();
    for (name, version, source) in packages {
        text.push_str(&format!(
            "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"
        ));
        if let Some(source) = source {
            text.push_str(&format!("source = \"{source}\"\n"));
        }
    }
    crate::lock::parse(&text).unwrap()
}

#[test]
fn version_ranges_follow_patched_and_unaffected() {
    let (_dir, db) = db(&[(
        "RUSTSEC-2021-0003",
        "smallvec",
        "",
        "\">= 0.6.14, < 1.0.0\", \">= 1.6.1\"]\nunaffected = [\"< 0.6.3\"",
    )]);
    let lock = lock_of(&[
        ("root", "1.0.0", None),
        ("smallvec", "0.6.2", Some(REGISTRY)),
        ("smallvec", "0.6.13", Some(REGISTRY)),
        ("smallvec", "0.6.14", Some(REGISTRY)),
        ("smallvec", "1.6.0", Some(REGISTRY)),
        ("smallvec", "1.6.1", Some(REGISTRY)),
    ]);
    assert_eq!(
        found(&scan(&db, &lock, "root", false)),
        vec![
            "RUSTSEC-2021-0003 smallvec@0.6.13",
            "RUSTSEC-2021-0003 smallvec@1.6.0"
        ]
    );
}

#[test]
fn informational_kinds() {
    let (_dir, db) = db(&[
        ("RUSTSEC-2020-0010", "a", "informational = \"notice\"", ""),
        ("RUSTSEC-2020-0011", "b", "informational = \"unsound\"", ""),
        // Kinds rustsec has no warning for are skipped, as in cargo-audit.
        (
            "RUSTSEC-2020-0012",
            "c",
            "informational = \"something-new\"",
            "",
        ),
    ]);
    let lock = lock_of(&[
        ("root", "1.0.0", None),
        ("a", "1.0.0", Some(REGISTRY)),
        ("b", "1.0.0", Some(REGISTRY)),
        ("c", "1.0.0", Some(REGISTRY)),
    ]);
    assert_eq!(
        found(&scan(&db, &lock, "root", false)),
        vec![
            "notice RUSTSEC-2020-0010 a@1.0.0",
            "unsound RUSTSEC-2020-0011 b@1.0.0"
        ]
    );
}

#[test]
fn sources_decide_what_is_checked() {
    let (_dir, db) = db(&[
        ("RUSTSEC-2020-0020", "sparse", "", "\">= 2.0.0\""),
        ("RUSTSEC-2020-0021", "private", "", "\">= 2.0.0\""),
        ("RUSTSEC-2020-0022", "local", "", "\">= 2.0.0\""),
    ]);
    let lock = lock_of(&[
        ("root", "1.0.0", None),
        // crates.io over the sparse protocol is still crates.io.
        ("sparse", "1.0.0", Some("sparse+https://index.crates.io/")),
        (
            "private",
            "1.0.0",
            Some("registry+https://example.com/index"),
        ),
        ("local", "1.0.0", None),
    ]);
    assert_eq!(
        found(&scan(&db, &lock, "root", false)),
        vec!["RUSTSEC-2020-0020 sparse@1.0.0"]
    );
}

#[test]
fn self_check_reports_vulnerabilities_only() {
    let (_dir, db) = db(&[
        (
            "RUSTSEC-2020-0030",
            env!("CARGO_PKG_NAME"),
            "",
            "\">= 999.0.0\"",
        ),
        (
            "RUSTSEC-2020-0031",
            "rustsec",
            "informational = \"unmaintained\"",
            "",
        ),
    ]);
    let found = self_check(&db);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].0.name, env!("CARGO_PKG_NAME"));
    assert_eq!(found[0].0.version.to_string(), env!("CARGO_PKG_VERSION"));
    assert_eq!(found[0].1.id, "RUSTSEC-2020-0030");
}

#[test]
fn self_findings_can_be_ignored_until_they_expire() {
    let (_dir, db) = db(&[(
        "RUSTSEC-2020-0030",
        env!("CARGO_PKG_NAME"),
        "aliases = [\"CVE-2020-0030\"]",
        "\">= 999.0.0\"",
    )]);
    let found = self_check(&db);
    let today = "2026-01-01";
    assert_eq!(evaluate_self(&found, &AuditIgnore::default(), today), 1);
    let by_id = ignores(
        "[[ignore]]\nid = \"RUSTSEC-2020-0030\"\nreason = \"x\"\nexpires = \"2026-02-01\"\n",
    );
    assert_eq!(evaluate_self(&found, &by_id, today), 0);
    let by_alias =
        ignores("[[ignore]]\nid = \"CVE-2020-0030\"\nreason = \"x\"\nexpires = \"2026-02-01\"\n");
    assert_eq!(evaluate_self(&found, &by_alias, today), 0);
    let expired = ignores(
        "[[ignore]]\nid = \"RUSTSEC-2020-0030\"\nreason = \"x\"\nexpires = \"2025-12-31\"\n",
    );
    assert_eq!(evaluate_self(&found, &expired, today), 1);
}

/// Every package of the lock, as if all of it were built.
fn all_of(lock_text: &str) -> Built {
    crate::lock::parse(lock_text)
        .unwrap()
        .packages
        .iter()
        .map(|p| (p.name.as_str().to_string(), p.version.clone()))
        .collect()
}

/// The full `Auditor::audit` path with a lock on disk and a seeded index.
fn audit_built(
    lock_text: &str,
    git_tool: bool,
    index: &[registry::IndexEntry],
    built: &Built,
) -> Report {
    let (dir, db) = fixture();
    let lock_path = dir.path().join("Cargo.lock");
    fs::write(&lock_path, lock_text).unwrap();
    let mut auditor = Auditor::new(db);
    for name in ["mytool", "smallvec", "ansi_term", "gitdep", "localdep"] {
        let entries = index.iter().filter(|e| e.name == name).cloned().collect();
        auditor.index.insert(name, entries);
    }
    auditor
        .audit(&lock_path, "mytool", git_tool, built)
        .unwrap()
}

fn audit_file(lock_text: &str, git_tool: bool, index: &[registry::IndexEntry]) -> Report {
    audit_built(lock_text, git_tool, index, &all_of(lock_text))
}

/// `mytool` 1.0.1 is patched; `smallvec` is vulnerable but only an optional dependency.
const OPTIONAL_LOCK: &str = r#"version = 3

[[package]]
name = "mytool"
version = "1.0.1"

[[package]]
name = "smallvec"
version = "1.6.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

#[test]
fn packages_that_are_not_built_are_skipped() {
    let built = Built::from([("mytool".to_string(), Version::new(1, 0, 1))]);
    let r = audit_built(OPTIONAL_LOCK, false, &[], &built);
    assert!(r.vulnerabilities.is_empty());
    assert_eq!(r.skipped, 1);

    let r = audit_built(OPTIONAL_LOCK, false, &[], &all_of(OPTIONAL_LOCK));
    assert_eq!(r.vulnerabilities.len(), 1);
    assert_eq!(r.vulnerabilities[0].package.name, "smallvec");
    assert_eq!(r.skipped, 0);
}

const LOCK: &str = r#"version = 3

[[package]]
name = "mytool"
version = "1.0.0"

[[package]]
name = "ansi_term"
version = "0.12.1"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

#[test]
fn yanked_covers_the_root_of_registry_tools() {
    let index = [
        entry("mytool", "1.0.0", true),
        entry("ansi_term", "0.12.1", true),
    ];
    let r = audit_file(LOCK, false, &index);
    let yanked: Vec<_> = r.warnings["yanked"]
        .iter()
        .map(|w| w.package.name.as_str())
        .collect();
    assert_eq!(yanked, vec!["mytool", "ansi_term"]);
    assert_eq!(r.vulnerabilities.len(), 1);

    // A git tool's version isn't a crates.io release, so its root isn't checked.
    let r = audit_file(LOCK, true, &index);
    let yanked: Vec<_> = r.warnings["yanked"]
        .iter()
        .map(|w| w.package.name.as_str())
        .collect();
    assert_eq!(yanked, vec!["ansi_term"]);
}

#[test]
fn not_yanked_means_no_yanked_warnings() {
    let r = audit_file(LOCK, false, &[entry("ansi_term", "0.12.1", false)]);
    assert!(!r.warnings.contains_key("yanked"));
}

#[test]
fn fix_suggests_lowest_patched_release_that_is_not_yanked() {
    let (_dir, db) = fixture();
    let r = scan(&db, &lock(), "mytool", false);
    let v = &r.vulnerabilities[1];
    let mut index = IndexCache::default();
    index.insert(
        "smallvec",
        vec![
            entry("smallvec", "1.6.0", false),
            entry("smallvec", "1.6.1", true),
            entry("smallvec", "1.6.2", false),
            entry("smallvec", "1.7.0", false),
        ],
    );
    assert_eq!(
        suggestion("mytool", "smallvec", &v.package.version, &[v], &mut index),
        format!("cargo {SUBCOMMAND} update mytool smallvec@1.6.0 --precise 1.6.2")
    );
}

#[test]
fn fix_without_patched_versions() {
    let (_dir, db) = db(&[("RUSTSEC-2020-0040", "dead", "", "")]);
    let r = scan(
        &db,
        &lock_of(&[("root", "1.0.0", None), ("dead", "1.0.0", Some(REGISTRY))]),
        "root",
        false,
    );
    let v = &r.vulnerabilities[0];
    let fix = suggestion(
        "root",
        "dead",
        &v.package.version,
        &[v],
        &mut IndexCache::default(),
    );
    assert!(fix.starts_with("no patched version"), "{fix}");
}

#[test]
fn evaluate_counts_what_is_left_after_ignores() {
    let (_dir, db) = fixture();
    let r = scan(&db, &lock(), "mytool", false);
    let mut index = IndexCache::default();
    index.insert("smallvec", vec![entry("smallvec", "1.6.1", false)]);
    let none = AuditIgnore::default();
    let o = evaluate("mytool", &r, &none, "2026-01-01", &mut index);
    assert_eq!((o.vulnerabilities, o.warnings), (2, 2));

    let some = ignores(
        "[[ignore]]\nid = \"CVE-2021-25900\"\nreason = \"x\"\n[[ignore]]\nid = \"RUSTSEC-2020-0004\"\nreason = \"git\"\n",
    );
    let o = evaluate("mytool", &r, &some, "2026-01-01", &mut index);
    assert_eq!((o.vulnerabilities, o.warnings), (1, 1));
}
