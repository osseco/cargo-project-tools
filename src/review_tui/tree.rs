//! The review screen's tree: the kinds of build-time code, the pending pieces of each, and
//! their files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::text::{Warn, shown};
use crate::review::{Item, Kind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NodeKind {
    Group,
    /// A piece of build-time code that is a single file.
    ItemFile(PathBuf),
    /// A piece of build-time code that is a directory.
    ItemDir,
    Dir,
    File(PathBuf),
    Link,
}

#[derive(Debug, Clone)]
pub(super) struct Node {
    pub(super) depth: usize,
    /// Already sanitized.
    pub(super) label: String,
    pub(super) item: Option<usize>,
    pub(super) kind: NodeKind,
    pub(super) expanded: bool,
    pub(super) warn: Warn,
}

impl Node {
    pub(super) fn folds(&self) -> bool {
        matches!(
            self.kind,
            NodeKind::Group | NodeKind::ItemDir | NodeKind::Dir
        )
    }
}

/// The tree for `items`: groups expanded, everything below them folded.
pub(super) fn build(items: &[Item]) -> Result<Vec<Node>> {
    let mut nodes = Vec::new();
    for kind in [Kind::BuildScript, Kind::ProcMacro, Kind::CargoConfig] {
        if !items.iter().any(|i| i.kind == kind) {
            continue;
        }
        nodes.push(Node {
            depth: 0,
            label: kind.to_string(),
            item: None,
            kind: NodeKind::Group,
            expanded: true,
            warn: Warn::None,
        });
        for (idx, item) in items.iter().enumerate().filter(|(_, i)| i.kind == kind) {
            let label = shown(&item.describe());
            let script_dir = item.location.parent().filter(|p| *p != item.base);
            if item.kind == Kind::BuildScript
                && (item.pulled.whole_package.is_some() || !item.pulled.files.is_empty())
            {
                pulling_script(idx, item, label, script_dir, &mut nodes)?;
                continue;
            }
            let dir = match item.kind {
                Kind::ProcMacro => Some(item.location.as_path()),
                Kind::BuildScript => script_dir,
                Kind::CargoConfig => None,
            };
            match dir {
                None => nodes.push(Node {
                    depth: 1,
                    label,
                    item: Some(idx),
                    kind: NodeKind::ItemFile(item.location.clone()),
                    expanded: false,
                    warn: Warn::of(&item.location),
                }),
                Some(dir) => {
                    let at = nodes.len();
                    nodes.push(Node {
                        depth: 1,
                        label,
                        item: Some(idx),
                        kind: NodeKind::ItemDir,
                        expanded: false,
                        warn: Warn::None,
                    });
                    walk(dir, 2, idx, item, &mut nodes)?;
                    if item.kind == Kind::BuildScript {
                        nodes[at].warn = Warn::of(&item.location);
                    }
                }
            }
        }
    }
    Ok(nodes)
}

/// A build script that pulls in other files: a folder with its directory (or itself), then
/// the files it pulls in by their path in the package, or the whole package.
fn pulling_script(
    idx: usize,
    item: &Item,
    label: String,
    script_dir: Option<&Path>,
    nodes: &mut Vec<Node>,
) -> Result<()> {
    nodes.push(Node {
        depth: 1,
        label,
        item: Some(idx),
        kind: NodeKind::ItemDir,
        expanded: false,
        warn: Warn::of(&item.location),
    });
    if item.pulled.whole_package.is_some() {
        return walk(&item.base, 2, idx, item, nodes);
    }
    let file = |path: &Path, label: String| Node {
        depth: 2,
        label,
        item: Some(idx),
        kind: NodeKind::File(path.to_path_buf()),
        expanded: false,
        warn: Warn::of(path),
    };
    match script_dir {
        Some(dir) => walk(dir, 2, idx, item, nodes)?,
        None => {
            let name = shown(&item.path);
            nodes.push(file(&item.location, format!("{name}  ← build script")));
        }
    }
    for path in &item.pulled.files {
        let rel = path.strip_prefix(&item.base).unwrap_or(path);
        nodes.push(file(path, shown(&rel.to_string_lossy())));
    }
    Ok(())
}

fn walk(dir: &Path, depth: usize, idx: usize, item: &Item, nodes: &mut Vec<Node>) -> Result<()> {
    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<io::Result<Vec<_>>>()?;
    // Directories first, then files, each by name.
    entries.sort_by_key(|e| (!e.file_type().is_ok_and(|t| t.is_dir()), e.file_name()));
    for entry in entries {
        let path = entry.path();
        let name = shown(&entry.file_name().to_string_lossy());
        let kind = entry.file_type()?;
        let node = |label, kind, warn| Node {
            depth,
            label,
            item: Some(idx),
            kind,
            expanded: false,
            warn,
        };
        if kind.is_symlink() {
            let target = shown(&fs::read_link(&path)?.to_string_lossy());
            nodes.push(node(
                format!("{name} -> {target}"),
                NodeKind::Link,
                Warn::None,
            ));
        } else if kind.is_dir() {
            if entry.file_name() != ".git" {
                nodes.push(node(format!("{name}/"), NodeKind::Dir, Warn::None));
                walk(&path, depth + 1, idx, item, nodes)?;
            }
        } else {
            let label = if path == item.location {
                format!("{name}  ← build script")
            } else {
                name
            };
            let warn = Warn::of(&path);
            nodes.push(node(label, NodeKind::File(path), warn));
        }
    }
    Ok(())
}

/// The nodes not hidden in a folded node, in order.
pub(super) fn visible(nodes: &[Node]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut hidden_below: Option<usize> = None;
    for (i, n) in nodes.iter().enumerate() {
        if let Some(d) = hidden_below {
            if n.depth > d {
                continue;
            }
            hidden_below = None;
        }
        out.push(i);
        if n.folds() && !n.expanded {
            hidden_below = Some(n.depth);
        }
    }
    out
}

pub(super) fn parent(nodes: &[Node], node: usize) -> Option<usize> {
    let depth = nodes[node].depth;
    (0..node).rev().find(|&i| nodes[i].depth < depth)
}

#[cfg(test)]
#[path = "tree_test.rs"]
mod tests;
