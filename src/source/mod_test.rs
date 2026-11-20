use super::*;

#[test]
fn stubs_cover_declared_targets() {
    let paths = stub_paths(
        r#"
        [package]
        name = "x"
        build = "build.rs"
        [lib]
        path = "src/lib.rs"
        [[bin]]
        name = "x"
        path = "src/bin/x.rs"
        "#,
    )
    .unwrap();
    assert!(paths.contains(&PathBuf::from("src/bin/x.rs")));
    assert!(paths.contains(&PathBuf::from("build.rs")));
}

fn member_checkout(root: &Path) -> Checkout {
    Checkout {
        root: root.to_path_buf(),
        workspace_root: root.join("ws"),
        package_dir: root.join("ws/crates/tool"),
        version: "1.0.0".into(),
        commit: None,
        _tmp: None,
    }
}

#[test]
fn search_dirs_stop_at_checkout_root() {
    let root = Path::new("/x/src");
    let dirs = search_dirs(&member_checkout(root)).unwrap();
    assert_eq!(
        dirs,
        [
            root.join("ws/crates/tool"),
            root.join("ws/crates"),
            root.join("ws"),
            root.to_path_buf(),
        ]
    );
}

#[test]
fn search_dirs_never_leave_the_checkout() {
    // A package path spelled differently from the root (e.g. through a link) must not walk
    // up into the user's directories.
    let mut checkout = member_checkout(Path::new("/x/src"));
    checkout.package_dir = PathBuf::from("/private/x/src/ws/crates/tool");
    assert!(search_dirs(&checkout).is_err());
    assert!(strip_toolchain(&checkout).is_err());
}

#[test]
fn toolchain_files_are_removed_along_the_search_path() {
    let tmp = tempfile::tempdir().unwrap();
    let checkout = member_checkout(tmp.path());
    fs::create_dir_all(&checkout.package_dir).unwrap();
    fs::write(checkout.package_dir.join("rust-toolchain.toml"), "").unwrap();
    fs::write(checkout.workspace_root.join("rust-toolchain"), "").unwrap();
    fs::write(tmp.path().join("rust-toolchain.toml"), "").unwrap();
    fs::write(checkout.package_dir.join("keep.toml"), "").unwrap();

    strip_toolchain(&checkout).unwrap();

    assert!(!checkout.package_dir.join("rust-toolchain.toml").exists());
    assert!(!checkout.workspace_root.join("rust-toolchain").exists());
    assert!(!tmp.path().join("rust-toolchain.toml").exists());
    assert!(checkout.package_dir.join("keep.toml").exists());
}

#[test]
fn stubs_reject_escaping_paths() {
    assert!(stub_paths("[[bin]]\nname = \"x\"\npath = \"../x.rs\"\n").is_err());
}
