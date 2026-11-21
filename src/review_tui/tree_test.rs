use super::*;

use crate::review::tui::fixtures::{cargo_config, dir_script, proc_macro, root_script};

fn sample(dir: &Path) -> Vec<Node> {
    build(&[
        root_script(dir, "a", "fn main() {}\n"),
        dir_script(dir, "b"),
        proc_macro(dir, "m"),
        cargo_config(dir, "[build]\n"),
    ])
    .unwrap()
}

#[test]
fn tree_holds_pieces_and_their_files() {
    let tmp = tempfile::tempdir().unwrap();
    let nodes = sample(tmp.path());
    let labels: Vec<(usize, &str)> = nodes.iter().map(|n| (n.depth, n.label.as_str())).collect();
    assert_eq!(
        labels,
        [
            (0, "build-script"),
            (1, "a@1.0.0 build.rs"),
            (1, "b@1.0.0 build/main.rs"),
            (2, "helper.rs"),
            (2, "main.rs  ← build script"),
            (0, "proc-macro"),
            (1, "m@1.0.0"),
            (2, "src/"),
            (3, "lib.rs"),
            (2, "Cargo.toml"),
            (0, "cargo-config"),
            (1, ".cargo/config.toml"),
        ]
    );
    assert!(matches!(nodes[1].kind, NodeKind::ItemFile(_)));
    assert_eq!(nodes[2].kind, NodeKind::ItemDir);
    assert!(matches!(nodes[11].kind, NodeKind::ItemFile(_)));
    // Every node under a piece knows it.
    assert_eq!(nodes[8].item, Some(2));
}

#[test]
fn folded_nodes_hide_what_is_below_them() {
    let tmp = tempfile::tempdir().unwrap();
    let mut nodes = sample(tmp.path());
    // Groups start open, pieces and directories folded.
    assert_eq!(visible(&nodes), [0, 1, 2, 5, 6, 10, 11]);
    nodes[6].expanded = true;
    assert_eq!(visible(&nodes), [0, 1, 2, 5, 6, 7, 9, 10, 11]);
    nodes[7].expanded = true;
    assert_eq!(visible(&nodes), [0, 1, 2, 5, 6, 7, 8, 9, 10, 11]);
    nodes[5].expanded = false;
    assert_eq!(visible(&nodes), [0, 1, 2, 5, 10, 11]);
}

#[test]
fn parents() {
    let tmp = tempfile::tempdir().unwrap();
    let nodes = sample(tmp.path());
    assert_eq!(parent(&nodes, 8), Some(7));
    assert_eq!(parent(&nodes, 9), Some(6));
    assert_eq!(parent(&nodes, 6), Some(5));
    assert_eq!(parent(&nodes, 5), None);
}

#[test]
fn names_and_link_targets_are_sanitized() {
    let tmp = tempfile::tempdir().unwrap();
    let item = proc_macro(tmp.path(), "m");
    fs::write(item.location.join("evil\u{202e}sr.txt"), "x").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/\x1b[2Jpasswd", item.location.join("link")).unwrap();
    let nodes = build(&[item]).unwrap();
    let labels: Vec<&str> = nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"evil<U+202E>sr.txt"), "{labels:?}");
    #[cfg(unix)]
    assert!(
        labels.contains(&"link -> /etc/\\x1b[2Jpasswd"),
        "{labels:?}"
    );
    assert!(
        nodes
            .iter()
            .all(|n| !n.label.contains(['\u{202e}', '\x1b']))
    );
}

#[test]
fn a_script_that_pulls_in_files_is_a_folder_of_them() {
    let tmp = tempfile::tempdir().unwrap();
    let mut item = root_script(tmp.path(), "a", "mod helper;");
    crate::review::tui::fixtures::write(&item.base.join("helper.rs"), "");
    item.pulled.files = vec![item.base.join("helper.rs")];
    let mut whole = root_script(tmp.path(), "w", "x");
    crate::review::tui::fixtures::write(&whole.base.join("src/lib.rs"), "");
    whole.pulled.whole_package = Some("build.rs includes a file by a computed path".into());

    let nodes = build(&[item, whole]).unwrap();
    let labels: Vec<(usize, &str)> = nodes.iter().map(|n| (n.depth, n.label.as_str())).collect();
    assert_eq!(
        labels,
        [
            (0, "build-script"),
            (1, "a@1.0.0 build.rs"),
            (2, "build.rs  ← build script"),
            (2, "helper.rs"),
            (1, "w@1.0.0 build.rs"),
            (2, "src/"),
            (3, "lib.rs"),
            (2, "build.rs  ← build script"),
        ]
    );
    assert_eq!(nodes[1].kind, NodeKind::ItemDir);
}
