//! Pending pieces on disk for the review screen's tests.

use std::fs;
use std::path::Path;

use crate::review::{Item, Kind};

pub(super) fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A build script at its package root.
pub(super) fn root_script(dir: &Path, package: &str, text: &str) -> Item {
    let base = dir.join("vendor").join(package);
    write(&base.join("build.rs"), text);
    Item {
        kind: Kind::BuildScript,
        package: package.into(),
        version: "1.0.0".into(),
        path: "build.rs".into(),
        hash: "h".into(),
        location: base.join("build.rs"),
        base,
        vendored: true,
    }
}

/// A build script in a subdirectory with a helper module.
pub(super) fn dir_script(dir: &Path, package: &str) -> Item {
    let base = dir.join("vendor").join(package);
    write(&base.join("build/main.rs"), "mod helper;\nfn main() {}\n");
    write(&base.join("build/helper.rs"), "pub fn help() {}\n");
    Item {
        kind: Kind::BuildScript,
        package: package.into(),
        version: "1.0.0".into(),
        path: "build/main.rs".into(),
        hash: "h".into(),
        location: base.join("build/main.rs"),
        base,
        vendored: true,
    }
}

pub(super) fn proc_macro(dir: &Path, package: &str) -> Item {
    let base = dir.join("vendor").join(package);
    write(&base.join("Cargo.toml"), "[package]\nname = \"m\"\n");
    write(&base.join("src/lib.rs"), "pub fn m() {}\n");
    write(&base.join(".git/HEAD"), "ref: x\n");
    Item {
        kind: Kind::ProcMacro,
        package: package.into(),
        version: "1.0.0".into(),
        path: String::new(),
        hash: "c".into(),
        location: base.clone(),
        base,
        vendored: true,
    }
}

/// Cargo config shipped in the tool's checkout.
pub(super) fn cargo_config(dir: &Path, text: &str) -> Item {
    let root = dir.join("checkout");
    write(&root.join(".cargo/config.toml"), text);
    Item {
        kind: Kind::CargoConfig,
        package: String::new(),
        version: String::new(),
        path: ".cargo/config.toml".into(),
        hash: "h".into(),
        location: root.join(".cargo/config.toml"),
        base: root,
        vendored: false,
    }
}
