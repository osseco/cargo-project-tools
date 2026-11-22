//! A stand-in for crates.io in tests: a directory source that cargo reads offline.

use std::fs;
use std::path::{Path, PathBuf};

/// A crate in the test registry.
pub struct Crate {
    name: String,
    version: String,
    build: Option<String>,
    proc_macro: bool,
    /// `[dependencies]` and `[build-dependencies]` lines.
    deps: Vec<String>,
    build_deps: Vec<String>,
}

impl Crate {
    pub fn new(name: &str, version: &str) -> Self {
        Crate {
            name: name.into(),
            version: version.into(),
            build: None,
            proc_macro: false,
            deps: Vec::new(),
            build_deps: Vec::new(),
        }
    }

    /// Depend on `name` at `req` (`[dependencies]`).
    pub fn dep(mut self, name: &str, req: &str) -> Self {
        self.deps.push(format!("{name} = \"{req}\""));
        self
    }

    /// Build-depend on `name` at `req` (`[build-dependencies]`).
    pub fn build_dep(mut self, name: &str, req: &str) -> Self {
        self.build_deps.push(format!("{name} = \"{req}\""));
        self
    }

    /// Give it a build script with this source.
    pub fn build(mut self, source: &str) -> Self {
        self.build = Some(source.into());
        self
    }

    pub fn proc_macro(mut self) -> Self {
        self.proc_macro = true;
        self
    }
}

pub struct Registry(tempfile::TempDir);

impl Registry {
    /// Plain library crates.
    pub fn new(crates: &[(&str, &str)]) -> Self {
        Registry::with(crates.iter().map(|(n, v)| Crate::new(n, v)).collect())
    }

    pub fn with(crates: Vec<Crate>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for c in crates {
            let root = dir.path().join(format!("{}-{}", c.name, c.version));
            fs::create_dir_all(root.join("src")).unwrap();
            let mut manifest = format!(
                "[package]\nname = \"{}\"\nversion = \"{}\"\nedition = \"2021\"\n",
                c.name, c.version
            );
            if c.build.is_some() {
                manifest.push_str("build = \"build.rs\"\n");
            }
            if c.proc_macro {
                manifest.push_str("\n[lib]\nproc-macro = true\n");
            }
            if !c.deps.is_empty() {
                manifest.push_str(&format!("\n[dependencies]\n{}\n", c.deps.join("\n")));
            }
            if !c.build_deps.is_empty() {
                manifest.push_str(&format!(
                    "\n[build-dependencies]\n{}\n",
                    c.build_deps.join("\n")
                ));
            }
            fs::write(root.join("Cargo.toml"), manifest).unwrap();
            fs::write(root.join("src/lib.rs"), "").unwrap();
            if let Some(source) = &c.build {
                fs::write(root.join("build.rs"), source).unwrap();
            }
            fs::write(
                root.join(".cargo-checksum.json"),
                r#"{"files":{},"package":null}"#,
            )
            .unwrap();
        }
        Registry(dir)
    }

    /// Make cargo resolve against this directory only, without network.
    pub fn cargo_args(&self) -> Vec<String> {
        let dir = toml_edit::Value::from(self.0.path().to_string_lossy().as_ref());
        vec![
            "--offline".into(),
            "--config".into(),
            "source.crates-io.replace-with = \"local\"".into(),
            "--config".into(),
            format!("source.local.directory = {}", dir.to_string().trim()),
        ]
    }
}

/// A build script that creates `marker` when it runs.
pub fn marker_script(marker: &Path) -> String {
    let path = marker.to_string_lossy();
    format!("fn main() {{ std::fs::write({path:?}, \"\").unwrap(); }}\n")
}

/// Like `marker_script`, and it asks to rerun when the package's `dir` changes.
pub fn watching_marker_script(marker: &Path, dir: &str) -> String {
    let path = marker.to_string_lossy();
    format!(
        "fn main() {{\n    println!(\"cargo:rerun-if-changed={dir}\");\n    std::fs::write({path:?}, \"\").unwrap();\n}}\n"
    )
}

/// Package `tool` at `version` in `dir` with registry dependencies (name, requirement),
/// and a build script that leaves a marker if it ever runs. Returns the manifest path.
pub fn tool(dir: &Path, version: &str, deps: &[(&str, &str)]) -> PathBuf {
    let deps: String = deps
        .iter()
        .map(|(n, r)| format!("{n} = \"{r}\"\n"))
        .collect();
    tool_with(dir, version, &format!("[dependencies]\n{deps}"))
}

/// Like `tool`, with `tables` (dependency tables) as written.
pub fn tool_with(dir: &Path, version: &str, tables: &str) -> PathBuf {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(dir.join("build.rs"), marker_script(&dir.join("build-ran"))).unwrap();
    let manifest = dir.join("Cargo.toml");
    fs::write(
        &manifest,
        format!(
            "[package]\nname = \"tool\"\nversion = \"{version}\"\nedition = \"2021\"\n\n{tables}\n[workspace]\n"
        ),
    )
    .unwrap();
    manifest
}
