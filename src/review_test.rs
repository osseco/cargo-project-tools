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
    };
    let err = collect("tool", selection, &f.checkout, &f.vendored, true)
        .unwrap_err()
        .to_string();
    assert!(err.contains("is a link"), "{err}");
}
