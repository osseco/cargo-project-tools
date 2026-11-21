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
fn missing_files_cover_the_whole_package() {
    // rustc might find them some other way than we look for them.
    for text in [
        "#[cfg(windows)] mod win; fn main() {}",
        "#[path = \"gone.rs\"] mod gone;",
        "fn main() { include!(\"gone.rs\"); }",
    ] {
        let pkg = package(&[("build.rs", text)]);
        let (_, whole) = found(pkg.path(), "build.rs");
        assert!(whole.is_some_and(|w| w.contains("isn't there")), "{text}");
    }
}

#[test]
fn includes_named_some_other_way_cover_the_whole_package() {
    for (text, why) in [
        ("fn main() { r#include!(\"payload.rs\"); }", ""),
        (
            "use std::include as inc; fn main() { inc!(\"payload.rs\"); }",
            "renames",
        ),
        (
            "macro_rules! m { () => { include!(\"payload.rs\") } } fn main() { m!(); }",
            "inside `macro_rules!`",
        ),
        (
            "macro_rules! call { ($m:ident) => { $m!(\"payload.rs\") } } fn main() { call!(include); }",
            "passed in",
        ),
        (
            "#[cfg_attr(all(), cfg_attr(all(), path = \"payload.rs\"))] mod x;",
            "can't be read for sure",
        ),
        (
            "fn main() { include!(\"inc.rs\"); }",
            "declares a module file",
        ),
    ] {
        let pkg = package(&[
            ("build.rs", text),
            ("payload.rs", ""),
            ("x.rs", ""),
            ("inc.rs", "mod x;"),
        ]);
        let (files, whole) = found(pkg.path(), "build.rs");
        // `r#include!` is simply followed, as rustc does.
        if why.is_empty() {
            assert_eq!(files, ["payload.rs"], "{text}");
            continue;
        }
        let whole = whole.unwrap_or_else(|| panic!("followed: {text}"));
        assert!(whole.contains(why), "{text}: {whole}");
    }
}

#[test]
fn only_plain_out_dir_paths_count_as_generated() {
    // Written by the script itself: not followed.
    let pkg = package(&[(
        "build.rs",
        "fn main() { include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\")); }",
    )]);
    assert_eq!(found(pkg.path(), "build.rs"), (vec![], None));
    for text in [
        // Climbs out of OUT_DIR into the sources.
        "fn main() { include!(concat!(env!(\"OUT_DIR\"), \"/../../../src/payload.rs\")); }",
        // A decoy mention.
        "macro_rules! e { ($x:tt) => { \"\" } } fn main() { include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/payload.rs\", e!(\"OUT_DIR\"))); }",
    ] {
        let pkg = package(&[("build.rs", text), ("payload.rs", "")]);
        let (_, whole) = found(pkg.path(), "build.rs");
        assert!(whole.is_some(), "followed: {text}");
    }
}

#[test]
fn an_included_file_may_have_inline_modules() {
    let pkg = package(&[
        ("build.rs", "mod triple { include!(\"src/triple.rs\"); }"),
        (
            "src/triple.rs",
            "pub fn f() {} #[cfg(test)] mod tests { fn t() {} }",
        ),
    ]);
    assert_eq!(
        found(pkg.path(), "build.rs"),
        (vec!["src/triple.rs".into()], None)
    );
}

#[test]
fn other_uses_of_the_name_are_fine() {
    let pkg = package(&[(
        "build.rs",
        "fn main() { cc::Build::new().include(\"src\").compile(\"x\"); let include = 1; }",
    )]);
    assert_eq!(found(pkg.path(), "build.rs"), (vec![], None));
}

#[test]
fn files_in_dot_git_are_refused() {
    let pkg = package(&[
        ("build.rs", "#[path = \".git/x.rs\"] mod x;"),
        (".git/x.rs", ""),
    ]);
    let err = find(pkg.path(), &pkg.path().join("build.rs")).unwrap_err();
    assert!(format!("{err:#}").contains("inside .git"), "{err:#}");
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
        ("#[path = concat!(\"a\", \".rs\")] mod a;", "read for sure"),
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
