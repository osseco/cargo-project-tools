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

#[test]
fn stubs_reject_escaping_paths() {
    assert!(stub_paths("[[bin]]\nname = \"x\"\npath = \"../x.rs\"\n").is_err());
}
