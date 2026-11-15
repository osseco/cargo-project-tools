use super::*;

const REPORT: &str = r#"{
  "database": {}, "lockfile": {}, "settings": {},
  "vulnerabilities": {"found": true, "count": 1, "list": [{
    "advisory": {"id": "RUSTSEC-2021-0003", "title": "Buffer overflow", "aliases": ["CVE-2021-25900"]},
    "versions": {"patched": [">=1.6.1"], "unaffected": []},
    "package": {"name": "smallvec", "version": "1.6.0"}
  }]},
  "warnings": {"unmaintained": [{
    "kind": "unmaintained",
    "package": {"name": "ansi_term", "version": "0.12.1"},
    "advisory": {"id": "RUSTSEC-2021-0139", "title": "unmaintained"}
  }]}
}"#;

fn ignores(toml_text: &str) -> AuditIgnore {
    toml::from_str(toml_text).unwrap()
}

#[test]
fn parses_cargo_audit_json() {
    let r: Report = serde_json::from_str(REPORT).unwrap();
    assert_eq!(r.vulnerabilities.list[0].package.name, "smallvec");
    assert_eq!(r.warnings["unmaintained"].len(), 1);
}

#[test]
fn ignore_by_alias_and_unused() {
    let r: Report = serde_json::from_str(REPORT).unwrap();
    let file = ignores(
        r#"
        [[ignore]]
        id = "CVE-2021-25900"
        reason = "not used"
        [[ignore]]
        id = "RUSTSEC-2021-0139"
        reason = "fine"
        [[ignore]]
        id = "RUSTSEC-1999-0001"
        reason = "stale"
        "#,
    );
    let mut ig = Ignores::new(&file, "2026-01-01", "t");
    assert!(ig.matches(&r.vulnerabilities.list[0].advisory).is_some());
    assert!(
        ig.matches(r.warnings["unmaintained"][0].advisory.as_ref().unwrap())
            .is_some()
    );
    let unused: Vec<_> = ig.unused().map(|e| e.id.as_str()).collect();
    assert_eq!(unused, vec!["RUSTSEC-1999-0001"]);
}

#[test]
fn expired_ignore_does_not_apply() {
    let r: Report = serde_json::from_str(REPORT).unwrap();
    let file = ignores(
        "[[ignore]]\nid = \"RUSTSEC-2021-0003\"\nreason = \"x\"\nexpires = \"2025-01-01\"\n",
    );
    let mut ig = Ignores::new(&file, "2026-01-01", "t");
    assert!(ig.matches(&r.vulnerabilities.list[0].advisory).is_none());
}

#[test]
fn civil_dates() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(19_723), (2024, 1, 1));
    assert_eq!(civil_from_days(19_782), (2024, 2, 29));
}
