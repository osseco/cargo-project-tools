use super::*;

use crate::review::{Action, review_item};

fn script_in(base: &Path, text: &str) -> Item {
    fs::create_dir_all(base).unwrap();
    fs::write(base.join("build.rs"), text).unwrap();
    Item {
        kind: Kind::BuildScript,
        package: "zstd-sys".into(),
        version: "2.0.0".into(),
        path: "build.rs".into(),
        hash: "new".into(),
        pin: Pin::Checksum("new-crate".into()),
        location: base.join("build.rs"),
        base: base.to_path_buf(),
        vendored: true,
        pulled: build_files::Pulled::default(),
        source: Some("registry+https://github.com/rust-lang/crates.io-index".into()),
        old: None,
    }
}

fn entry(item: &Item, hash: &str, pin: Pin) -> Entry {
    Entry {
        kind: item.kind,
        package: item.package.clone(),
        path: item.path.clone(),
        hash: hash.into(),
        pin,
        action: Action::Approve,
        file: None,
        line: None,
    }
}

#[test]
fn git_url_drops_the_reference_and_commit() {
    assert_eq!(
        git_url("git+https://github.com/cross-rs/cross?rev=abc#0123").as_deref(),
        Some("https://github.com/cross-rs/cross")
    );
    assert_eq!(
        git_url("git+https://example.com/r.git#0123").as_deref(),
        Some("https://example.com/r.git")
    );
    assert_eq!(git_url("registry+https://x"), None);
}

#[test]
fn the_old_crate_is_the_one_the_entry_names() {
    let tmp = tempfile::tempdir().unwrap();
    let item = script_in(tmp.path(), "fn main() {}\n");
    assert_eq!(
        old_crate(
            &item,
            &entry(&item, "old", Pin::Checksum("old-crate".into()))
        ),
        Some(Pin::Checksum("old-crate".into()))
    );
    // Same crate, or nothing recorded about it: nothing older to fetch.
    assert_eq!(
        old_crate(
            &item,
            &entry(&item, "old", Pin::Checksum("new-crate".into()))
        ),
        None
    );
    assert_eq!(old_crate(&item, &entry(&item, "old", Pin::None)), None);

    let mut mac = item.clone();
    mac.kind = Kind::ProcMacro;
    mac.hash = "new-sum".into();
    assert_eq!(
        old_crate(&mac, &entry(&mac, "old-sum", Pin::None)),
        Some(Pin::Checksum("old-sum".into()))
    );
    // A proc macro of the tool's own workspace is pinned by its files.
    mac.source = None;
    assert_eq!(old_crate(&mac, &entry(&mac, "old-sum", Pin::None)), None);
}

#[test]
fn diff_shows_what_changed_in_the_covered_files() {
    let tmp = tempfile::tempdir().unwrap();
    let old_base = tmp.path().join("old");
    script_in(&old_base, "fn main() {\n    old();\n}\n");
    let mut item = script_in(&tmp.path().join("new"), "fn main() {\n    new();\n}\n");
    fs::write(item.base.join("helper.rs"), "pub fn new() {}\n").unwrap();
    item.pulled.files = vec![item.base.join("helper.rs")];
    let old = OldCopy {
        version: "1.0.0".into(),
        base: old_base,
    };

    let text = diff(&item, &old).unwrap();
    assert!(text.contains("--- a/build.rs\n+++ b/build.rs\n"), "{text}");
    assert!(text.contains("-    old();\n+    new();\n"), "{text}");
    assert!(text.contains("--- /dev/null\n+++ b/helper.rs\n"), "{text}");
}

#[test]
fn records_point_at_the_previous_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let mut item = script_in(tmp.path(), "fn main() {}\n");
    item.old = Some(OldCopy {
        version: "1.0.0".into(),
        base: tmp.path().join("previous/zstd-sys-1.0.0"),
    });
    let previous = entry(&item, "old", Pin::Checksum("old-crate".into()));
    let record = review_item(
        &item,
        Some(&previous),
        None,
        "v".into(),
        Some(".project-tools/targets/t/previous/zstd-sys-1.0.0/build.rs".into()),
    );
    let p = record.previous.unwrap();
    assert_eq!(p.version.as_deref(), Some("1.0.0"));
    assert_eq!(
        p.location.as_deref(),
        Some(".project-tools/targets/t/previous/zstd-sys-1.0.0/build.rs")
    );
}

#[test]
fn nothing_is_fetched_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let project = Project {
        root: tmp.path().to_path_buf(),
    };
    let checkout = Checkout::at(
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
        tmp.path().to_path_buf(),
    );
    let mut items = vec![script_in(&tmp.path().join("p"), "fn main() {}\n")];
    fetch(&project, "t", &checkout, &mut items, true).unwrap();
    assert_eq!(items[0].old, None);
}
