use super::*;

fn script(dir: &Path, package: &str, hash: &str) -> Item {
    Item {
        kind: Kind::BuildScript,
        package: package.into(),
        version: "1.0.0".into(),
        path: "build.rs".into(),
        hash: hash.into(),
        location: dir.join(package).join("build.rs"),
        base: dir.join(package),
        vendored: true,
    }
}

fn proc_macro(package: &str, checksum: &str) -> Item {
    Item {
        kind: Kind::ProcMacro,
        package: package.into(),
        version: "1.0.0".into(),
        path: String::new(),
        hash: checksum.into(),
        location: PathBuf::from("/v").join(package),
        base: PathBuf::from("/v").join(package),
        vendored: true,
    }
}

#[test]
fn entries_parse() {
    let entries = parse(
        r#"
[[build-script]]
package = "libc"
path = "build.rs"
sha256 = "aa"
action = "approve"

[[build-script]]
package = "ring"
path = "build.rs"
sha256 = "bb"
action = "override"
file = "overrides/ring/build.rs"

[[proc-macro]]
package = "serde_derive"
checksum = "cc"
action = "approve"

[[cargo-config]]
path = ".cargo/config.toml"
sha256 = "dd"
action = "strip"
"#,
    )
    .unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[1].file.as_deref(), Some("overrides/ring/build.rs"));
    assert_eq!(entries[2].kind, Kind::ProcMacro);
    assert_eq!(entries[2].hash, "cc");
    assert_eq!(entries[3].action, Action::Strip);
}

#[test]
fn invalid_entries_are_refused() {
    let bad = [
        // a proc macro can only be approved
        "[[proc-macro]]\npackage = \"x\"\nchecksum = \"c\"\naction = \"strip\"\n",
        // override needs a file
        "[[build-script]]\npackage = \"x\"\npath = \"build.rs\"\nsha256 = \"a\"\naction = \"override\"\n",
        // a file only goes with override
        "[[build-script]]\npackage = \"x\"\npath = \"build.rs\"\nsha256 = \"a\"\naction = \"approve\"\nfile = \"f\"\n",
        // override files stay in the crate directory
        "[[build-script]]\npackage = \"x\"\npath = \"build.rs\"\nsha256 = \"a\"\naction = \"override\"\nfile = \"../f\"\n",
        // fields of another kind
        "[[cargo-config]]\npackage = \"x\"\npath = \"c\"\nsha256 = \"a\"\naction = \"approve\"\n",
        // missing hash
        "[[build-script]]\npackage = \"x\"\npath = \"build.rs\"\naction = \"approve\"\n",
        // unknown action
        "[[build-script]]\npackage = \"x\"\npath = \"build.rs\"\nsha256 = \"a\"\naction = \"maybe\"\n",
    ];
    for text in bad {
        assert!(parse(text).is_err(), "accepted:\n{text}");
    }
}

#[test]
fn new_code_halts_until_approved() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(FILE);
    let items = || {
        vec![
            script(tmp.path(), "libc", "aa"),
            proc_macro("serde_derive", "cc"),
        ]
    };

    let err = check(&file, items(), false, tmp.path())
        .unwrap_err()
        .to_string();
    assert!(err.contains("2 pieces of build-time code"), "{err}");
    assert!(!file.exists());

    let decided = check(&file, items(), true, tmp.path()).unwrap();
    assert_eq!(decided.len(), 2);
    assert!(decided.iter().all(|d| d.action == Action::Approve));
    let entries = load(&file).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        fs::read_to_string(&file)
            .unwrap()
            .starts_with("# Code that runs")
    );

    // Recorded: passes without the flag, and the file isn't rewritten.
    let before = fs::read_to_string(&file).unwrap();
    assert_eq!(check(&file, items(), false, tmp.path()).unwrap().len(), 2);
    assert_eq!(fs::read_to_string(&file).unwrap(), before);
}

#[test]
fn changed_code_keeps_its_action_and_drops_the_old_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(FILE);
    fs::write(
        &file,
        "# why we override ring\n[[build-script]]\npackage = \"ring\"\npath = \"build.rs\"\nsha256 = \"old\"\naction = \"override\"\nfile = \"overrides/ring.rs\"\n\n[[build-script]]\npackage = \"gone\"\npath = \"build.rs\"\nsha256 = \"x\"\naction = \"approve\"\n",
    )
    .unwrap();
    let items = || vec![script(tmp.path(), "ring", "new")];

    let err = check(&file, items(), false, tmp.path())
        .unwrap_err()
        .to_string();
    assert!(err.contains("1 piece of"), "{err}");

    let decided = check(&file, items(), true, tmp.path()).unwrap();
    assert_eq!(decided[0].action, Action::Override);
    assert_eq!(decided[0].file.as_deref(), Some("overrides/ring.rs"));
    let entries = load(&file).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].hash, "new");
    assert_eq!(entries[0].action, Action::Override);
    assert!(
        fs::read_to_string(&file)
            .unwrap()
            .starts_with("# why we override ring")
    );
}

#[test]
fn two_versions_of_a_proc_macro_are_separate_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(FILE);
    let items = || {
        vec![
            proc_macro("thiserror-impl", "v1"),
            proc_macro("thiserror-impl", "v2"),
        ]
    };
    check(&file, items(), true, tmp.path()).unwrap();
    assert_eq!(load(&file).unwrap().len(), 2);
    assert!(check(&file, items(), false, tmp.path()).is_ok());
}

fn vendored_package(dir: &Path) -> PathBuf {
    let pkg = dir.join("vendor/dep-1.0.0");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("build.rs"), "fn main() { upstream() }").unwrap();
    fs::write(
        pkg.join("Cargo.toml"),
        "[package]\nname = \"dep\"\nversion = \"1.0.0\"\nbuild = \"build.rs\"\n",
    )
    .unwrap();
    fs::write(
        pkg.join(".cargo-checksum.json"),
        r#"{"files":{"build.rs":"aa","Cargo.toml":"bb","src/lib.rs":"cc"},"package":"pp"}"#,
    )
    .unwrap();
    pkg
}

fn checksum_files(pkg: &Path) -> serde_json::Value {
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(pkg.join(".cargo-checksum.json")).unwrap())
            .unwrap();
    assert_eq!(json["package"], "pp");
    json["files"].clone()
}

fn decided(item: Item, action: Action, file: Option<&str>) -> Decided {
    Decided {
        item,
        action,
        file: file.map(str::to_string),
    }
}

#[test]
fn override_replaces_a_vendored_build_script() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = vendored_package(tmp.path());
    let crate_dir = tmp.path().join("crate");
    fs::create_dir_all(crate_dir.join("overrides")).unwrap();
    fs::write(crate_dir.join("overrides/dep.rs"), "fn main() {}").unwrap();
    let mut item = script(tmp.path(), "dep", "aa");
    item.base = pkg.clone();
    item.location = pkg.join("build.rs");

    let digest = apply(
        &[decided(item, Action::Override, Some("overrides/dep.rs"))],
        &crate_dir,
    )
    .unwrap();

    assert!(!digest.is_empty());
    assert_eq!(
        fs::read_to_string(pkg.join("build.rs")).unwrap(),
        "fn main() {}"
    );
    let files = checksum_files(&pkg);
    assert!(files.get("build.rs").is_none());
    assert!(files.get("src/lib.rs").is_some());
}

#[test]
fn strip_turns_off_a_vendored_build_script() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = vendored_package(tmp.path());
    let mut item = script(tmp.path(), "dep", "aa");
    item.base = pkg.clone();
    item.location = pkg.join("build.rs");

    apply(&[decided(item, Action::Strip, None)], tmp.path()).unwrap();

    let manifest: toml::Table = fs::read_to_string(pkg.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(manifest["package"]["build"].as_bool(), Some(false));
    assert!(checksum_files(&pkg).get("Cargo.toml").is_none());
}

#[test]
fn approve_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = vendored_package(tmp.path());
    let mut item = script(tmp.path(), "dep", "aa");
    item.base = pkg.clone();
    item.location = pkg.join("build.rs");
    let digest = apply(&[decided(item, Action::Approve, None)], tmp.path()).unwrap();
    assert!(digest.is_empty());
    assert_eq!(checksum_files(&pkg).as_object().unwrap().len(), 3);
}

fn checkout_at(root: &Path) -> Checkout {
    Checkout::at(root.to_path_buf(), root.join("ws"), root.join("ws/tool"))
}

#[test]
fn shipped_configs_along_the_search_path() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout = checkout_at(tmp.path());
    fs::create_dir_all(checkout.package_dir.join(".cargo")).unwrap();
    fs::create_dir_all(checkout.workspace_root.join(".cargo")).unwrap();
    fs::write(
        checkout.package_dir.join(".cargo/config.toml"),
        "[alias]\nx = \"build\"\n",
    )
    .unwrap();
    fs::write(
        checkout.workspace_root.join(".cargo/config"),
        "[build]\nrustc-wrapper = \"./evil.sh\"\n",
    )
    .unwrap();

    let items = shipped_configs(&checkout).unwrap();
    let paths: Vec<&str> = items.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, ["ws/tool/.cargo/config.toml", "ws/.cargo/config"]);
    assert_eq!(
        items[1].hash,
        sha256_hex(b"[build]\nrustc-wrapper = \"./evil.sh\"\n")
    );

    // A stripped config is deleted from the checkout.
    let item = items[1].clone();
    let location = item.location.clone();
    apply(&[decided(item, Action::Strip, None)], tmp.path()).unwrap();
    assert!(!location.exists());
}

#[test]
fn config_include_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout = checkout_at(tmp.path());
    fs::create_dir_all(checkout.package_dir.join(".cargo")).unwrap();
    fs::write(
        checkout.package_dir.join(".cargo/config.toml"),
        "include = \"more.toml\"\n",
    )
    .unwrap();
    let err = shipped_configs(&checkout).unwrap_err().to_string();
    assert!(err.contains("include"), "{err}");
}

#[cfg(unix)]
#[test]
fn config_links_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout = checkout_at(tmp.path());
    fs::create_dir_all(checkout.package_dir.join(".cargo")).unwrap();
    fs::write(tmp.path().join("elsewhere.toml"), "").unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("elsewhere.toml"),
        checkout.package_dir.join(".cargo/config.toml"),
    )
    .unwrap();
    assert!(shipped_configs(&checkout).is_err());
}

#[test]
fn script_hash_covers_its_directory_when_not_at_the_root() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path();
    fs::write(pkg.join("build.rs"), "a").unwrap();
    assert_eq!(
        script_hash(pkg, &pkg.join("build.rs")).unwrap(),
        sha256_hex(b"a")
    );

    fs::create_dir_all(pkg.join("build")).unwrap();
    fs::write(pkg.join("build/main.rs"), "mod util;").unwrap();
    fs::write(pkg.join("build/util.rs"), "one").unwrap();
    let first = script_hash(pkg, &pkg.join("build/main.rs")).unwrap();
    fs::write(pkg.join("build/util.rs"), "two").unwrap();
    assert_ne!(script_hash(pkg, &pkg.join("build/main.rs")).unwrap(), first);
}
