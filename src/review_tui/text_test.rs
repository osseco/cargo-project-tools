use super::*;

use crate::review::tui::fixtures::{cargo_config, root_script};

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
