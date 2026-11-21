use super::*;

fn script(dir: &Path, package: &str, hash: &str) -> Item {
    Item {
        kind: Kind::BuildScript,
        package: package.into(),
        version: "1.0.0".into(),
        path: "build.rs".into(),
        hash: hash.into(),
        pin: Pin::None,
        location: dir.join(package).join("build.rs"),
        base: dir.join(package),
        vendored: true,
        pulled: Pulled::default(),
    }
}

/// `item` placed under `dir`, as everything that's built is under the project.
fn under(mut item: Item, dir: &Path) -> Item {
    item.location = dir.join(&item.package);
    item.base = item.location.clone();
    item
}

fn proc_macro(package: &str, checksum: &str) -> Item {
    Item {
        kind: Kind::ProcMacro,
        package: package.into(),
        version: "1.0.0".into(),
        path: String::new(),
        hash: checksum.into(),
        pin: Pin::None,
        location: PathBuf::from("/v").join(package),
        base: PathBuf::from("/v").join(package),
        vendored: true,
        pulled: Pulled::default(),
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
            under(proc_macro("serde_derive", "cc"), tmp.path()),
        ]
    };

    let err = check(&file, items(), Mode::Report, &KnownMap::new(), tmp.path())
        .unwrap_err()
        .to_string();
    assert!(err.contains("2 pieces of build-time code"), "{err}");
    // Tests run with or without a terminal, and the hint follows it.
    if !crate::screen::interactive() {
        assert!(err.contains("where you have a terminal"), "{err}");
    }
    assert!(!file.exists());

    let decided = check(
        &file,
        items(),
        Mode::ApproveAll,
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap();
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
    assert_eq!(
        check(&file, items(), Mode::Report, &KnownMap::new(), tmp.path())
            .unwrap()
            .len(),
        2
    );
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

    let err = check(&file, items(), Mode::Report, &KnownMap::new(), tmp.path())
        .unwrap_err()
        .to_string();
    assert!(err.contains("1 piece of"), "{err}");

    let decided = check(
        &file,
        items(),
        Mode::ApproveAll,
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap();
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
fn paste_ready_entries_parse_back() {
    let tmp = tempfile::tempdir().unwrap();
    let text = paste_ready(&script(tmp.path(), "libc", "aa"), None, None, tmp.path())
        + &paste_ready(&proc_macro("serde_derive", "cc"), None, None, tmp.path());
    let entries = parse(&text).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].package, "libc");
    assert_eq!(entries[0].hash, "aa");
    assert_eq!(entries[0].action, Action::Approve);
    assert_eq!(entries[1].kind, Kind::ProcMacro);
    assert_eq!(entries[1].hash, "cc");
}

#[test]
fn paste_ready_entry_of_changed_code_keeps_its_decision() {
    let tmp = tempfile::tempdir().unwrap();
    let previous = Entry {
        kind: Kind::BuildScript,
        package: "ring".into(),
        path: "build.rs".into(),
        hash: "old".into(),
        pin: Pin::None,
        action: Action::Override,
        file: Some("overrides/ring.rs".into()),
    };
    let text = paste_ready(
        &script(tmp.path(), "ring", "new"),
        Some(&previous),
        None,
        tmp.path(),
    );
    assert!(
        text.contains("# replaces the entry with sha256 = \"old\""),
        "{text}"
    );
    let entries = parse(&text).unwrap();
    assert_eq!(entries[0].hash, "new");
    assert_eq!(entries[0].action, Action::Override);
    assert_eq!(entries[0].file.as_deref(), Some("overrides/ring.rs"));
}

#[test]
fn review_records_name_the_hash_and_what_it_replaces() {
    let tmp = tempfile::tempdir().unwrap();
    let previous = Entry {
        kind: Kind::BuildScript,
        package: "ring".into(),
        path: "build.rs".into(),
        hash: "old".into(),
        pin: Pin::None,
        action: Action::Strip,
        file: None,
    };
    let item = review_item(
        &script(tmp.path(), "ring", "new"),
        Some(&previous),
        None,
        "v/ring/build.rs".into(),
    );
    assert_eq!(item.kind, "build-script");
    assert_eq!(item.sha256.as_deref(), Some("new"));
    assert_eq!(item.checksum, None);
    let previous = item.previous.unwrap();
    assert_eq!(previous.action, "strip");
    assert_eq!(previous.sha256.as_deref(), Some("old"));

    let known = Known {
        tool: "other".into(),
        action: Action::Approve,
        file: None,
    };
    let item = review_item(
        &proc_macro("serde_derive", "cc"),
        None,
        Some(&known),
        "v".into(),
    );
    assert_eq!(item.checksum.as_deref(), Some("cc"));
    assert_eq!(item.sha256, None);
    assert_eq!(item.known.unwrap().tool, "other");
}

#[test]
fn a_new_decision_for_changed_code_updates_the_entry_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(FILE);
    fs::write(
        &file,
        "# why we override ring\n[[build-script]]\npackage = \"ring\"\npath = \"build.rs\"\nsha256 = \"old\"\naction = \"override\"\nfile = \"overrides/ring.rs\"\n",
    )
    .unwrap();
    let previous = load(&file).unwrap().remove(0);
    let decided = record(
        &file,
        vec![Decision {
            item: script(tmp.path(), "ring", "new"),
            previous: Some(&previous),
            action: Action::Strip,
            file: None,
        }],
        &[&previous],
    )
    .unwrap();
    assert_eq!(decided[0].action, Action::Strip);
    let entries = load(&file).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].hash, "new");
    assert_eq!(entries[0].action, Action::Strip);
    assert_eq!(entries[0].file, None);
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
            under(proc_macro("thiserror-impl", "v1"), tmp.path()),
            under(proc_macro("thiserror-impl", "v2"), tmp.path()),
        ]
    };
    check(
        &file,
        items(),
        Mode::ApproveAll,
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap();
    assert_eq!(load(&file).unwrap().len(), 2);
    assert!(check(&file, items(), Mode::Report, &KnownMap::new(), tmp.path()).is_ok());
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
fn an_unedited_commented_out_copy_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = vendored_package(tmp.path());
    let crate_dir = tmp.path().join("crate");
    fs::create_dir_all(crate_dir.join("overrides")).unwrap();
    fs::write(
        crate_dir.join("overrides/dep.rs"),
        format!("// {}\n// fn main() {{}}\n", tui::UNREVIEWED),
    )
    .unwrap();
    let mut item = script(tmp.path(), "dep", "aa");
    item.base = pkg.clone();
    item.location = pkg.join("build.rs");

    let err = apply(
        &[decided(item, Action::Override, Some("overrides/dep.rs"))],
        &crate_dir,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("commented-out copy"), "{err}");
    assert_eq!(
        fs::read_to_string(pkg.join("build.rs")).unwrap(),
        "fn main() { upstream() }"
    );
}

/// A build script on disk whose hash is what `collect` would record.
fn real_script(dir: &Path, package: &str) -> Item {
    let pkg = dir.join("vendor").join(package);
    fs::create_dir_all(&pkg).unwrap();
    fs::write(pkg.join("build.rs"), format!("fn main() {{ {package}() }}")).unwrap();
    let mut item = script(dir, package, "");
    item.location = pkg.join("build.rs");
    item.hash = script_hash(&pkg, &item.location, &Pulled::default(), true).unwrap();
    item.base = pkg;
    item
}

#[test]
fn screen_decisions_are_recorded_and_the_undecided_still_fail() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(FILE);
    fs::write(
        &file,
        "[[build-script]]\npackage = \"b\"\npath = \"build.rs\"\nsha256 = \"old\"\naction = \"strip\"\n\n[[build-script]]\npackage = \"gone\"\npath = \"build.rs\"\nsha256 = \"x\"\naction = \"approve\"\n",
    )
    .unwrap();
    let entries = load(&file).unwrap();
    let stale: Vec<&Entry> = entries.iter().collect();
    let a = real_script(tmp.path(), "a");
    let b = real_script(tmp.path(), "b");
    let before = vec![snapshot(&a).unwrap(), snapshot(&b).unwrap()];
    let pending = vec![(a, None), (b, Some(&entries[0]))];
    let choices = vec![
        Some(tui::Choice {
            action: Action::Approve,
            file: None,
        }),
        None,
    ];

    let err = settle(
        &file,
        pending,
        choices,
        &before,
        &stale,
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("1 piece of"), "{err}");
    let entries = load(&file).unwrap();
    let packages: Vec<(&str, Action)> = entries
        .iter()
        .map(|e| (e.package.as_str(), e.action))
        .collect();
    // `a` recorded, `b`'s old entry kept until it's decided, `gone` removed.
    assert_eq!(packages, [("b", Action::Strip), ("a", Action::Approve)]);
    assert_eq!(entries[0].hash, "old");
}

#[test]
fn code_changed_during_the_review_is_not_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(FILE);
    let approve = || {
        vec![Some(tui::Choice {
            action: Action::Approve,
            file: None,
        })]
    };

    let a = real_script(tmp.path(), "a");
    let before = vec![snapshot(&a).unwrap()];
    fs::write(&a.location, "fn main() { swapped() }").unwrap();
    let err = settle(
        &file,
        vec![(a, None)],
        approve(),
        &before,
        &[],
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("changed during the review"), "{err}");

    // A proc macro: any file of its crate.
    let pkg = tmp.path().join("vendor/m");
    fs::create_dir_all(pkg.join("src")).unwrap();
    fs::write(pkg.join("src/lib.rs"), "pub fn m() {}").unwrap();
    let mut m = proc_macro("m", "lock-checksum");
    m.location = pkg.clone();
    m.base = pkg.clone();
    let before = vec![snapshot(&m).unwrap()];
    fs::write(pkg.join("src/extra.rs"), "pub fn sneaky() {}").unwrap();
    let err = settle(
        &file,
        vec![(m.clone(), None)],
        approve(),
        &before,
        &[],
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("changed during the review"), "{err}");
    assert!(!file.exists());

    // Unchanged, it's recorded with its lock checksum.
    let before = vec![snapshot(&m).unwrap()];
    settle(
        &file,
        vec![(m, None)],
        approve(),
        &before,
        &[],
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap();
    assert_eq!(load(&file).unwrap()[0].hash, "lock-checksum");
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
        script_hash(pkg, &pkg.join("build.rs"), &Pulled::default(), true).unwrap(),
        sha256_hex(b"a")
    );

    fs::create_dir_all(pkg.join("build")).unwrap();
    fs::write(pkg.join("build/main.rs"), "mod util;").unwrap();
    fs::write(pkg.join("build/util.rs"), "one").unwrap();
    let first = script_hash(pkg, &pkg.join("build/main.rs"), &Pulled::default(), true).unwrap();
    fs::write(pkg.join("build/util.rs"), "two").unwrap();
    assert_ne!(
        script_hash(pkg, &pkg.join("build/main.rs"), &Pulled::default(), true).unwrap(),
        first
    );
}

#[test]
fn script_hash_covers_the_files_it_pulls_in() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path();
    fs::write(
        pkg.join("build.rs"),
        "mod helper; const T: &str = include_str!(\"table.txt\");",
    )
    .unwrap();
    fs::write(pkg.join("helper.rs"), "pub fn f() {}").unwrap();
    fs::write(pkg.join("table.txt"), "one").unwrap();
    fs::create_dir_all(pkg.join("src")).unwrap();
    fs::write(pkg.join("src/lib.rs"), "untouched").unwrap();
    let script = pkg.join("build.rs");

    let pulled = pulled_by(pkg, &script).unwrap();
    assert_eq!(pulled.files, [pkg.join("helper.rs"), pkg.join("table.txt")]);
    let first = script_hash(pkg, &script, &pulled, true).unwrap();
    // Not the old single-file hash, so such entries come up once as changed.
    assert_ne!(first, sha256_hex(&fs::read(&script).unwrap()));
    // Each pulled-in file counts; the rest of the package doesn't.
    fs::write(pkg.join("src/lib.rs"), "changed").unwrap();
    assert_eq!(script_hash(pkg, &script, &pulled, true).unwrap(), first);
    fs::write(pkg.join("table.txt"), "two").unwrap();
    assert_ne!(script_hash(pkg, &script, &pulled, true).unwrap(), first);

    // In a directory: files there are covered by it and aren't listed again.
    fs::create_dir_all(pkg.join("build")).unwrap();
    fs::write(
        pkg.join("build/main.rs"),
        "mod util; include!(\"../gen.rs\");",
    )
    .unwrap();
    fs::write(pkg.join("build/util.rs"), "").unwrap();
    fs::write(pkg.join("gen.rs"), "fn g() {}").unwrap();
    let pulled = pulled_by(pkg, &pkg.join("build/main.rs")).unwrap();
    assert_eq!(pulled.files, [pkg.join("gen.rs")]);

    // Not followed: the whole package.
    fs::write(pkg.join("build.rs"), "fn main() { include!(PATH); }").unwrap();
    let pulled = pulled_by(pkg, &script).unwrap();
    assert!(pulled.whole_package.is_some());
    assert_eq!(
        script_hash(pkg, &script, &pulled, true).unwrap(),
        dir_hash(pkg, true).unwrap()
    );
}

#[test]
fn slash_separates_with_forward_slashes() {
    assert_eq!(slash(&Path::new("a").join("b").join("c.rs")), "a/b/c.rs");
}

/// A tool built from the test registry: `scripted` has a build script that leaves a
/// marker, `macro` is a proc macro, and `winonly`, with a build script, is a Windows-only
/// dependency.
struct Fixture {
    tmp: tempfile::TempDir,
    registry: crate::test_registry::Registry,
    project: Project,
    checkout: Checkout,
    vendored: Vendored,
}

impl Fixture {
    fn new() -> Fixture {
        use crate::test_registry::{Crate, Registry, tool_with, watching_marker_script};
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("markers")).unwrap();
        let registry = Registry::with(vec![
            // Watches a directory, as typenum's build script does.
            Crate::new("scripted", "1.0.0").build(&watching_marker_script(
                &tmp.path().join("markers/upstream"),
                "src",
            )),
            Crate::new("macro", "1.0.0").proc_macro(),
            Crate::new("winonly", "1.0.0").build("fn main() {}\n"),
        ]);
        let src = tmp.path().join("src");
        let manifest = tool_with(
            &src,
            "1.0.0",
            "[dependencies]\nscripted = \"1\"\nmacro = \"1\"\n\n[target.'cfg(windows)'.dependencies]\nwinonly = \"1\"\n",
        );
        crate::lock::shipped_or_generated(
            &manifest,
            &src.join("Cargo.lock"),
            &registry.cargo_args(),
        )
        .unwrap();
        let project = Project {
            root: tmp.path().to_path_buf(),
        };
        let checkout = Checkout::at(src.clone(), src.clone(), src);
        let vendored =
            crate::vendor::vendor(&project, "tool", &checkout, false, &registry.cargo_args())
                .unwrap();
        Fixture {
            tmp,
            registry,
            project,
            checkout,
            vendored,
        }
    }

    fn collect(&self) -> Vec<Item> {
        self.collect_for(None)
    }

    fn collect_for(&self, targets: Option<&[String]>) -> Vec<Item> {
        let selection = crate::graph::Selection {
            features: &[],
            targets,
            config: &[],
        };
        collect("tool", selection, &self.checkout, &self.vendored, true).unwrap()
    }

    fn marker(&self, name: &str) -> bool {
        self.tmp.path().join("markers").join(name).exists()
    }

    /// `cargo build` of the tool from the vendored sources. Returns cargo's JSON messages.
    fn build(&self) -> String {
        crate::process::output_quiet(
            crate::process::cargo()
                .args(["build", "--offline", "--locked", "--message-format", "json"])
                .arg("--manifest-path")
                .arg(self.checkout.manifest_path())
                .arg("--target-dir")
                .arg(self.tmp.path().join("target"))
                .args(&self.vendored.config_args),
        )
        .unwrap()
    }

    fn decide(&self, package: &str, action: Action, file: Option<&str>) -> Vec<Decided> {
        self.collect()
            .into_iter()
            .map(|item| {
                let mine = item.kind == Kind::BuildScript && item.package == package;
                Decided {
                    item,
                    action: if mine { action } else { Action::Approve },
                    file: if mine { file.map(str::to_string) } else { None },
                }
            })
            .collect()
    }
}

#[test]
fn collect_finds_build_time_code_in_vendored_sources() {
    let f = Fixture::new();
    let found: Vec<String> = f
        .collect()
        .iter()
        .map(|i| format!("{} {} {}", i.kind, i.package, i.vendored))
        .collect();
    assert_eq!(
        found,
        [
            "build-script scripted true",
            "build-script tool false",
            "build-script winonly true",
            "proc-macro macro true",
        ]
    );
}

#[test]
fn check_only_records_nothing() {
    let f = Fixture::new();
    let selection = || crate::graph::Selection {
        features: &[],
        targets: None,
        config: &[],
    };
    let check = || {
        check_only(
            &f.project,
            "tool",
            selection(),
            &f.checkout,
            &f.vendored,
            true,
        )
    };
    let err = check().unwrap_err().to_string();
    assert!(err.contains("4 pieces of build-time code"), "{err}");
    assert!(!f.project.review_path("tool").exists());

    review(
        &f.project,
        "tool",
        selection(),
        &f.checkout,
        &f.vendored,
        true,
        true,
    )
    .unwrap();
    check().unwrap();
}

fn packages(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.package.as_str()).collect()
}

#[test]
fn chosen_targets_leave_out_code_for_other_platforms() {
    let f = Fixture::new();
    let mac_and_linux = [
        "aarch64-apple-darwin".to_string(),
        "x86_64-unknown-linux-gnu".to_string(),
    ];
    assert_eq!(
        packages(&f.collect_for(Some(&mac_and_linux))),
        ["scripted", "tool", "macro"]
    );
    let windows = ["x86_64-pc-windows-msvc".to_string()];
    assert_eq!(
        packages(&f.collect_for(Some(&windows))),
        ["scripted", "tool", "winonly", "macro"]
    );
}

#[test]
fn approved_code_builds_as_shipped() {
    let f = Fixture::new();
    let decided = f.decide("scripted", Action::Approve, None);
    assert!(
        apply(&decided, &f.project.crate_dir("tool"))
            .unwrap()
            .is_empty()
    );
    f.build();
    assert!(f.marker("upstream"));
}

#[test]
fn overridden_build_script_runs_instead() {
    let f = Fixture::new();
    let crate_dir = f.project.crate_dir("tool");
    fs::create_dir_all(crate_dir.join("overrides")).unwrap();
    fs::write(
        crate_dir.join("overrides/scripted.rs"),
        crate::test_registry::marker_script(&f.tmp.path().join("markers/ours")),
    )
    .unwrap();
    let decided = f.decide("scripted", Action::Override, Some("overrides/scripted.rs"));
    apply(&decided, &crate_dir).unwrap();
    f.build();
    assert!(f.marker("ours"));
    assert!(!f.marker("upstream"));
}

#[test]
fn stripped_build_script_does_not_run() {
    let f = Fixture::new();
    let decided = f.decide("scripted", Action::Strip, None);
    apply(&decided, &f.project.crate_dir("tool")).unwrap();
    f.build();
    assert!(!f.marker("upstream"));
}

#[test]
fn settled_vendor_dir_rebuilds_nothing() {
    let mut f = Fixture::new();
    f.build();
    // Vendor afresh, as every install does, then settle.
    let args = f.registry.cargo_args();
    f.vendored = crate::vendor::vendor(&f.project, "tool", &f.checkout, false, &args).unwrap();
    crate::vendor::settle(&f.vendored).unwrap();
    let messages = f.build();
    // The tool itself is dirty anyway: its build script writes a marker into its own source.
    let rebuilt: Vec<&str> = messages
        .lines()
        .filter(|l| {
            l.contains("\"reason\":\"compiler-artifact\"")
                && l.contains("\"fresh\":false")
                && !l.contains("\"package_id\":\"path+")
        })
        .collect();
    assert!(rebuilt.is_empty(), "rebuilt: {rebuilt:#?}");
}

#[cfg(unix)]
#[test]
fn linked_build_scripts_are_refused() {
    let f = Fixture::new();
    let build = f.checkout.package_dir.join("build.rs");
    let elsewhere = f.tmp.path().join("elsewhere.rs");
    fs::rename(&build, &elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &build).unwrap();
    let selection = crate::graph::Selection {
        features: &[],
        targets: None,
        config: &[],
    };
    let err = collect("tool", selection, &f.checkout, &f.vendored, true)
        .unwrap_err()
        .to_string();
    assert!(err.contains("is a link"), "{err}");
}

#[test]
fn decisions_are_read_from_other_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let crates = tmp.path().join("crates");
    let entry = |action: &str, extra: &str| {
        format!(
            "[[build-script]]\npackage = \"ring\"\npath = \"build.rs\"\nsha256 = \"h\"\naction = \"{action}\"\n{extra}"
        )
    };
    for (tool, text) in [
        ("alpha", entry("strip", "")),
        ("beta", entry("approve", "")),
        ("me", entry("override", "file = \"overrides/mine.rs\"\n")),
        ("broken", "not toml [[".to_string()),
    ] {
        fs::create_dir_all(crates.join(tool)).unwrap();
        fs::write(crates.join(tool).join(FILE), text).unwrap();
    }
    fs::create_dir_all(crates.join("no-file")).unwrap();

    let tools = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let ring = script(tmp.path(), "ring", "h");
    // Tools that decided differently give nothing to start from.
    let known = known_elsewhere(&crates, "me", &tools(&["alpha", "beta", "me", "broken"]));
    assert_eq!(known_for(&known, &ring), None);
    // One tool's decision is used; this tool's own file and broken files aren't.
    let known = known_elsewhere(&crates, "me", &tools(&["alpha", "me", "broken"]));
    assert_eq!(
        known_for(&known, &ring),
        Some(&Known {
            tool: "alpha".into(),
            action: Action::Strip,
            file: None
        })
    );
    assert_eq!(known.len(), 1);
    // Another hash isn't the same piece.
    assert_eq!(
        known_for(&known, &script(tmp.path(), "ring", "other")),
        None
    );
    // A directory that isn't a configured tool counts for nothing.
    let known = known_elsewhere(&crates, "me", &tools(&["me"]));
    assert!(known.is_empty());
}

#[test]
fn paste_ready_entries_use_what_another_tool_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let ring = script(tmp.path(), "ring", "h");
    let strip = Known {
        tool: "alpha".into(),
        action: Action::Strip,
        file: None,
    };
    let text = paste_ready(&ring, None, Some(&strip), tmp.path());
    assert!(text.contains("# as recorded for alpha"), "{text}");
    assert_eq!(parse(&text).unwrap()[0].action, Action::Strip);

    let over = Known {
        tool: "alpha".into(),
        action: Action::Override,
        file: Some(tmp.path().join("crates/alpha/overrides/ring/build.rs")),
    };
    let text = paste_ready(&ring, None, Some(&over), tmp.path());
    assert!(
        text.contains("# copy crates/alpha/overrides/ring/build.rs to overrides/ring/build.rs"),
        "{text}"
    );
    let entry = &parse(&text).unwrap()[0];
    assert_eq!(entry.action, Action::Override);
    assert_eq!(entry.file.as_deref(), Some("overrides/ring/build.rs"));
}

#[test]
fn packages_outside_the_sources_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("src");
    fs::create_dir_all(root.join("member")).unwrap();
    fs::create_dir_all(tmp.path().join("project")).unwrap();
    let roots = [root.canonicalize().unwrap()];
    inside_sources(&root.join("member"), &roots).unwrap();
    // `..` out of the checkout.
    assert!(inside_sources(&root.join("../project"), &roots).is_err());
    // A link out of it.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tmp.path().join("project"), root.join("linked")).unwrap();
        let err = inside_sources(&root.join("linked"), &roots)
            .unwrap_err()
            .to_string();
        assert!(err.contains("outside the tool's checkout"), "{err}");
    }
}

#[test]
fn a_vendored_packages_git_dir_is_hashed() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path();
    fs::write(pkg.join("build.rs"), "fn main() {}").unwrap();
    fs::create_dir(pkg.join(".git")).unwrap();
    fs::write(pkg.join(".git/config"), "a").unwrap();
    let (vendored, ours) = (dir_hash(pkg, true).unwrap(), dir_hash(pkg, false).unwrap());
    fs::write(pkg.join(".git/config"), "b").unwrap();
    assert_ne!(dir_hash(pkg, true).unwrap(), vendored);
    // In a checkout, `.git` is our own clone.
    assert_eq!(dir_hash(pkg, false).unwrap(), ours);
}

#[test]
fn only_registry_packages_take_a_lock_checksum() {
    let sums = BTreeMap::from([(("foo".to_string(), "1.0.0".to_string()), "abc".to_string())]);
    let registry = "registry+https://github.com/rust-lang/crates.io-index";
    let git = "git+https://example.com/foo.git#0123456789abcdef0123456789abcdef01234567";
    assert_eq!(
        crate_pin(Some(registry), "foo", "1.0.0", &sums),
        Pin::Checksum("abc".into())
    );
    // A git or path package of the same name and version as a registry one in the lock.
    assert_eq!(
        crate_pin(Some(git), "foo", "1.0.0", &sums),
        Pin::Commit("0123456789abcdef0123456789abcdef01234567".into())
    );
    assert_eq!(crate_pin(None, "foo", "1.0.0", &sums), Pin::None);
}

#[test]
fn build_script_entries_carry_their_crate() {
    let parsed = parse(
        "[[build-script]]\npackage = \"a\"\npath = \"build.rs\"\nsha256 = \"h\"\nchecksum = \"c\"\naction = \"approve\"\n\n[[build-script]]\npackage = \"b\"\npath = \"build.rs\"\nsha256 = \"h\"\ncommit = \"r\"\naction = \"approve\"\n",
    )
    .unwrap();
    assert_eq!(parsed[0].pin, Pin::Checksum("c".into()));
    assert_eq!(parsed[1].pin, Pin::Commit("r".into()));
    for bad in [
        "[[build-script]]\npackage = \"a\"\npath = \"build.rs\"\nsha256 = \"h\"\nchecksum = \"c\"\ncommit = \"r\"\naction = \"approve\"\n",
        "[[proc-macro]]\npackage = \"m\"\nchecksum = \"c\"\ncommit = \"r\"\naction = \"approve\"\n",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn an_entry_for_another_crate_comes_up_as_changed() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join(FILE);
    // Recorded before entries named their crate.
    fs::write(
        &file,
        "[[build-script]]\npackage = \"libc\"\npath = \"build.rs\"\nsha256 = \"aa\"\naction = \"strip\"\n",
    )
    .unwrap();
    let item = || {
        let mut i = script(tmp.path(), "libc", "aa");
        i.pin = Pin::Checksum("c1".into());
        i
    };
    let err = check(
        &file,
        vec![item()],
        Mode::Report,
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("1 piece"), "{err}");

    let decided = check(
        &file,
        vec![item()],
        Mode::ApproveAll,
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap();
    // It keeps its action, now for this crate.
    assert_eq!(decided[0].action, Action::Strip);
    let entries = load(&file).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].pin, Pin::Checksum("c1".into()));
    check(
        &file,
        vec![item()],
        Mode::Report,
        &KnownMap::new(),
        tmp.path(),
    )
    .unwrap();

    // Another version of the crate, same script.
    let mut other = item();
    other.pin = Pin::Checksum("c2".into());
    assert!(
        check(
            &file,
            vec![other],
            Mode::Report,
            &KnownMap::new(),
            tmp.path()
        )
        .is_err()
    );
}

#[test]
fn config_that_changes_what_is_built_is_refused() {
    for (text, key) in [
        ("[source.crates-io]\nreplace-with = \"x\"\n", "source"),
        (
            "[patch.crates-io]\nserde = { path = \"../serde\" }\n",
            "patch",
        ),
        ("paths = [\"third_party/serde\"]\n", "paths"),
        (
            "[build]\ntarget = \"x86_64-pc-windows-gnu\"\n",
            "build.target",
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let checkout = checkout_at(tmp.path());
        fs::create_dir_all(checkout.package_dir.join(".cargo")).unwrap();
        fs::write(checkout.package_dir.join(".cargo/config.toml"), text).unwrap();
        let err = shipped_configs(&checkout).unwrap_err().to_string();
        assert!(err.contains(&format!("`{key}`")), "{err}");
    }
}

#[test]
fn another_tools_file_stays_in_its_comment() {
    let tmp = tempfile::tempdir().unwrap();
    let ring = script(tmp.path(), "ring", "h");
    // Written by hand, or by an attacker, past `safe_relative`.
    let forged = "x\n[[build-script]]\npackage = \"evil\"\npath = \"build.rs\"\nsha256 = \"e\"\naction = \"approve\"\n#";
    let over = Known {
        tool: "alpha".into(),
        action: Action::Override,
        file: Some(tmp.path().join(forged)),
    };
    let text = paste_ready(&ring, None, Some(&over), tmp.path());
    let entries = parse(&text).unwrap();
    assert_eq!(entries.len(), 1, "{text}");
    assert_eq!(entries[0].package, "ring");
}

#[test]
fn override_paths_with_control_characters_are_refused() {
    assert!(safe_relative("overrides/a\nb.rs").is_err());
    assert!(safe_relative("overrides/a\u{1b}[2J.rs").is_err());
    safe_relative("overrides/a/build.rs").unwrap();
}

#[test]
fn pieces_outside_the_project_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("project").join(FILE);
    let elsewhere = script(tmp.path(), "libc", "aa");
    let err = check(
        &file,
        vec![elsewhere],
        Mode::Report,
        &KnownMap::new(),
        &tmp.path().join("project"),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("outside the project"), "{err}");
}
