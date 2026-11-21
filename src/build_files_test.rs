use super::*;

/// A package in a temporary directory with these files.
fn package(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = tmp.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    tmp
}

/// The pulled-in files, relative to the package.
fn found(pkg: &Path, script: &str) -> (Vec<String>, Option<String>) {
    let pulled = find(pkg, &pkg.join(script)).unwrap();
    let mut files: Vec<String> = pulled
        .files
        .iter()
        .map(|f| {
            f.strip_prefix(pkg)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    files.sort();
    (files, pulled.whole_package)
}

#[test]
fn a_plain_script_pulls_in_nothing() {
    let pkg = package(&[
        ("build.rs", "fn main() { println!(\"x\"); }"),
        ("src/lib.rs", ""),
    ]);
    assert_eq!(found(pkg.path(), "build.rs"), (vec![], None));
}

#[test]
fn modules_paths_and_includes_are_followed() {
    let pkg = package(&[
        (
            "build.rs",
            r#"
            mod helper;
            #[path = "tools/gen.rs"]
            mod gen;
            #[cfg_attr(windows, path = "win.rs")]
            mod platform;
            const TABLE: &str = include_str!("data/table.txt");
            const BYTES: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/blob.bin"));
            fn main() {
                include!("snippet.rs");
                println!("{}", include_str!("in_macro.txt"));
                let _ = include!(concat!(env!("OUT_DIR"), "/generated.rs"));
            }
            "#,
        ),
        ("helper.rs", "mod inner;"),
        ("helper/inner.rs", "pub fn f() {}"),
        ("tools/gen.rs", "mod util;"),
        ("tools/util.rs", ""),
        ("win.rs", ""),
        ("platform.rs", ""),
        ("data/table.txt", "a,b"),
        ("data/blob.bin", "\0\0"),
        ("snippet.rs", "{ let x = include_str!(\"deeper.txt\"); }"),
        ("deeper.txt", "x"),
        ("in_macro.txt", "x"),
        ("src/lib.rs", "never read"),
    ]);
    let (files, whole) = found(pkg.path(), "build.rs");
    assert_eq!(whole, None);
    assert_eq!(
        files,
        [
            "data/blob.bin",
            "data/table.txt",
            "deeper.txt",
            "helper.rs",
            "helper/inner.rs",
            "in_macro.txt",
            "platform.rs",
            "snippet.rs",
            "tools/gen.rs",
            "tools/util.rs",
            "win.rs",
        ]
    );
}

#[test]
fn mod_rs_and_inline_modules() {
    let pkg = package(&[
        (
            "build/main.rs",
            "mod a; mod inline { mod b; #[path = \"x.rs\"] mod c; }",
        ),
        ("build/a/mod.rs", "mod nested;"),
        ("build/a/nested.rs", ""),
        ("build/inline/b.rs", ""),
        ("build/inline/x.rs", ""),
    ]);
    let (files, whole) = found(pkg.path(), "build/main.rs");
    assert_eq!(whole, None);
    assert_eq!(
        files,
        [
            "build/a/mod.rs",
            "build/a/nested.rs",
            "build/inline/b.rs",
            "build/inline/x.rs",
        ]
    );
}

#[test]
fn missing_files_are_left_out() {
    // A cfg'd-out module or include doesn't exist and can't run.
    let pkg = package(&[("build.rs", "#[cfg(windows)] mod win; fn main() {}")]);
    assert_eq!(found(pkg.path(), "build.rs"), (vec![], None));
}

#[test]
fn what_cannot_be_followed_covers_the_whole_package() {
    for (text, why) in [
        (
            "fn main() { include!(concat!(env!(\"HOME\"), \"/x.rs\")); }",
            "computed path",
        ),
        ("fn main() { include!(PATH); }", "computed path"),
        ("fn main( {", "can't be read"),
        (
            "cfg_if::cfg_if! { if #[cfg(unix)] { mod unix; } }",
            "can't be followed",
        ),
        ("macro_rules! m { () => { mod x; } }", "can't be followed"),
        ("fn main() { mod inner { mod deep; } }", "can't be followed"),
        ("#[path = concat!(\"a\", \".rs\")] mod a;", "plain string"),
    ] {
        let pkg = package(&[("build.rs", text), ("unix.rs", ""), ("x.rs", "")]);
        let (files, whole) = found(pkg.path(), "build.rs");
        let whole = whole.unwrap_or_else(|| panic!("followed: {text}"));
        assert!(whole.contains(why), "{text}: {whole}");
        assert!(whole.contains("build.rs"), "{whole}");
        let _ = files;
    }
}

#[test]
fn files_outside_the_package_are_refused() {
    let tmp = package(&[
        (
            "pkg/build.rs",
            "const X: &str = include_str!(\"../secret.txt\");",
        ),
        ("secret.txt", "x"),
    ]);
    let pkg = tmp.path().join("pkg");
    let err = find(&pkg, &pkg.join("build.rs")).unwrap_err();
    assert!(
        format!("{err:#}").contains("outside its package"),
        "{err:#}"
    );
}

#[cfg(unix)]
#[test]
fn links_are_refused() {
    let pkg = package(&[("build.rs", "mod helper;"), ("real.rs", "")]);
    std::os::unix::fs::symlink(pkg.path().join("real.rs"), pkg.path().join("helper.rs")).unwrap();
    let err = find(pkg.path(), &pkg.path().join("build.rs")).unwrap_err();
    assert!(format!("{err:#}").contains("is a link"), "{err:#}");
}

#[test]
fn include_loops_end() {
    let pkg = package(&[
        ("build.rs", "fn main() { include!(\"a.rs\"); }"),
        ("a.rs", "{ include!(\"b.rs\") }"),
        ("b.rs", "{ include!(\"a.rs\") }"),
    ]);
    let (files, whole) = found(pkg.path(), "build.rs");
    assert_eq!(whole, None);
    assert_eq!(files, ["a.rs", "b.rs"]);
}
