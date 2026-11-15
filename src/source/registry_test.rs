use super::*;

fn entry(v: &str, yanked: bool) -> IndexEntry {
    IndexEntry {
        name: "x".into(),
        vers: v.parse().unwrap(),
        cksum: String::new(),
        yanked,
    }
}

#[test]
fn index_paths() {
    assert_eq!(index_path("a"), "1/a");
    assert_eq!(index_path("ab"), "2/ab");
    assert_eq!(index_path("abc"), "3/a/abc");
    assert_eq!(index_path("Serde"), "se/rd/serde");
}

#[test]
fn selects_latest_stable_non_yanked() {
    let e = [
        entry("1.0.0", false),
        entry("1.2.0", true),
        entry("1.1.0", false),
        entry("2.0.0-rc.1", false),
    ];
    assert_eq!(select(&e, None).unwrap().vers.to_string(), "1.1.0");
    let req = parse_req("1.0.0").unwrap();
    assert_eq!(select(&e, Some(&req)).unwrap().vers.to_string(), "1.0.0");
    let req = parse_req("=2.0.0-rc.1").unwrap();
    assert_eq!(
        select(&e, Some(&req)).unwrap().vers.to_string(),
        "2.0.0-rc.1"
    );
}

#[test]
fn lowest_patched_version() {
    let e = [
        entry("1.0.0", false),
        entry("1.0.1", true),
        entry("1.0.2", false),
        entry("1.1.0", false),
    ];
    let cur = Version::parse("1.0.0").unwrap();
    let a = vec![VersionReq::parse(">=1.0.1").unwrap()];
    assert_eq!(
        lowest_patched(&e, &cur, std::slice::from_ref(&a))
            .unwrap()
            .vers
            .to_string(),
        "1.0.2"
    );
    // A second advisory needing >=1.1.0 moves the fix up.
    let b = vec![VersionReq::parse(">=1.1.0").unwrap()];
    assert_eq!(
        lowest_patched(&e, &cur, &[a, b]).unwrap().vers.to_string(),
        "1.1.0"
    );
}

#[test]
fn rejects_unsafe_archive_paths() {
    assert!(inner_path(Path::new("x-1.0.0/../evil"), "x-1.0.0").is_err());
    assert!(inner_path(Path::new("other/file"), "x-1.0.0").is_err());
    assert_eq!(
        inner_path(Path::new("x-1.0.0/src/lib.rs"), "x-1.0.0").unwrap(),
        Some(PathBuf::from("src/lib.rs"))
    );
}

#[test]
fn rejects_bad_names() {
    assert!(validate_name("../etc").is_err());
    assert!(validate_name("cargo-nextest").is_ok());
}
