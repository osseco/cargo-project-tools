use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::build_files::{self, Pulled};
use crate::graph;
use crate::lock;
use crate::output::{self, Record, say};
use crate::process;
use crate::project::Project;
use crate::sanitize::{shown, shown_line};
use crate::screen;
use crate::source::registry::sha256_hex;
use crate::source::{self, Checkout};
use crate::vendor::Vendored;

#[path = "review_tui.rs"]
mod tui;

#[path = "review_previous.rs"]
mod previous;

/// Per-tool file recording the build-time code that may run, next to the tool's lock.
pub const FILE: &str = "build-review.toml";

const TEMPLATE: &str =
    "# Code that runs while building this tool. `install` halts on anything not listed here.
# Review it on the review screen: `cargo project-tools review <tool>`. See docs/build-review.md.

";

/// What to do with a piece of build-time code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Build it as shipped.
    Approve,
    /// Replace it with our file.
    Override,
    /// Remove it.
    Strip,
}

impl Action {
    fn as_str(self) -> &'static str {
        match self {
            Action::Approve => "approve",
            Action::Override => "override",
            Action::Strip => "strip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    BuildScript,
    ProcMacro,
    /// A library that build scripts or proc macros run: a build-dependency, a proc macro's
    /// own dependency, and what those depend on.
    BuildDep,
    CargoConfig,
}

impl Kind {
    /// The array-of-tables name in the review file.
    fn table(self) -> &'static str {
        match self {
            Kind::BuildScript => "build-script",
            Kind::ProcMacro => "proc-macro",
            Kind::BuildDep => "build-dep",
            Kind::CargoConfig => "cargo-config",
        }
    }

    /// The field that pins the content.
    fn hash_key(self) -> &'static str {
        if self.whole_crate() {
            "checksum"
        } else {
            "sha256"
        }
    }

    /// A whole crate, pinned by its checksum (or commit, or files), that can only be
    /// approved: proc macros and the libraries build-time code runs.
    pub fn whole_crate(self) -> bool {
        matches!(self, Kind::ProcMacro | Kind::BuildDep)
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.table())
    }
}

/// The crate a build script comes with: its checksum in the lock for a registry package,
/// its commit for a git package, nothing for a package of the tool's own checkout. An entry
/// only holds for the crate it was recorded for, since a build script can run or read any
/// file of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pin {
    #[default]
    None,
    Checksum(String),
    Commit(String),
}

impl Pin {
    /// The review-file key and value.
    fn key(&self) -> Option<(&'static str, &str)> {
        match self {
            Pin::None => None,
            Pin::Checksum(c) => Some(("checksum", c)),
            Pin::Commit(c) => Some(("commit", c)),
        }
    }
}

/// A piece of build-time code found in what the tool builds.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    /// Package name; empty for cargo config.
    pub package: String,
    pub version: String,
    /// Build script relative to its package, or config relative to the checkout root; empty
    /// for proc macros.
    pub path: String,
    /// sha256 of the content, or the crate checksum for proc macros.
    pub hash: String,
    /// For a build script: the crate it comes with.
    pub pin: Pin,
    /// What to review on disk.
    pub location: PathBuf,
    /// The package directory for build scripts and proc macros, the checkout root for config.
    pub base: PathBuf,
    /// In the vendor directory, where edits must be dropped from `.cargo-checksum.json`.
    pub vendored: bool,
    /// For a build script: the files it pulls in that its hash covers besides itself (and
    /// its directory), or why it covers the whole package.
    pub pulled: Pulled,
    /// The package's source in the lock; none for the tool's own packages and cargo config.
    pub source: Option<String>,
    /// The crate its changed entry was recorded for, when that was fetched.
    pub old: Option<previous::OldCopy>,
}

impl Item {
    fn same_place(&self, e: &Entry) -> bool {
        self.kind == e.kind && self.package == e.package && self.path == e.path
    }

    fn matches(&self, e: &Entry) -> bool {
        self.same_place(e) && self.hash == e.hash && self.pin == e.pin
    }

    fn describe(&self) -> String {
        match self.kind {
            Kind::BuildScript => format!("{}@{} {}", self.package, self.version, self.path),
            Kind::ProcMacro | Kind::BuildDep => format!("{}@{}", self.package, self.version),
            Kind::CargoConfig => self.path.clone(),
        }
    }
}

/// A recorded decision from the review file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub package: String,
    pub path: String,
    pub hash: String,
    pub pin: Pin,
    pub action: Action,
    /// With `override`: the replacement, relative to the tool's crate directory.
    pub file: Option<String>,
    /// Where its `[[…]]` is in the file, for records.
    pub line: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    package: Option<String>,
    path: Option<String>,
    sha256: Option<String>,
    checksum: Option<String>,
    commit: Option<String>,
    action: Action,
    file: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RawFile {
    #[serde(default)]
    build_script: Vec<RawEntry>,
    #[serde(default)]
    proc_macro: Vec<RawEntry>,
    #[serde(default)]
    build_dep: Vec<RawEntry>,
    #[serde(default)]
    cargo_config: Vec<RawEntry>,
}

/// The kinds and packages a review file has entries for, whatever their hash.
pub fn recorded(path: &Path) -> Result<BTreeSet<(String, String)>> {
    Ok(load(path)?
        .into_iter()
        .map(|e| (e.kind.table().to_string(), e.package))
        .collect())
}

/// Read the review file; a missing file has no entries.
pub fn load(path: &Path) -> Result<Vec<Entry>> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    parse(&text).with_context(|| format!("parsing {}", path.display()))
}

fn parse(text: &str) -> Result<Vec<Entry>> {
    let raw: RawFile = toml::from_str(text)?;
    let mut out = Vec::new();
    for (kind, list) in [
        (Kind::BuildScript, raw.build_script),
        (Kind::ProcMacro, raw.proc_macro),
        (Kind::BuildDep, raw.build_dep),
        (Kind::CargoConfig, raw.cargo_config),
    ] {
        let mut lines = crate::toml_line::tables(text, kind.table()).into_iter();
        for r in list {
            let mut e = entry(kind, r)?;
            e.line = lines.next().map(|(line, _)| line);
            out.push(e);
        }
    }
    Ok(out)
}

fn entry(kind: Kind, r: RawEntry) -> Result<Entry> {
    let need = |v: Option<String>, field: &str| {
        v.with_context(|| format!("a [[{kind}]] entry needs `{field}`"))
    };
    let unexpected = match kind {
        Kind::BuildScript => r.checksum.is_some() && r.commit.is_some(),
        Kind::ProcMacro | Kind::BuildDep => {
            r.path.is_some() || r.sha256.is_some() || r.commit.is_some()
        }
        Kind::CargoConfig => r.package.is_some() || r.checksum.is_some() || r.commit.is_some(),
    };
    if unexpected {
        bail!("a [[{kind}]] entry has fields that belong to another kind");
    }
    let pin = match (kind, r.checksum.clone(), r.commit) {
        (Kind::BuildScript, Some(c), _) => Pin::Checksum(c),
        (Kind::BuildScript, None, Some(c)) => Pin::Commit(c),
        _ => Pin::None,
    };
    let (package, path, hash) = match kind {
        Kind::BuildScript => (
            need(r.package, "package")?,
            need(r.path, "path")?,
            need(r.sha256, "sha256")?,
        ),
        Kind::ProcMacro | Kind::BuildDep => (
            need(r.package, "package")?,
            String::new(),
            need(r.checksum, "checksum")?,
        ),
        Kind::CargoConfig => (
            String::new(),
            need(r.path, "path")?,
            need(r.sha256, "sha256")?,
        ),
    };
    if kind.whole_crate() && r.action != Action::Approve {
        bail!("[[{kind}]] `{package}` can only be approved");
    }
    match (r.action, &r.file) {
        (Action::Override, None) => bail!("[[{kind}]] `{package}{path}`: override needs `file`"),
        (Action::Override, Some(f)) => safe_relative(f)?,
        (_, Some(_)) => bail!("[[{kind}]] `{package}{path}`: `file` only goes with override"),
        _ => {}
    }
    Ok(Entry {
        kind,
        package,
        path,
        hash,
        pin,
        action: r.action,
        file: r.file,
        line: None,
    })
}

fn safe_relative(p: &str) -> Result<()> {
    let path = Path::new(p);
    if p.is_empty()
        || p.chars().any(char::is_control)
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("unsafe override path `{}`", shown_line(p));
    }
    Ok(())
}

/// `rel` with `/` separators, as cargo writes paths in `.cargo-checksum.json`.
fn slash(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// The sha256 line (`<hash>  <path>`, like `sha256sum`) for each file under `dir`, with
/// paths relative to `root`. Links are recorded by their target. `.git` is skipped unless
/// `vendored`: in a checkout it's our own clone, but a vendored package's is part of it.
fn dir_lines(root: &Path, dir: &Path, vendored: bool, lines: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let rel = slash(path.strip_prefix(root)?);
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            let target = fs::read_link(&path)?;
            lines.push(format!("link {} -> {}\n", rel, target.display()));
        } else if kind.is_dir() {
            if vendored || entry.file_name() != ".git" {
                dir_lines(root, &path, vendored, lines)?;
            }
        } else {
            lines.push(file_line(root, &path)?);
        }
    }
    Ok(())
}

fn file_line(root: &Path, path: &Path) -> Result<String> {
    let rel = slash(path.strip_prefix(root)?);
    Ok(format!("{}  {}\n", sha256_hex(&fs::read(path)?), rel))
}

/// The lines, sorted and hashed.
fn lines_hash(mut lines: Vec<String>) -> String {
    lines.sort();
    sha256_hex(lines.concat().as_bytes())
}

/// sha256 lines for every file under `dir`, sorted, and hashed.
fn dir_hash(dir: &Path, vendored: bool) -> Result<String> {
    let mut lines = Vec::new();
    dir_lines(dir, dir, vendored, &mut lines)?;
    Ok(lines_hash(lines))
}

/// The build script's own file when it sits at the crate root, else its whole directory,
/// which is where the modules it uses usually live; plus the files it pulls in from
/// elsewhere, or else the whole package. Without such files, the hash is what it was before
/// they were followed.
fn script_hash(pkg_dir: &Path, script: &Path, pulled: &Pulled, vendored: bool) -> Result<String> {
    if pulled.whole_package.is_some() {
        return dir_hash(pkg_dir, vendored);
    }
    let parent = script
        .parent()
        .context("build script without a directory")?;
    if pulled.files.is_empty() {
        return if parent == pkg_dir {
            Ok(sha256_hex(&fs::read(script)?))
        } else {
            dir_hash(parent, vendored)
        };
    }
    let mut lines = Vec::new();
    if parent == pkg_dir {
        lines.push(file_line(pkg_dir, script)?);
    } else {
        dir_lines(pkg_dir, parent, vendored, &mut lines)?;
    }
    for f in &pulled.files {
        lines.push(file_line(pkg_dir, f)?);
    }
    Ok(lines_hash(lines))
}

/// What the build script `script` pulls in beyond what `script_hash` already covers.
fn pulled_by(pkg_dir: &Path, script: &Path) -> Result<Pulled> {
    let mut pulled = build_files::find(pkg_dir, script)?;
    if let Some(dir) = script.parent().filter(|d| *d != pkg_dir) {
        pulled.files.retain(|f| !f.starts_with(dir));
    }
    Ok(pulled)
}

/// A key in a shipped cargo config that makes cargo build something other than what was
/// reviewed: other sources (`[source]`, `[patch]`, `paths`) or another target.
fn changes_what_is_built(doc: &toml::Table) -> Option<&'static str> {
    for key in ["source", "patch", "paths"] {
        if doc.contains_key(key) {
            return Some(key);
        }
    }
    doc.get("build")
        .and_then(|b| b.get("target"))
        .map(|_| "build.target")
}

/// The crate a package comes from: its checksum for a registry package, its commit for a git
/// one, nothing for a package of the checkout. `checksums` holds registry packages only.
fn crate_pin(
    source: Option<&str>,
    name: &str,
    version: &str,
    checksums: &BTreeMap<(String, String), String>,
) -> Pin {
    let source = source.unwrap_or("");
    if is_registry(source) {
        checksums
            .get(&(name.to_string(), version.to_string()))
            .cloned()
            .map_or(Pin::None, Pin::Checksum)
    } else {
        git_commit(source).map_or(Pin::None, Pin::Commit)
    }
}

/// A registry source in cargo's notation: `registry+https://…` or `sparse+https://…`.
fn is_registry(source: &str) -> bool {
    source.starts_with("registry+") || source.starts_with("sparse+")
}

/// The commit of a `git+https://…#<commit>` source.
fn git_commit(source: &str) -> Option<String> {
    source
        .strip_prefix("git+")
        .and_then(|s| s.rsplit_once('#'))
        .map(|(_, c)| c.to_string())
}

/// Fail unless `dir`, with links resolved, is inside one of `roots` (already resolved). A
/// workspace member or path dependency that leads elsewhere, through a link or `..`, would
/// let an override or strip write outside the tool's sources.
fn inside_sources(dir: &Path, roots: &[PathBuf]) -> Result<()> {
    let real = dir
        .canonicalize()
        .with_context(|| format!("reading {}", dir.display()))?;
    if !roots.iter().any(|r| real.starts_with(r)) {
        bail!(
            "{} is outside the tool's checkout and vendored sources",
            dir.display()
        );
    }
    Ok(())
}

/// Fail when `path` is a link. Registry archives and `cargo vendor` have none, but a git
/// repository can point a file anywhere.
fn refuse_link(path: &Path) -> Result<()> {
    let meta = path
        .symlink_metadata()
        .with_context(|| format!("reading {}", path.display()))?;
    if meta.file_type().is_symlink() {
        bail!("{} is a link; refusing to build with it", path.display());
    }
    Ok(())
}

/// Cargo config files a build of the checkout would read. Links are refused, and so is
/// `include`, which would pull in files nobody reviewed.
fn shipped_configs(checkout: &Checkout) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for dir in source::search_dirs(checkout)? {
        let cargo_dir = dir.join(".cargo");
        let Ok(meta) = cargo_dir.symlink_metadata() else {
            continue;
        };
        if meta.file_type().is_symlink() {
            bail!(
                "{} is a link; refusing to build with it",
                cargo_dir.display()
            );
        }
        for name in ["config", "config.toml"] {
            let path = cargo_dir.join(name);
            let Ok(meta) = path.symlink_metadata() else {
                continue;
            };
            if !meta.file_type().is_file() {
                bail!(
                    "{} is not a plain file; refusing to build with it",
                    path.display()
                );
            }
            let bytes = fs::read(&path)?;
            let doc: toml::Table = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|t| t.parse().ok())
                .with_context(|| format!("parsing {}", path.display()))?;
            if doc.contains_key("include") {
                bail!(
                    "{} uses `include`; included files would bypass review",
                    path.display()
                );
            }
            if let Some(key) = changes_what_is_built(&doc) {
                bail!(
                    "{} sets `{key}`, which changes what's built behind the review; refusing it",
                    path.display()
                );
            }
            items.push(Item {
                kind: Kind::CargoConfig,
                package: String::new(),
                version: String::new(),
                path: slash(path.strip_prefix(&checkout.root)?),
                hash: sha256_hex(&bytes),
                pin: Pin::None,
                location: path,
                base: checkout.root.clone(),
                vendored: false,
                pulled: Pulled::default(),
                source: None,
                old: None,
            });
        }
    }
    Ok(items)
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetaPackage>,
    #[serde(default)]
    resolve: Option<Resolve>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDep>,
}

#[derive(Deserialize)]
struct NodeDep {
    pkg: String,
    dep_kinds: Vec<DepKind>,
}

#[derive(Deserialize)]
struct DepKind {
    /// `null` for a normal dependency, else `build` or `dev`.
    kind: Option<String>,
}

#[derive(Deserialize)]
struct MetaPackage {
    id: String,
    name: String,
    version: String,
    source: Option<String>,
    manifest_path: PathBuf,
    targets: Vec<MetaTarget>,
}

#[derive(Deserialize)]
struct MetaTarget {
    kind: Vec<String>,
    src_path: PathBuf,
}

/// Every build script and proc macro the tool builds for `selection`, and the cargo config
/// it ships. Uses `cargo tree` and `cargo metadata` on the vendored sources: nothing is
/// compiled and nothing runs.
pub fn collect(
    name: &str,
    selection: graph::Selection,
    checkout: &Checkout,
    vendored: &Vendored,
    offline: bool,
) -> Result<Vec<Item>> {
    let mut items = shipped_configs(checkout)?;
    let manifest = checkout.manifest_path();
    let built = graph::tree(&manifest, selection, &vendored.config_args, offline, name)?;

    // Every feature, so the packages listed cover whatever `built` holds: without them,
    // cargo lists only what the default features build, and a build script or proc macro
    // behind another feature would be missed. `built` decides what counts.
    let mut cmd = process::cargo();
    cmd.args([
        "metadata",
        "--locked",
        "--all-features",
        "--format-version",
        "1",
        "--manifest-path",
    ])
    .arg(&manifest)
    .args(selection.config)
    .args(&vendored.config_args);
    if offline {
        cmd.arg("--offline");
    }
    let json = process::output(&mut cmd).context("reading package metadata")?;
    let meta: Metadata = serde_json::from_str(&json)?;
    let host = run_at_build_time(&manifest, selection, vendored, offline)?;

    // What an override or strip may write into: the checkout and the vendored copies.
    let roots: Vec<PathBuf> = [&checkout.root, &vendored.dir]
        .into_iter()
        .filter_map(|d| d.canonicalize().ok())
        .collect();
    let lock = lock::load(&checkout.lock_path())?;
    // Checksums of registry packages only: a git or path package of the same name and version
    // must never borrow one.
    let checksums: BTreeMap<(String, String), String> = lock
        .packages
        .iter()
        .filter(|p| {
            p.source
                .as_ref()
                .is_some_and(|s| is_registry(&s.to_string()))
        })
        .filter_map(|p| {
            let sum = p.checksum.as_ref()?.to_string();
            Some(((p.name.as_str().to_string(), p.version.to_string()), sum))
        })
        .collect();

    for pkg in meta.packages {
        let Ok(version) = semver::Version::parse(&pkg.version) else {
            continue;
        };
        if !built.contains(&(pkg.name.clone(), version)) {
            continue;
        }
        let pkg_dir = pkg
            .manifest_path
            .parent()
            .context("bad manifest path")?
            .to_path_buf();
        inside_sources(&pkg_dir, &roots)
            .with_context(|| format!("package {} {}", pkg.name, pkg.version))?;
        let in_vendor = pkg_dir.starts_with(&vendored.dir);
        let pin = crate_pin(pkg.source.as_deref(), &pkg.name, &pkg.version, &checksums);
        for t in &pkg.targets {
            if t.kind.iter().any(|k| k == "custom-build") {
                let rel = t.src_path.strip_prefix(&pkg_dir).with_context(|| {
                    format!("build script of {} is outside its package", pkg.name)
                })?;
                // The path goes into review files, comments and the screen as one line.
                if rel.to_string_lossy().chars().any(char::is_control) {
                    bail!(
                        "build script of {} has control characters in its path: {}",
                        pkg.name,
                        shown_line(&rel.to_string_lossy())
                    );
                }
                // An override or strip writes these; through a link that would be elsewhere.
                refuse_link(&t.src_path)?;
                refuse_link(&pkg.manifest_path)?;
                let pulled = pulled_by(&pkg_dir, &t.src_path)
                    .with_context(|| format!("reading the build script of {}", pkg.name))?;
                items.push(Item {
                    kind: Kind::BuildScript,
                    package: pkg.name.clone(),
                    version: pkg.version.clone(),
                    path: slash(rel),
                    hash: script_hash(&pkg_dir, &t.src_path, &pulled, in_vendor)?,
                    pin: pin.clone(),
                    location: t.src_path.clone(),
                    base: pkg_dir.clone(),
                    vendored: in_vendor,
                    pulled,
                    source: pkg.source.clone(),
                    old: None,
                });
            }
        }
        if pkg
            .targets
            .iter()
            .any(|t| t.kind.iter().any(|k| k == "proc-macro"))
        {
            let hash = match pin.clone() {
                Pin::Checksum(sum) => sum,
                Pin::Commit(commit) => commit,
                // A package of the tool's own workspace: pinned by its files.
                Pin::None => dir_hash(&pkg_dir, in_vendor)?,
            };
            items.push(Item {
                kind: Kind::ProcMacro,
                package: pkg.name.clone(),
                version: pkg.version.clone(),
                path: String::new(),
                hash,
                pin: Pin::None,
                location: pkg_dir.clone(),
                base: pkg_dir,
                vendored: in_vendor,
                pulled: Pulled::default(),
                source: pkg.source.clone(),
                old: None,
            });
        } else if host.contains(&pkg.id) {
            // A library build-time code runs; a proc macro is reviewed as one already.
            let hash = match pin.clone() {
                Pin::Checksum(sum) => sum,
                Pin::Commit(commit) => commit,
                Pin::None => dir_hash(&pkg_dir, in_vendor)?,
            };
            items.push(Item {
                kind: Kind::BuildDep,
                package: pkg.name.clone(),
                version: pkg.version.clone(),
                path: String::new(),
                hash,
                pin: Pin::None,
                location: pkg_dir.clone(),
                base: pkg_dir,
                vendored: in_vendor,
                pulled: Pulled::default(),
                source: pkg.source.clone(),
                old: None,
            });
        }
    }
    items.sort_by(|a, b| {
        (a.kind, &a.package, &a.version, &a.path).cmp(&(b.kind, &b.package, &b.version, &b.path))
    });
    Ok(items)
}

/// The packages build-time code runs, by package id: what a build script of a built package
/// depends on (its build-dependencies), what a built proc macro depends on, and everything
/// those depend on in turn. Resolved with the tool's features and the build's config, once
/// per target (they run on the machine that builds, which is one of them), or for every
/// platform without targets.
fn run_at_build_time(
    manifest: &Path,
    selection: graph::Selection,
    vendored: &Vendored,
    offline: bool,
) -> Result<BTreeSet<String>> {
    let platforms: Vec<Option<&String>> = match selection.targets {
        Some(list) => list.iter().map(Some).collect(),
        None => vec![None],
    };
    let mut host = BTreeSet::new();
    for platform in platforms {
        let mut cmd = process::cargo();
        cmd.args(["metadata", "--locked", "--format-version", "1"])
            .arg("--manifest-path")
            .arg(manifest)
            .args(selection.features)
            .args(selection.config)
            .args(&vendored.config_args);
        if let Some(t) = platform {
            cmd.arg("--filter-platform").arg(t);
        }
        if offline {
            cmd.arg("--offline");
        }
        let json = process::output(&mut cmd).context("reading the dependency graph")?;
        let meta: Metadata = serde_json::from_str(&json)?;
        host.extend(host_closure(&meta)?);
    }
    Ok(host)
}

/// The libraries build-time code runs in a `cargo metadata` graph, as name and version,
/// proc macros left out (they're their own kind). For `inspect`.
pub fn build_deps_in(metadata_json: &str) -> Result<BTreeSet<(String, String)>> {
    let meta: Metadata = serde_json::from_str(metadata_json)?;
    let host = host_closure(&meta)?;
    Ok(meta
        .packages
        .iter()
        .filter(|p| host.contains(&p.id))
        .filter(|p| {
            !p.targets
                .iter()
                .any(|t| t.kind.iter().any(|k| k == "proc-macro"))
        })
        .map(|p| (p.name.clone(), p.version.clone()))
        .collect())
}

/// `run_at_build_time` for one resolved graph.
fn host_closure(meta: &Metadata) -> Result<BTreeSet<String>> {
    let resolve = meta
        .resolve
        .as_ref()
        .context("cargo metadata gave no dependency graph")?;
    let nodes: BTreeMap<&str, &Node> = resolve.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let proc_macros: BTreeSet<&str> = meta
        .packages
        .iter()
        .filter(|p| {
            p.targets
                .iter()
                .any(|t| t.kind.iter().any(|k| k == "proc-macro"))
        })
        .map(|p| p.id.as_str())
        .collect();
    let is =
        |d: &NodeDep, kind: Option<&str>| d.dep_kinds.iter().any(|k| k.kind.as_deref() == kind);
    let mut todo: Vec<&str> = Vec::new();
    for node in &resolve.nodes {
        for d in &node.deps {
            if is(d, Some("build")) || (proc_macros.contains(node.id.as_str()) && is(d, None)) {
                todo.push(&d.pkg);
            }
        }
    }
    let mut host = BTreeSet::new();
    while let Some(id) = todo.pop() {
        if !host.insert(id.to_string()) {
            continue;
        }
        if let Some(node) = nodes.get(id) {
            for d in &node.deps {
                if d.dep_kinds.iter().any(|k| k.kind.as_deref() != Some("dev")) {
                    todo.push(&d.pkg);
                }
            }
        }
    }
    Ok(host)
}

/// An item with the decision that applies to it.
#[derive(Debug)]
pub struct Decided {
    pub item: Item,
    pub action: Action,
    pub file: Option<String>,
}

/// An item with no matching entry, and the entry for the same place whose content changed.
type Pending<'a> = (Item, Option<&'a Entry>);

/// A decision about a pending item, to be written to the review file.
struct Decision<'a> {
    item: Item,
    /// The entry for the same place whose content changed; it's updated in place.
    previous: Option<&'a Entry>,
    action: Action,
    file: Option<String>,
}

impl<'a> Decision<'a> {
    /// Keep the action and file of the changed entry, or approve new code.
    fn keep_or_approve((item, previous): Pending<'a>) -> Self {
        Decision {
            item,
            previous,
            action: previous.map_or(Action::Approve, |p| p.action),
            file: previous.and_then(|p| p.file.clone()),
        }
    }
}

/// A decision recorded for the same piece (kind, package, path and hash) in another tool's
/// review file: that piece has been reviewed already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    pub tool: String,
    pub action: Action,
    /// With `override`: that tool's override file.
    pub file: Option<PathBuf>,
}

/// Decisions in other tools' review files, by kind, package, path and hash.
pub type KnownMap = BTreeMap<(Kind, String, String, String, Pin), Known>;

fn known_for<'a>(known: &'a KnownMap, item: &Item) -> Option<&'a Known> {
    known.get(&(
        item.kind,
        item.package.clone(),
        item.path.clone(),
        item.hash.clone(),
        item.pin.clone(),
    ))
}

/// The decisions in the review files of the other configured tools, `tools`, under
/// `crates_dir`; a directory that isn't a configured tool counts for nothing. A file that
/// can't be read is skipped with a warning. A piece the tools decided differently on is left
/// out: there's no one decision to start from.
fn known_elsewhere(crates_dir: &Path, name: &str, tools: &[String]) -> KnownMap {
    let mut known = KnownMap::new();
    let mut disputed = BTreeSet::new();
    let mut tools: Vec<&String> = tools.iter().filter(|t| *t != name).collect();
    tools.sort();
    for tool in tools {
        let dir = crates_dir.join(tool);
        let path = dir.join(FILE);
        if !path.exists() {
            continue;
        }
        let entries = match load(&path) {
            Ok(e) => e,
            Err(e) => {
                say!(
                    "  warning: skipping {}: {e:#}",
                    shown(&path.display().to_string())
                );
                continue;
            }
        };
        for e in entries {
            let key = (e.kind, e.package, e.path, e.hash, e.pin);
            match known.get(&key) {
                Some(k) if k.action != e.action => {
                    disputed.insert(key);
                }
                Some(_) => {}
                None => {
                    known.insert(
                        key,
                        Known {
                            tool: tool.clone(),
                            action: e.action,
                            file: e.file.map(|f| dir.join(f)),
                        },
                    );
                }
            }
        }
    }
    known.retain(|k, _| !disputed.contains(k));
    known
}

/// The tools in project-tools.toml, whose review decisions may be reused; none without one.
fn configured_tools(project: &Project) -> Result<Vec<String>> {
    let path = project.config_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(crate::config::Config::load(&path)?
        .tool
        .into_keys()
        .collect())
}

/// How `check` treats pending items.
pub enum Mode<'a> {
    /// Fail listing them.
    Report,
    /// Fail listing them, pointing at the `review` command (`check`).
    Check,
    /// Record them all (`--auto-approve`).
    ApproveAll,
    /// Decide on each on the review screen.
    Interactive { tool: &'a str, crate_dir: &'a Path },
    /// Record the decisions in `picks` (`--approve`, `--strip`, `--override`), and fail
    /// listing the rest.
    Picked {
        picks: &'a [Pick],
        crate_dir: &'a Path,
    },
}

/// How pending build-time code gets decided outside the review screen.
#[derive(Debug, Clone, Copy)]
pub enum Approval<'a> {
    /// On the review screen in a terminal, else not at all.
    Ask,
    /// All of it (`--auto-approve`).
    All,
    /// What the command line names (`review --approve` and the like).
    Picked(&'a [Pick]),
}

impl<'a> Approval<'a> {
    pub fn new(all: bool, picks: &'a [Pick]) -> Approval<'a> {
        match (all, picks) {
            (true, _) => Approval::All,
            (false, []) => Approval::Ask,
            (false, picks) => Approval::Picked(picks),
        }
    }
}

/// A decision named on the command line for pending build-time code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    /// `package@version`, or a cargo config's path.
    pub spec: String,
    /// The hash it was read with: a record's `sha256`, or `checksum` for a whole crate.
    pub hash: Option<String>,
    pub action: Action,
}

impl Pick {
    /// `package@version[=hash]`, or `path[=hash]` for a cargo config.
    pub fn parse(action: Action, text: &str) -> Result<Pick> {
        let (spec, hash) = match text.split_once('=') {
            Some((spec, hash)) => (spec, Some(hash.to_string())),
            None => (text, None),
        };
        if spec.is_empty() || hash.as_deref() == Some("") {
            bail!(
                "expected package@version[=hash], got `{}`",
                shown_line(text)
            );
        }
        Ok(Pick {
            spec: spec.to_string(),
            hash,
            action,
        })
    }

    /// Strip and override only take build scripts and cargo config; approve takes anything
    /// of the package.
    fn selects(&self, item: &Item) -> bool {
        if self.action != Action::Approve && item.kind.whole_crate() {
            return false;
        }
        match self.spec.split_once('@') {
            Some((name, version)) => item.package == name && item.version == version,
            None => item.kind == Kind::CargoConfig && item.path == self.spec,
        }
    }
}

/// Match `items` against the review file at `path`. Items without a matching entry are
/// pending. `Mode::Report` fails listing them. `Mode::ApproveAll` records them, keeping the
/// action of an entry for the same place whose content changed. `Mode::Interactive` records
/// what's decided on the review screen and fails listing the rest. Entries that match nothing
/// are reported, and removed when recording. Decisions `known` from other tools are offered
/// on the screen and in the listing.
pub fn check(
    path: &Path,
    items: Vec<Item>,
    mode: Mode,
    known: &KnownMap,
    root: &Path,
) -> Result<Vec<Decided>> {
    let entries = load(path)?;
    let stale: Vec<&Entry> = entries
        .iter()
        .filter(|e| !items.iter().any(|i| i.matches(e)))
        .collect();
    let mut decided = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    for item in items {
        match entries.iter().find(|e| item.matches(e)) {
            Some(e) => decided.push(Decided {
                item,
                action: e.action,
                file: e.file.clone(),
            }),
            None => {
                let previous = stale.iter().find(|e| item.same_place(e)).copied();
                pending.push((item, previous));
            }
        }
    }
    match mode {
        Mode::ApproveAll => {
            let decisions = pending.into_iter().map(Decision::keep_or_approve).collect();
            decided.extend(record(path, decisions, &stale)?);
        }
        Mode::Picked { picks, crate_dir } => {
            let choices = picked(&pending, picks, crate_dir)?;
            // Only what's decided is compared again before it's recorded.
            let before = pending
                .iter()
                .zip(&choices)
                .map(|((item, _), c)| match c {
                    Some(_) => snapshot(item),
                    None => Ok(String::new()),
                })
                .collect::<Result<Vec<_>>>()?;
            decided.extend(settle(
                path, pending, choices, &before, &stale, known, root,
            )?);
        }
        Mode::Interactive { tool, crate_dir } if !pending.is_empty() => {
            let before = pending
                .iter()
                .map(|(item, _)| snapshot(item))
                .collect::<Result<Vec<_>>>()?;
            let hints: Vec<Option<Known>> = pending
                .iter()
                .map(|(item, _)| known_for(known, item).cloned())
                .collect();
            match tui::run(tool, &pending, &hints, crate_dir)? {
                Some(choices) => decided.extend(settle(
                    path, pending, choices, &before, &stale, known, root,
                )?),
                // Cancelled.
                None => report(path, &pending, &stale, known, root, false)?,
            }
        }
        _ => report(
            path,
            &pending,
            &stale,
            known,
            root,
            matches!(mode, Mode::Check),
        )?,
    }
    Ok(decided)
}

/// The decision `picks` make for each pending item, if any. Fails, before anything is
/// written, on a pick that matches nothing pending, an item picked twice, a hash that isn't
/// the item's, or an override file that isn't there.
fn picked(
    pending: &[Pending],
    picks: &[Pick],
    crate_dir: &Path,
) -> Result<Vec<Option<tui::Choice>>> {
    let mut choices: Vec<Option<tui::Choice>> = pending.iter().map(|_| None).collect();
    for pick in picks {
        let mut found = false;
        for ((item, _), choice) in pending.iter().zip(choices.iter_mut()) {
            if !pick.selects(item) {
                continue;
            }
            found = true;
            if let Some(hash) = &pick.hash
                && *hash != item.hash
            {
                bail!(
                    "{} is now {}, not {}; read it again",
                    shown_line(&item.describe()),
                    item.hash,
                    shown_line(hash)
                );
            }
            if choice.is_some() {
                bail!("{} is picked twice", shown_line(&item.describe()));
            }
            let file = match pick.action {
                Action::Override => {
                    let rel = tui::override_rel(item)?;
                    if !crate_dir.join(&rel).is_file() {
                        bail!(
                            "the override for {} isn't there yet: write {}",
                            shown_line(&item.describe()),
                            crate_dir.join(&rel).display()
                        );
                    }
                    Some(rel)
                }
                _ => None,
            };
            *choice = Some(tui::Choice {
                action: pick.action,
                file,
            });
        }
        if !found {
            let what = match pick.action {
                Action::Approve => "",
                _ => "build script or cargo config ",
            };
            bail!("no pending {what}matches {}", shown_line(&pick.spec));
        }
    }
    Ok(choices)
}

/// Record the choices made on the review screen, then fail listing what's still undecided.
/// `before` holds each pending item's `snapshot` from before the screen opened.
fn settle(
    path: &Path,
    pending: Vec<Pending>,
    choices: Vec<Option<tui::Choice>>,
    before: &[String],
    stale: &[&Entry],
    known: &KnownMap,
    root: &Path,
) -> Result<Vec<Decided>> {
    let mut decisions = Vec::new();
    let mut rest = Vec::new();
    for (((item, previous), choice), before) in pending.into_iter().zip(choices).zip(before) {
        match choice {
            Some(c) => {
                unchanged(&item, before)?;
                decisions.push(Decision {
                    item,
                    previous,
                    action: c.action,
                    file: c.file,
                });
            }
            None => rest.push((item, previous)),
        }
    }
    // The entries of undecided items stay until they're decided.
    let keep: Vec<&Entry> = rest.iter().filter_map(|(_, p)| *p).collect();
    let unused: Vec<&Entry> = stale
        .iter()
        .copied()
        .filter(|e| !keep.contains(e))
        .collect();
    let decided = record(path, decisions, &unused)?;
    report(path, &rest, &[], known, root, false)?;
    Ok(decided)
}

/// A hash of what's on disk for `item`: the same as `item.hash` for build scripts and
/// config, and every file of the crate for a proc macro, whose `hash` is its lock checksum
/// or commit.
fn snapshot(item: &Item) -> Result<String> {
    match item.kind {
        Kind::BuildScript => script_hash(&item.base, &item.location, &item.pulled, item.vendored),
        Kind::CargoConfig => Ok(sha256_hex(&fs::read(&item.location)?)),
        Kind::ProcMacro | Kind::BuildDep => dir_hash(&item.location, item.vendored),
    }
}

/// Fail when a piece's content changed while it was on the review screen, or, for build
/// scripts and config, since it was collected.
fn unchanged(item: &Item, before: &str) -> Result<()> {
    let now = snapshot(item)?;
    if now != before || (!item.kind.whole_crate() && now != item.hash) {
        bail!(
            "{} changed during the review; run the command again",
            shown(&item.describe())
        );
    }
    Ok(())
}

/// Warn about unused entries, and fail listing what's pending. All of it goes to stderr, so
/// the list comes before the error on the same stream.
/// `review` says to settle them with the `review` command rather than by running the same
/// command again.
fn report(
    path: &Path,
    pending: &[Pending],
    stale: &[&Entry],
    known: &KnownMap,
    root: &Path,
    review: bool,
) -> Result<()> {
    let rel = |p: &Path| shown(&p.strip_prefix(root).unwrap_or(p).display().to_string());
    // Records carry the raw text, which JSON escapes; humans get it `shown`.
    let raw = |p: &Path| slash(p.strip_prefix(root).unwrap_or(p));
    // `crates/<tool>/build-review.toml`
    let tool = path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    for e in stale {
        if !pending.iter().any(|(_, prev)| *prev == Some(*e)) {
            output::emit(Record::ReviewUnused {
                tool: tool.clone(),
                file: raw(path),
                line: e.line,
                kind: e.kind.table().to_string(),
                package: e.package.clone(),
                path: e.path.clone(),
            });
            eprintln!(
                "  warning: unused [[{}]] {}{} in {}; delete it",
                e.kind,
                shown(&e.package),
                shown(&e.path),
                rel(path)
            );
        }
    }
    if pending.is_empty() {
        return Ok(());
    }
    eprintln!("  build-time code to review:");
    for (item, previous) in pending {
        let note = match (known_for(known, item), previous) {
            (Some(k), _) => format!(" (reviewed for {}: {})", shown(&k.tool), k.action.as_str()),
            (None, Some(p)) if p.hash == item.hash && p.kind == Kind::BuildScript => format!(
                " (crate changed, script unchanged; was {})",
                p.action.as_str()
            ),
            (None, Some(p)) => format!(" (changed; was {})", p.action.as_str()),
            (None, None) => String::new(),
        };
        eprintln!(
            "    {:<12} {}{}  {}",
            item.kind,
            shown(&item.describe()),
            note,
            rel(&item.location)
        );
        if let Some(old) = &item.old {
            eprintln!(
                "      was {}  {}",
                shown(&old.version),
                rel(&old.location(item))
            );
        }
    }
    eprintln!(
        "  to decide piece by piece, put these in {} with the action you choose:",
        rel(path)
    );
    for (item, previous) in pending {
        // Never an absolute path in a record: what's built lives under the project.
        if !item.location.starts_with(root) {
            bail!(
                "{} is outside the project",
                shown_line(&item.location.to_string_lossy())
            );
        }
        let found = known_for(known, item);
        let text = paste_ready(item, *previous, found, root);
        eprint!("{}", shown(&text));
        output::emit(Record::ReviewPending {
            tool: tool.clone(),
            file: raw(path),
            line: previous.and_then(|p| p.line),
            item: Box::new(review_item(
                item,
                *previous,
                found,
                raw(&item.location),
                item.old.as_ref().map(|o| raw(&o.location(item))),
            )),
            entry: text
                .lines()
                .map(|l| format!("{}\n", l.trim_start()))
                .collect(),
        });
    }
    let what = match pending.len() {
        1 => "1 piece of build-time code isn't".to_string(),
        n => format!("{n} pieces of build-time code aren't"),
    };
    if review {
        let tool = shown(&tool);
        bail!(
            "{what} recorded in {}; run `cargo project-tools review {tool}` where you have a \
             terminal for the review screen, or with --approve, --strip or --override \
             <package@version> for each decision, then commit {}",
            rel(path),
            rel(path)
        )
    }
    if screen::interactive() {
        bail!(
            "{what} recorded in {}; review it, then rerun with --auto-approve",
            rel(path)
        )
    }
    bail!(
        "{what} recorded in {}; run this command where you have a terminal for the review \
         screen, or record each decision with `cargo project-tools review {} --approve, \
         --strip or --override <package@version>`, then commit {}",
        rel(path),
        shown(&tool),
        rel(path)
    )
}

/// A pending item as a `review-pending` record describes it.
fn review_item(
    item: &Item,
    previous: Option<&Entry>,
    known: Option<&Known>,
    location: String,
    old_location: Option<String>,
) -> output::ReviewItem {
    let hash = |kind: Kind, hash: &str| match kind {
        Kind::ProcMacro | Kind::BuildDep => (None, Some(hash.to_string())),
        _ => (Some(hash.to_string()), None),
    };
    let (sha256, mut checksum) = hash(item.kind, &item.hash);
    let mut commit = None;
    match &item.pin {
        Pin::Checksum(c) => checksum = Some(c.clone()),
        Pin::Commit(c) => commit = Some(c.clone()),
        Pin::None => {}
    }
    output::ReviewItem {
        kind: item.kind.table().to_string(),
        package: item.package.clone(),
        version: item.version.clone(),
        path: item.path.clone(),
        sha256,
        checksum,
        commit,
        location,
        previous: previous.map(|p| {
            let (sha256, mut checksum) = hash(p.kind, &p.hash);
            let mut commit = None;
            match &p.pin {
                Pin::Checksum(c) => checksum = Some(c.clone()),
                Pin::Commit(c) => commit = Some(c.clone()),
                Pin::None => {}
            }
            output::Previous {
                action: p.action.as_str().to_string(),
                sha256,
                checksum,
                commit,
                changed: changed_keys(item, p),
                version: item.old.as_ref().map(|o| o.version.clone()),
                location: old_location.clone(),
            }
        }),
        known: known.map(|k| output::Known {
            tool: k.tool.clone(),
            action: k.action.as_str().to_string(),
        }),
    }
}

/// The keys whose values differ between a pending item and the entry it replaces.
fn changed_keys(item: &Item, previous: &Entry) -> Vec<&'static str> {
    let mut keys = Vec::new();
    if item.hash != previous.hash {
        keys.push(previous.kind.hash_key());
    }
    if item.pin != previous.pin {
        for pin in [&previous.pin, &item.pin] {
            if let Some((key, _)) = pin.key()
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
    }
    keys
}

/// The key and value that name the entry a pending item replaces: its hash, or, when only
/// the crate changed, its pin.
fn replaced_key<'a>(item: &Item, previous: &'a Entry) -> (&'static str, &'a str) {
    if item.hash == previous.hash
        && let Some(pin) = previous.pin.key()
    {
        return pin;
    }
    (previous.kind.hash_key(), &previous.hash)
}

/// The review-file entry for a pending item, indented: with the decision another tool
/// recorded for it, else the action of the entry it replaces, else `approve`.
fn paste_ready(
    item: &Item,
    previous: Option<&Entry>,
    known: Option<&Known>,
    root: &Path,
) -> String {
    let mut doc = toml_edit::DocumentMut::new();
    let mut out = String::new();
    let (action, file) = match known {
        Some(k) => {
            out.push_str(&format!("    # as recorded for {}\n", shown_line(&k.tool)));
            let ours = (k.action == Action::Override)
                .then(|| tui::override_rel(item).ok())
                .flatten();
            if let (Some(theirs), Some(ours)) = (&k.file, &ours) {
                let theirs = theirs.strip_prefix(root).unwrap_or(theirs);
                // One comment line each, whatever the paths hold.
                out.push_str(&format!(
                    "    # copy {} to {} in this tool's crate directory\n",
                    shown_line(&theirs.to_string_lossy()),
                    shown_line(ours)
                ));
            }
            match ours {
                Some(f) => (Action::Override, Some(f)),
                None if k.action == Action::Override => (Action::Approve, None),
                None => (k.action, None),
            }
        }
        None => (
            previous.map_or(Action::Approve, |p| p.action),
            previous.and_then(|p| p.file.clone()),
        ),
    };
    add_entry(&mut doc, item, action, file.as_deref());
    if let Some(p) = previous {
        let (key, value) = replaced_key(item, p);
        out.push_str(&format!(
            "    # replaces the entry with {key} = \"{}\"\n",
            shown_line(value)
        ));
    }
    for line in doc.to_string().lines() {
        out.push_str(&format!("    {line}\n"));
    }
    out
}

/// Write `decisions` into the review file and drop `stale` entries. Returns what was decided.
fn record(path: &Path, decisions: Vec<Decision>, stale: &[&Entry]) -> Result<Vec<Decided>> {
    if decisions.is_empty() && stale.is_empty() {
        return Ok(Vec::new());
    }
    // A new file gets the header; it's added after editing, as a comment-only document's
    // text would end up after the tables.
    let (text, header) = match fs::read_to_string(path) {
        Ok(t) => (t, ""),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), TEMPLATE),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    let mut decided = Vec::new();
    // An entry whose content changed is updated in place, so comments on it stay.
    let mut reused: Vec<&Entry> = Vec::new();
    for Decision {
        item,
        previous,
        action,
        file,
    } in decisions
    {
        match previous.filter(|p| !reused.contains(p)) {
            Some(p) => {
                update_entry(&mut doc, p, &item, action, file.as_deref());
                reused.push(p);
            }
            None => add_entry(&mut doc, &item, action, file.as_deref()),
        }
        say!(
            "  recorded {:<12} {} ({})",
            item.kind,
            shown_line(&item.describe()),
            action.as_str()
        );
        decided.push(Decided { item, action, file });
    }
    for e in stale.iter().filter(|e| !reused.contains(e)) {
        remove_entry(&mut doc, e);
        say!(
            "  removed unused [[{}]] {}",
            e.kind,
            shown_line(&format!("{}{}", e.package, e.path))
        );
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, format!("{header}{doc}"))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(decided)
}

fn tables_of(doc: &mut toml_edit::DocumentMut, kind: Kind) -> &mut toml_edit::ArrayOfTables {
    let item = doc
        .entry(kind.table())
        .or_insert_with(|| toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()));
    if !item.is_array_of_tables() {
        *item = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    item.as_array_of_tables_mut()
        .expect("just made an array of tables")
}

/// Whether table `t` is the one `e` was read from.
fn is_entry(t: &toml_edit::Table, e: &Entry) -> bool {
    let get = |k: &str| t.get(k).and_then(|v| v.as_str());
    let pin = match e.kind {
        Kind::BuildScript => match &e.pin {
            Pin::None => get("checksum").is_none() && get("commit").is_none(),
            p => p.key().is_some_and(|(k, v)| get(k) == Some(v)),
        },
        _ => true,
    };
    get("package").unwrap_or("") == e.package
        && get("path").unwrap_or("") == e.path
        && get(e.kind.hash_key()) == Some(e.hash.as_str())
        && pin
}

/// Write `pin` into a build-script table, replacing any other.
fn set_pin(t: &mut toml_edit::Table, pin: &Pin) {
    t.remove("checksum");
    t.remove("commit");
    if let Some((k, v)) = pin.key() {
        t[k] = toml_edit::value(v);
    }
}

/// Point entry `e` at `item`'s content and the new decision, keeping its place and comments.
fn update_entry(
    doc: &mut toml_edit::DocumentMut,
    e: &Entry,
    item: &Item,
    action: Action,
    file: Option<&str>,
) {
    if let Some(t) = tables_of(doc, e.kind).iter_mut().find(|t| is_entry(t, e)) {
        t[e.kind.hash_key()] = toml_edit::value(item.hash.as_str());
        if e.kind == Kind::BuildScript {
            set_pin(t, &item.pin);
        }
        t["action"] = toml_edit::value(action.as_str());
        match file {
            Some(f) => t["file"] = toml_edit::value(f),
            None => {
                t.remove("file");
            }
        }
    }
}

fn remove_entry(doc: &mut toml_edit::DocumentMut, e: &Entry) {
    tables_of(doc, e.kind).retain(|t| !is_entry(t, e));
    if tables_of(doc, e.kind).is_empty() {
        doc.remove(e.kind.table());
    }
}

fn add_entry(doc: &mut toml_edit::DocumentMut, item: &Item, action: Action, file: Option<&str>) {
    let mut t = toml_edit::Table::new();
    if !item.package.is_empty() {
        t["package"] = toml_edit::value(item.package.as_str());
    }
    if !item.path.is_empty() {
        t["path"] = toml_edit::value(item.path.as_str());
    }
    t[item.kind.hash_key()] = toml_edit::value(item.hash.as_str());
    if item.kind == Kind::BuildScript {
        set_pin(&mut t, &item.pin);
    }
    t["action"] = toml_edit::value(action.as_str());
    if let Some(f) = file {
        t["file"] = toml_edit::value(f);
    }
    tables_of(doc, item.kind).push(t);
}

/// Carry out override and strip decisions on the checkout and vendored sources. `crate_dir`
/// holds the override files. Returns a digest of the changes, empty when there are none.
pub fn apply(decided: &[Decided], crate_dir: &Path) -> Result<String> {
    let mut changes = Vec::new();
    for d in decided {
        let item = &d.item;
        match d.action {
            Action::Approve => continue,
            Action::Override => {
                let rel = d.file.as_deref().context("override without a file")?;
                let src = crate_dir.join(rel);
                let content = fs::read(&src)
                    .with_context(|| format!("reading override {}", src.display()))?;
                let first = content.split(|b| *b == b'\n').next().unwrap_or_default();
                if String::from_utf8_lossy(first).contains(tui::UNREVIEWED) {
                    bail!(
                        "override {} is still the commented-out copy; edit it first",
                        src.display()
                    );
                }
                fs::write(&item.location, &content)
                    .with_context(|| format!("writing {}", item.location.display()))?;
                if item.vendored {
                    drop_checksum(&item.base, &item.path)?;
                }
                changes.push(format!(
                    "{}\0{}\0{}\0override\0{}",
                    item.kind,
                    item.package,
                    item.path,
                    sha256_hex(&content)
                ));
            }
            Action::Strip => {
                match item.kind {
                    Kind::BuildScript => {
                        strip_build(&item.base)?;
                        if item.vendored {
                            drop_checksum(&item.base, "Cargo.toml")?;
                        }
                    }
                    Kind::CargoConfig => fs::remove_file(&item.location)
                        .with_context(|| format!("removing {}", item.location.display()))?,
                    Kind::ProcMacro | Kind::BuildDep => bail!("a whole crate can't be stripped"),
                }
                changes.push(format!(
                    "{}\0{}\0{}\0strip",
                    item.kind, item.package, item.path
                ));
            }
        }
    }
    if changes.is_empty() {
        return Ok(String::new());
    }
    changes.sort();
    Ok(sha256_hex(changes.join("\n").as_bytes()))
}

/// Turn off the build script in a package's manifest.
fn strip_build(pkg_dir: &Path) -> Result<()> {
    let path = pkg_dir.join("Cargo.toml");
    let mut doc: toml_edit::DocumentMut = fs::read_to_string(&path)?.parse()?;
    let package = doc
        .get_mut("package")
        .and_then(|p| p.as_table_like_mut())
        .with_context(|| format!("{} has no [package]", path.display()))?;
    package.insert("build", toml_edit::value(false));
    fs::write(&path, doc.to_string())?;
    Ok(())
}

/// Stop cargo checking a vendored file we changed. The package checksum, which cargo
/// compares with the lock, stays.
fn drop_checksum(pkg_dir: &Path, rel: &str) -> Result<()> {
    let path = pkg_dir.join(".cargo-checksum.json");
    let mut json: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    json.get_mut("files")
        .and_then(|f| f.as_object_mut())
        .with_context(|| format!("{} has no files", path.display()))?
        .remove(rel);
    fs::write(&path, serde_json::to_string(&json)?)?;
    Ok(())
}

/// Review the build-time code of the tool in `checkout`: collect, check against its
/// `build-review.toml`, and apply the decisions. Returns the digest from `apply`.
pub fn review(
    project: &Project,
    name: &str,
    selection: graph::Selection,
    checkout: &Checkout,
    vendored: &Vendored,
    offline: bool,
    approval: Approval,
) -> Result<String> {
    source::refuse_config_above(&project.root, checkout)?;
    let mut items = collect(name, selection, checkout, vendored, offline)?;
    let crate_dir = project.crate_dir(name);
    let mode = match approval {
        Approval::All => Mode::ApproveAll,
        Approval::Picked(picks) => Mode::Picked {
            picks,
            crate_dir: &crate_dir,
        },
        Approval::Ask if screen::interactive() => Mode::Interactive {
            tool: name,
            crate_dir: &crate_dir,
        },
        Approval::Ask => Mode::Report,
    };
    // Without a terminal, the list and records point at the old version to compare with.
    if matches!(mode, Mode::Report) {
        previous::fetch(project, name, checkout, &mut items, offline)?;
    }
    let known = known_elsewhere(
        &project.tools_dir().join("crates"),
        name,
        &configured_tools(project)?,
    );
    let decided = check(
        &project.review_path(name),
        items,
        mode,
        &known,
        &project.root,
    )?;
    apply(&decided, &crate_dir)
}

/// Show how each changed piece of the tool's build-time code differs from the version its
/// entry was recorded for (`review --diff`). Records nothing.
pub fn diff(
    project: &Project,
    name: &str,
    selection: graph::Selection,
    checkout: &Checkout,
    vendored: &Vendored,
    offline: bool,
) -> Result<()> {
    source::refuse_config_above(&project.root, checkout)?;
    let mut items = collect(name, selection, checkout, vendored, offline)?;
    previous::fetch(project, name, checkout, &mut items, offline)?;
    previous::print_diffs(name, &items)
}

/// Check the build-time code of the tool in `checkout` against its `build-review.toml`,
/// without a review screen, and without recording or applying anything.
pub fn check_only(
    project: &Project,
    name: &str,
    selection: graph::Selection,
    checkout: &Checkout,
    vendored: &Vendored,
    offline: bool,
) -> Result<()> {
    source::refuse_config_above(&project.root, checkout)?;
    let items = collect(name, selection, checkout, vendored, offline)?;
    let known = known_elsewhere(
        &project.tools_dir().join("crates"),
        name,
        &configured_tools(project)?,
    );
    check(
        &project.review_path(name),
        items,
        Mode::Check,
        &known,
        &project.root,
    )?;
    Ok(())
}

/// Digest of the review file and its override files, for the install stamp.
pub fn file_digest(project: &Project, name: &str) -> Result<String> {
    let path = project.review_path(name);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut parts = vec![sha256_hex(text.as_bytes())];
    let entries = parse(&text).with_context(|| format!("parsing {}", path.display()))?;
    for e in entries {
        if let Some(f) = e.file {
            let src = project.crate_dir(name).join(&f);
            let bytes =
                fs::read(&src).with_context(|| format!("reading override {}", src.display()))?;
            parts.push(sha256_hex(&bytes));
        }
    }
    Ok(sha256_hex(parts.join("\0").as_bytes()))
}

#[cfg(test)]
#[path = "review_test.rs"]
mod tests;
