use super::*;
use crate::test_registry::{Registry, tool};

const LOCK_A: &str = r#"
version = 3

[[package]]
name = "tool"
version = "1.0.0"
dependencies = ["dep", "gone"]

[[package]]
name = "dep"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "gone"
version = "0.1.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

const LOCK_B: &str = r#"
version = 3

[[package]]
name = "tool"
version = "1.0.0"
dependencies = ["dep", "new"]

[[package]]
name = "dep"
version = "1.0.5"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "new"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

#[test]
fn root_check() {
    let lock = parse(LOCK_A).unwrap();
    check_root(&lock, "tool", "1.0.0").unwrap();
    assert!(check_root(&lock, "tool", "1.0.1").is_err());
    assert!(check_root(&lock, "other", "1.0.0").is_err());
}

#[test]
fn only_https_sources() {
    let lock = |source: &str| {
        format!(
            "version = 4\n\n[[package]]\nname = \"tool\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"dep\"\nversion = \"1.0.0\"\nsource = \"{source}\"\n"
        )
    };
    assert!(
        parse(&lock(
            "registry+https://github.com/rust-lang/crates.io-index"
        ))
        .is_ok()
    );
    assert!(parse(&lock("sparse+https://index.crates.io/")).is_ok());
    assert!(
        parse(&lock(
            "git+https://example.com/dep.git#0123456789abcdef0123456789abcdef01234567"
        ))
        .is_ok()
    );
    for bad in [
        "git+ssh://git@example.com/dep.git#0123456789abcdef0123456789abcdef01234567",
        "git+http://example.com/dep.git#0123456789abcdef0123456789abcdef01234567",
        "git+file:///home/me/secret#0123456789abcdef0123456789abcdef01234567",
        "registry+http://example.com/index",
    ] {
        let err = parse(&lock(bad)).unwrap_err().to_string();
        assert!(err.contains("only https"), "{bad}: {err}");
    }
}

#[test]
fn diffs_locks() {
    let changes = diff(&parse(LOCK_A).unwrap(), &parse(LOCK_B).unwrap());
    assert_eq!(
        changes,
        vec![
            Change::Updated(
                "dep".into(),
                "1.0.0".parse().unwrap(),
                "1.0.5".parse().unwrap()
            ),
            Change::Removed("gone".into(), "0.1.0".parse().unwrap()),
            Change::Added("new".into(), "2.0.0".parse().unwrap()),
        ]
    );
}

#[test]
fn carry_forward_candidates() {
    let out = newer_than(&parse(LOCK_B).unwrap(), &parse(LOCK_A).unwrap());
    assert_eq!(
        out,
        vec![(
            "dep".into(),
            "1.0.0".parse().unwrap(),
            "1.0.5".parse().unwrap()
        )]
    );
}

fn registry() -> Registry {
    Registry::new(&[
        ("dep", "1.0.0"),
        ("dep", "1.1.0"),
        ("dep", "2.0.0"),
        ("extra", "0.1.0"),
    ])
}

fn versions_of(lock: &Lockfile, name: &str) -> Vec<String> {
    lock.packages
        .iter()
        .filter(|p| p.name.as_str() == name)
        .map(|p| p.version.to_string())
        .collect()
}

#[test]
fn shipped_lock_is_used_as_is() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = tool(dir.path(), "1.0.0", &[]);
    let lock_path = dir.path().join("Cargo.lock");
    std::fs::write(&lock_path, LOCK_A).unwrap();
    // No cargo args: a cargo run here would need the network and fail.
    let (text, start) = shipped_or_generated(&manifest, &lock_path, &[]).unwrap();
    assert_eq!(start, Start::Shipped);
    assert_eq!(text, LOCK_A);
}

#[test]
fn missing_lock_is_generated_without_building() {
    let reg = registry();
    let dir = tempfile::tempdir().unwrap();
    let manifest = tool(dir.path(), "1.0.0", &[("dep", "1"), ("extra", "0.1")]);
    let lock_path = dir.path().join("Cargo.lock");
    let (text, start) = shipped_or_generated(&manifest, &lock_path, &reg.cargo_args()).unwrap();
    assert_eq!(start, Start::Generated);
    let lock = parse(&text).unwrap();
    check_root(&lock, "tool", "1.0.0").unwrap();
    assert_eq!(versions_of(&lock, "dep"), ["1.1.0"]);
    assert_eq!(versions_of(&lock, "extra"), ["0.1.0"]);
    assert!(!dir.path().join("build-ran").exists());
    assert!(!dir.path().join("target").exists());
}

#[test]
fn unresolvable_tool_gets_no_lock() {
    let reg = registry();
    let dir = tempfile::tempdir().unwrap();
    let manifest = tool(dir.path(), "1.0.0", &[("dep", "3")]);
    let lock_path = dir.path().join("Cargo.lock");
    assert!(shipped_or_generated(&manifest, &lock_path, &reg.cargo_args()).is_err());
    assert!(!lock_path.exists());
}

/// Our lock for tool 1.0.0 depending on `dep` and `extra`, with `dep` pinned to 1.0.0.
fn pinned_lock(reg: &Registry, dir: &Path) -> std::path::PathBuf {
    let manifest = tool(dir, "1.0.0", &[("dep", "1"), ("extra", "0.1")]);
    let lock_path = dir.join("Cargo.lock");
    let (text, _) = shipped_or_generated(&manifest, &lock_path, &reg.cargo_args()).unwrap();
    // `cargo update --precise` ignores a replaced directory source, so pin by hand.
    let pinned = text.replace(
        "name = \"dep\"\nversion = \"1.1.0\"",
        "name = \"dep\"\nversion = \"1.0.0\"",
    );
    assert_eq!(versions_of(&parse(&pinned).unwrap(), "dep"), ["1.0.0"]);
    std::fs::write(&lock_path, pinned).unwrap();
    lock_path
}

#[test]
fn reseed_keeps_pins() {
    let reg = registry();
    let old = tempfile::tempdir().unwrap();
    let ours = pinned_lock(&reg, old.path());

    let new = tempfile::tempdir().unwrap();
    let manifest = tool(new.path(), "1.1.0", &[("dep", "1"), ("extra", "0.1")]);
    let lock_path = new.path().join("Cargo.lock");
    reseed(&manifest, &lock_path, &ours, &reg.cargo_args()).unwrap();

    let lock = load(&lock_path).unwrap();
    check_root(&lock, "tool", "1.1.0").unwrap();
    assert_eq!(versions_of(&lock, "dep"), ["1.0.0"]);
    assert_eq!(versions_of(&lock, "extra"), ["0.1.0"]);
    assert!(!new.path().join("build-ran").exists());
}

#[test]
fn reseed_moves_pins_only_when_required_and_drops_unused() {
    let reg = registry();
    let old = tempfile::tempdir().unwrap();
    let ours = pinned_lock(&reg, old.path());

    let new = tempfile::tempdir().unwrap();
    let manifest = tool(new.path(), "2.0.0", &[("dep", "1.1")]);
    let lock_path = new.path().join("Cargo.lock");
    reseed(&manifest, &lock_path, &ours, &reg.cargo_args()).unwrap();

    let lock = load(&lock_path).unwrap();
    check_root(&lock, "tool", "2.0.0").unwrap();
    assert_eq!(versions_of(&lock, "dep"), ["1.1.0"]);
    assert!(versions_of(&lock, "extra").is_empty());
}

#[test]
fn semver_compat() {
    let v = |s: &str| semver::Version::parse(s).unwrap();
    assert!(compatible(&v("1.0.0"), &v("1.9.0")));
    assert!(!compatible(&v("0.1.0"), &v("0.2.0")));
    assert!(compatible(&v("0.2.1"), &v("0.2.9")));
}

#[test]
fn a_bare_name_with_two_locked_versions_is_ambiguous() {
    let lock = parse(&format!(
        "{LOCK_A}\n[[package]]\nname = \"dep\"\nversion = \"2.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
    ))
    .unwrap();
    let deps = ["dep", "gone", "dep@1.0.0", "missing"].map(String::from);
    assert_eq!(
        ambiguous(&lock, &deps),
        [(
            "dep".to_string(),
            vec!["dep@1.0.0".to_string(), "dep@2.0.0".to_string()]
        )]
    );
}
