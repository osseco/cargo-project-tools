use super::*;

use crate::review::tui::fixtures::{cargo_config, root_script};

#[test]
fn control_characters_become_hex_escapes() {
    let (text, n) = sanitize("a\x1b]52;c;aGk=\x07b");
    assert_eq!(text, "a\\x1b]52;c;aGk=\\x07b");
    assert_eq!(n, 2);
    let (text, n) = sanitize("cr\r c1\u{85} del\x7f");
    assert_eq!(text, "cr\\x0d c1\\x85 del\\x7f");
    assert_eq!(n, 3);
}

#[test]
fn crlf_is_a_line_ending_and_a_lone_cr_is_not() {
    // As in generic-array 0.14.7's build.rs.
    let crlf = "fn main() {\r\n    println!(\"x\");\r\n}\r\n";
    assert_eq!(
        sanitize(crlf),
        ("fn main() {\n    println!(\"x\");\n}\n".to_string(), 0)
    );
    // A `\r` that would draw over its line, and one at the very end.
    let (text, n) = sanitize("safe();\rEVIL\r\n\r");
    assert_eq!(text, "safe();\\x0dEVIL\n\\x0d");
    assert_eq!(n, 2);
}

#[test]
fn characters_that_hide_or_disguise_text_become_code_points() {
    for (c, name) in [
        ('\u{202e}', "bidi override"),
        ('\u{2066}', "bidi isolate"),
        ('\u{200b}', "zero-width space"),
        ('\u{feff}', "byte order mark"),
        ('\u{00ad}', "soft hyphen"),
        ('\u{e0041}', "tag character"),
        ('\u{2028}', "line separator"),
        ('\u{00a0}', "no-break space"),
        ('\u{3000}', "ideographic space"),
        ('\u{3164}', "hangul filler"),
        ('\u{2800}', "braille blank"),
        ('\u{034f}', "combining grapheme joiner"),
        ('\u{fe0f}', "variation selector"),
        ('\u{e0100}', "variation selector supplement"),
        ('\u{e000}', "private use"),
        ('\u{10fffd}', "private use, plane 16"),
        ('\u{fdd0}', "noncharacter"),
        ('\u{1fffe}', "noncharacter"),
    ] {
        let (text, n) = sanitize(&format!("a{c}b"));
        assert_eq!(text, format!("a<U+{:04X}>b", c as u32), "{name}");
        assert_eq!(n, 1, "{name}");
    }
}

#[test]
fn ordinary_text_is_left_alone() {
    let plain = "fn main() {\n\tprintln!(\"åäö 日本 Ωμέγα é 🦀\");\n}\n";
    assert_eq!(sanitize(plain), (plain.to_string(), 0));
}

#[test]
fn commented_copy_comments_out_every_line_under_the_header() {
    let copy = commented_copy(b"fn main() {\n}", true);
    assert_eq!(
        String::from_utf8(copy).unwrap(),
        format!("// {UNREVIEWED}\n// fn main() {{\n// }}\n")
    );
    let copy = commented_copy(b"[build]\n", false);
    assert_eq!(
        String::from_utf8(copy).unwrap(),
        format!("# {UNREVIEWED}\n# [build]\n")
    );
}

#[test]
fn overrides_go_under_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        override_rel(&root_script(tmp.path(), "ring", "x")).unwrap(),
        "overrides/ring/build.rs"
    );
    assert_eq!(
        override_rel(&cargo_config(tmp.path(), "x")).unwrap(),
        "overrides/cargo-config/.cargo/config.toml"
    );
}

#[test]
fn write_override_creates_directories_and_replaces_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("crates/tool");
    let dest = write_override(&crate_dir, "overrides/ring/build.rs", b"one").unwrap();
    assert_eq!(dest, crate_dir.join("overrides/ring/build.rs"));
    write_override(&crate_dir, "overrides/ring/build.rs", b"two").unwrap();
    assert_eq!(fs::read_to_string(&dest).unwrap(), "two");
    // No temporary files left behind.
    let names: Vec<_> = fs::read_dir(dest.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["build.rs"]);
}

#[test]
fn write_override_refuses_unsafe_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("crate");
    for rel in ["../x.rs", "/abs.rs", "overrides/../../x.rs", ""] {
        assert!(write_override(&crate_dir, rel, b"x").is_err(), "{rel}");
    }
    assert!(!tmp.path().join("x.rs").exists());
}

#[cfg(unix)]
#[test]
fn write_override_follows_no_links() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let elsewhere = tmp.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();

    // A linked directory on the way.
    let crate_dir = tmp.path().join("a");
    fs::create_dir_all(&crate_dir).unwrap();
    symlink(&elsewhere, crate_dir.join("overrides")).unwrap();
    let err = write_override(&crate_dir, "overrides/ring/build.rs", b"x").unwrap_err();
    assert!(format!("{err:#}").contains("is a link"), "{err:#}");

    // The file itself.
    let crate_dir = tmp.path().join("b");
    fs::create_dir_all(crate_dir.join("overrides/ring")).unwrap();
    symlink(
        elsewhere.join("target.rs"),
        crate_dir.join("overrides/ring/build.rs"),
    )
    .unwrap();
    assert!(write_override(&crate_dir, "overrides/ring/build.rs", b"x").is_err());

    // The crate directory.
    let crate_dir = tmp.path().join("c");
    symlink(&elsewhere, &crate_dir).unwrap();
    assert!(write_override(&crate_dir, "overrides/ring/build.rs", b"x").is_err());

    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}
