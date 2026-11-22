use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::process;

pub const CONFIG_FILE: &str = "project-tools.toml";
pub const TOOLS_DIR: &str = ".project-tools";

/// Entries `.project-tools/.gitignore` must contain.
const IGNORED: &[&str] = &[
    "/bin/",
    "/targets/",
    "/advisory-db/",
    "/advisory-db.lock",
    // Left behind if a database download is interrupted.
    "/.advisory-db-*/",
    "/.crates.toml",
    "/.crates2.json",
];

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
}

impl Project {
    /// Find the nearest ancestor of the current directory containing `project-tools.toml`.
    pub fn discover() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        for dir in cwd.ancestors() {
            if dir.join(CONFIG_FILE).is_file() {
                let project = Project {
                    root: dir.to_path_buf(),
                };
                project.ensure_layout()?;
                return Ok(project);
            }
        }
        bail!(
            "no {CONFIG_FILE} found in {} or its parents; run `cargo project-tools init`",
            cwd.display()
        )
    }

    /// Project root for `init`: the cargo workspace root, which must be inside a git repository.
    pub fn for_init() -> Result<Self> {
        let manifest = process::output(process::cargo().args([
            "locate-project",
            "--workspace",
            "--message-format",
            "plain",
        ]))
        .context("not inside a cargo project")?;
        let root = Path::new(manifest.trim())
            .parent()
            .context("unexpected manifest path")?
            .to_path_buf();
        process::output(
            std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["rev-parse", "--show-toplevel"]),
        )
        .context("project is not inside a git repository")?;
        Ok(Project { root })
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG_FILE)
    }

    pub fn tools_dir(&self) -> PathBuf {
        self.root.join(TOOLS_DIR)
    }

    /// `--root` for cargo install; binaries land in `<install_root>/bin`.
    pub fn install_root(&self) -> PathBuf {
        self.tools_dir()
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.tools_dir().join("bin")
    }

    /// What `name` was last installed from; next to the binaries, so restoring `bin` from a
    /// cache is enough to skip a build.
    pub fn stamp_path(&self, name: &str) -> PathBuf {
        self.bin_dir().join(".stamps").join(format!("{name}.json"))
    }

    /// The packages `name` builds, remembered next to its stamp (`graph::built_packages`).
    pub fn built_path(&self, name: &str) -> PathBuf {
        self.bin_dir()
            .join(".stamps")
            .join(format!("{name}.built.json"))
    }

    pub fn crate_dir(&self, name: &str) -> PathBuf {
        self.tools_dir().join("crates").join(name)
    }

    pub fn lock_path(&self, name: &str) -> PathBuf {
        self.crate_dir(name).join("Cargo.lock")
    }

    pub fn audit_ignore_path(&self, name: &str) -> PathBuf {
        self.crate_dir(name).join("audit-ignore.toml")
    }

    pub fn review_path(&self, name: &str) -> PathBuf {
        self.crate_dir(name).join(crate::review::FILE)
    }

    pub fn self_audit_ignore_path(&self) -> PathBuf {
        self.tools_dir().join(crate::config::SELF_AUDIT_IGNORE)
    }

    pub fn target_dir(&self, name: &str) -> PathBuf {
        self.tools_dir().join("targets").join(name)
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.tools_dir().join("targets").join(".cache")
    }

    pub fn tmp_dir(&self) -> Result<PathBuf> {
        let dir = self.tools_dir().join("targets").join(".tmp");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn advisory_db(&self) -> PathBuf {
        self.tools_dir().join("advisory-db")
    }

    /// Create missing directories and make sure the `.gitignore` has every required entry.
    pub fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.tools_dir().join("crates"))?;
        let path = self.tools_dir().join(".gitignore");
        let current = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let updated = with_ignored_entries(&current);
        if updated != current {
            fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
        }
        Ok(())
    }
}

fn with_ignored_entries(current: &str) -> String {
    let present: Vec<&str> = current.lines().map(str::trim).collect();
    let mut out = current.to_string();
    for entry in IGNORED {
        if !present.contains(entry) {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(entry);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
#[path = "project_test.rs"]
mod tests;
