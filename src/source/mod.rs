pub mod git;
pub mod registry;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::Tool;
use crate::project::Project;

/// Where a tool comes from, independent of `project-tools.toml`.
#[derive(Debug, Clone)]
pub enum Origin {
    Registry { version_req: Option<String> },
    Git { url: String, reference: git::GitRef },
}

impl Origin {
    pub fn of(tool: &Tool) -> Origin {
        match &tool.git {
            Some(url) => Origin::Git {
                url: url.clone(),
                reference: git::GitRef {
                    branch: tool.branch.clone(),
                    tag: tool.tag.clone(),
                    rev: tool.rev.clone(),
                },
            },
            None => Origin::Registry {
                version_req: Some(tool.version.clone()),
            },
        }
    }
}

/// A tool's source on disk.
pub struct Checkout {
    pub workspace_root: PathBuf,
    pub package_dir: PathBuf,
    pub version: String,
    /// Resolved git commit, for git origins.
    pub commit: Option<String>,
    _tmp: Option<tempfile::TempDir>,
}

impl Checkout {
    pub fn lock_path(&self) -> PathBuf {
        self.workspace_root.join("Cargo.lock")
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.package_dir.join("Cargo.toml")
    }

    /// Read the lock the source ships with, if any.
    pub fn shipped_lock(&self) -> Result<Option<String>> {
        match fs::read_to_string(self.lock_path()) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn put_lock(&self, lock: &Path) -> Result<()> {
        fs::copy(lock, self.lock_path())
            .with_context(|| format!("copying {} into source", lock.display()))?;
        Ok(())
    }
}

pub enum Dest {
    /// A fresh temporary directory under `.project-tools/targets/.tmp`.
    Temp,
    /// A fixed directory (replaced), so cargo's fingerprints stay valid between builds.
    Fixed(PathBuf),
}

fn dest_dir(project: &Project, dest: Dest) -> Result<(PathBuf, Option<tempfile::TempDir>)> {
    Ok(match dest {
        Dest::Fixed(p) => (p, None),
        Dest::Temp => {
            let tmp = tempfile::Builder::new()
                .prefix("src-")
                .tempdir_in(project.tmp_dir()?)?;
            (tmp.path().join("src"), Some(tmp))
        }
    })
}

/// Resolve the registry version for `name` given an optional requirement.
pub fn resolve_registry(name: &str, req: Option<&str>) -> Result<registry::IndexEntry> {
    let entries = registry::fetch_index(name)?;
    let req = req.map(registry::parse_req).transpose()?;
    registry::select(&entries, req.as_ref())
        .cloned()
        .with_context(|| match &req {
            Some(r) => format!("no published version of `{name}` matches `{r}`"),
            None => format!("no stable published version of `{name}`"),
        })
}

/// Full source of the tool, ready to build.
pub fn checkout(project: &Project, name: &str, origin: &Origin, dest: Dest) -> Result<Checkout> {
    let (dir, tmp) = dest_dir(project, dest)?;
    let checkout = match origin {
        Origin::Registry { version_req } => {
            let entry = resolve_registry(name, version_req.as_deref())?;
            let file = registry::download(&entry, &project.cache_dir())?;
            registry::extract(&file, &entry, &dir)?;
            Checkout {
                workspace_root: dir.clone(),
                package_dir: dir,
                version: entry.vers.to_string(),
                commit: None,
                _tmp: tmp,
            }
        }
        Origin::Git { url, reference } => {
            let c = git::clone(url, reference, name, &dir)?;
            Checkout {
                workspace_root: c.workspace_root,
                package_dir: c.package_dir,
                version: c.version,
                commit: Some(c.commit),
                _tmp: tmp,
            }
        }
    };
    isolate_workspace(&checkout)?;
    Ok(checkout)
}

/// Minimal project for re-resolving a lock: for registry tools only the manifest plus empty
/// stubs for its targets; git tools use a full clone (workspaces may need path dependencies).
pub fn scratch(project: &Project, name: &str, origin: &Origin) -> Result<Checkout> {
    let Origin::Registry { version_req } = origin else {
        return checkout(project, name, origin, Dest::Temp);
    };
    let (dir, tmp) = dest_dir(project, Dest::Temp)?;
    let entry = resolve_registry(name, version_req.as_deref())?;
    let file = registry::download(&entry, &project.cache_dir())?;
    let manifest = registry::read_file(&file, &entry, "Cargo.toml")?
        .context("crate archive has no Cargo.toml")?;
    fs::create_dir_all(&dir)?;
    for stub in stub_paths(&manifest)? {
        let path = dir.join(&stub);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, "")?;
    }
    fs::write(dir.join("Cargo.toml"), manifest)?;
    if let Some(lock) = registry::read_file(&file, &entry, "Cargo.lock")? {
        fs::write(dir.join("Cargo.lock"), lock)?;
    }
    let checkout = Checkout {
        workspace_root: dir.clone(),
        package_dir: dir,
        version: entry.vers.to_string(),
        commit: None,
        _tmp: tmp,
    };
    isolate_workspace(&checkout)?;
    Ok(checkout)
}

/// Files that must exist for cargo to load the manifest: default target roots, explicit
/// target paths and the build script.
fn stub_paths(manifest: &str) -> Result<Vec<PathBuf>> {
    let doc: toml_edit::DocumentMut = manifest.parse()?;
    let mut paths = vec![PathBuf::from("src/lib.rs"), PathBuf::from("src/main.rs")];
    let mut push = |p: &str| -> Result<()> {
        let path = PathBuf::from(p);
        if path.is_absolute()
            || path
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            bail!("unsafe target path `{p}` in manifest");
        }
        paths.push(path);
        Ok(())
    };
    if let Some(p) = doc
        .get("lib")
        .and_then(|l| l.get("path"))
        .and_then(|p| p.as_str())
    {
        push(p)?;
    }
    for kind in ["bin", "example", "test", "bench"] {
        if let Some(arr) = doc.get(kind).and_then(|v| v.as_array_of_tables()) {
            for t in arr.iter() {
                if let Some(p) = t.get("path").and_then(|p| p.as_str()) {
                    push(p)?;
                }
            }
        }
    }
    if let Some(p) = doc
        .get("package")
        .and_then(|p| p.get("build"))
        .and_then(|b| b.as_str())
    {
        push(p)?;
    }
    Ok(paths)
}

/// Make a standalone package its own workspace so a parent directory's workspace can't claim it.
fn isolate_workspace(checkout: &Checkout) -> Result<()> {
    if checkout.package_dir != checkout.workspace_root {
        return Ok(());
    }
    let path = checkout.manifest_path();
    let text = fs::read_to_string(&path)?;
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    if !doc.contains_key("workspace") {
        doc.insert("workspace", toml_edit::Item::Table(toml_edit::Table::new()));
        fs::write(&path, doc.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
