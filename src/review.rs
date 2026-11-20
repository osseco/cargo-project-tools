use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::graph;
use crate::lock;
use crate::process;
use crate::project::Project;
use crate::source::registry::sha256_hex;
use crate::source::{self, Checkout};
use crate::vendor::Vendored;

/// Per-tool file recording the build-time code that may run, next to the tool's lock.
pub const FILE: &str = "build-review.toml";

const TEMPLATE: &str =
    "# Code that runs while building this tool. `install` halts on anything not listed here.
# Review it, then record it with `--approve-build`. See docs/build-review.md.

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
    CargoConfig,
}

impl Kind {
    /// The array-of-tables name in the review file.
    fn table(self) -> &'static str {
        match self {
            Kind::BuildScript => "build-script",
            Kind::ProcMacro => "proc-macro",
            Kind::CargoConfig => "cargo-config",
        }
    }

    /// The field that pins the content.
    fn hash_key(self) -> &'static str {
        match self {
            Kind::ProcMacro => "checksum",
            _ => "sha256",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.table())
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
    /// What to review on disk.
    pub location: PathBuf,
    /// The package directory for build scripts and proc macros, the checkout root for config.
    pub base: PathBuf,
    /// In the vendor directory, where edits must be dropped from `.cargo-checksum.json`.
    pub vendored: bool,
}

impl Item {
    fn same_place(&self, e: &Entry) -> bool {
        self.kind == e.kind && self.package == e.package && self.path == e.path
    }

    fn matches(&self, e: &Entry) -> bool {
        self.same_place(e) && self.hash == e.hash
    }

    fn describe(&self) -> String {
        match self.kind {
            Kind::BuildScript => format!("{}@{} {}", self.package, self.version, self.path),
            Kind::ProcMacro => format!("{}@{}", self.package, self.version),
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
    pub action: Action,
    /// With `override`: the replacement, relative to the tool's crate directory.
    pub file: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    package: Option<String>,
    path: Option<String>,
    sha256: Option<String>,
    checksum: Option<String>,
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
    cargo_config: Vec<RawEntry>,
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
        (Kind::CargoConfig, raw.cargo_config),
    ] {
        for r in list {
            out.push(entry(kind, r)?);
        }
    }
    Ok(out)
}

fn entry(kind: Kind, r: RawEntry) -> Result<Entry> {
    let need = |v: Option<String>, field: &str| {
        v.with_context(|| format!("a [[{kind}]] entry needs `{field}`"))
    };
    let unexpected = match kind {
        Kind::BuildScript => r.checksum.is_some(),
        Kind::ProcMacro => r.path.is_some() || r.sha256.is_some(),
        Kind::CargoConfig => r.package.is_some() || r.checksum.is_some(),
    };
    if unexpected {
        bail!("a [[{kind}]] entry has fields that belong to another kind");
    }
    let (package, path, hash) = match kind {
        Kind::BuildScript => (
            need(r.package, "package")?,
            need(r.path, "path")?,
            need(r.sha256, "sha256")?,
        ),
        Kind::ProcMacro => (
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
    if kind == Kind::ProcMacro && r.action != Action::Approve {
        bail!("[[proc-macro]] `{package}` can only be approved");
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
        action: r.action,
        file: r.file,
    })
}

fn safe_relative(p: &str) -> Result<()> {
    let path = Path::new(p);
    if p.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        bail!("unsafe override path `{p}`");
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

/// sha256 lines (`<hash>  <path>`, like `sha256sum`) for every file under `dir`, sorted, and
/// hashed. Links are recorded by their target; `.git` is skipped.
fn dir_hash(dir: &Path) -> Result<String> {
    fn walk(root: &Path, dir: &Path, lines: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let rel = slash(path.strip_prefix(root)?);
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                let target = fs::read_link(&path)?;
                lines.push(format!("link {} -> {}\n", rel, target.display()));
            } else if kind.is_dir() {
                if entry.file_name() != ".git" {
                    walk(root, &path, lines)?;
                }
            } else {
                lines.push(format!("{}  {}\n", sha256_hex(&fs::read(&path)?), rel));
            }
        }
        Ok(())
    }
    let mut lines = Vec::new();
    walk(dir, dir, &mut lines)?;
    lines.sort();
    Ok(sha256_hex(lines.concat().as_bytes()))
}

/// The build script's own file when it sits at the crate root, else its whole directory,
/// which is where the modules it uses usually live.
fn script_hash(pkg_dir: &Path, script: &Path) -> Result<String> {
    let parent = script
        .parent()
        .context("build script without a directory")?;
    if parent == pkg_dir {
        Ok(sha256_hex(&fs::read(script)?))
    } else {
        dir_hash(parent)
    }
}

/// Cargo config files a build of the checkout would read. Links are refused, and so is
/// `include`, which would pull in files nobody reviewed.
fn shipped_configs(checkout: &Checkout) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    for dir in source::search_dirs(checkout) {
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
            items.push(Item {
                kind: Kind::CargoConfig,
                package: String::new(),
                version: String::new(),
                path: slash(path.strip_prefix(&checkout.root)?),
                hash: sha256_hex(&bytes),
                location: path,
                base: checkout.root.clone(),
                vendored: false,
            });
        }
    }
    Ok(items)
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetaPackage>,
}

#[derive(Deserialize)]
struct MetaPackage {
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

/// Every build script and proc macro the tool builds with `features`, and the cargo config
/// it ships. Uses `cargo tree` and `cargo metadata` on the vendored sources: nothing is
/// compiled and nothing runs.
pub fn collect(
    name: &str,
    features: &[String],
    checkout: &Checkout,
    vendored: &Vendored,
    offline: bool,
) -> Result<Vec<Item>> {
    let mut items = shipped_configs(checkout)?;
    let manifest = checkout.manifest_path();
    let built = graph::tree(&manifest, features, &vendored.config_args, offline, name)?;

    let mut cmd = process::cargo();
    cmd.args([
        "metadata",
        "--locked",
        "--format-version",
        "1",
        "--manifest-path",
    ])
    .arg(&manifest)
    .args(&vendored.config_args);
    if offline {
        cmd.arg("--offline");
    }
    let json = process::output(&mut cmd).context("reading package metadata")?;
    let meta: Metadata = serde_json::from_str(&json)?;

    let lock = lock::load(&checkout.lock_path())?;
    let checksums: BTreeMap<(String, String), String> = lock
        .packages
        .iter()
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
        let in_vendor = pkg_dir.starts_with(&vendored.dir);
        for t in &pkg.targets {
            if t.kind.iter().any(|k| k == "custom-build") {
                let rel = t.src_path.strip_prefix(&pkg_dir).with_context(|| {
                    format!("build script of {} is outside its package", pkg.name)
                })?;
                items.push(Item {
                    kind: Kind::BuildScript,
                    package: pkg.name.clone(),
                    version: pkg.version.clone(),
                    path: slash(rel),
                    hash: script_hash(&pkg_dir, &t.src_path)?,
                    location: t.src_path.clone(),
                    base: pkg_dir.clone(),
                    vendored: in_vendor,
                });
            }
        }
        if pkg
            .targets
            .iter()
            .any(|t| t.kind.iter().any(|k| k == "proc-macro"))
        {
            let key = (pkg.name.clone(), pkg.version.clone());
            let git_commit = pkg
                .source
                .as_deref()
                .filter(|s| s.starts_with("git+"))
                .and_then(|s| s.rsplit_once('#'))
                .map(|(_, c)| c.to_string());
            let hash = match (checksums.get(&key), git_commit) {
                (Some(sum), _) => sum.clone(),
                (None, Some(commit)) => commit,
                // A package of the tool's own workspace: pinned by its files.
                (None, None) => dir_hash(&pkg_dir)?,
            };
            items.push(Item {
                kind: Kind::ProcMacro,
                package: pkg.name.clone(),
                version: pkg.version.clone(),
                path: String::new(),
                hash,
                location: pkg_dir.clone(),
                base: pkg_dir,
                vendored: in_vendor,
            });
        }
    }
    items.sort_by(|a, b| {
        (a.kind, &a.package, &a.version, &a.path).cmp(&(b.kind, &b.package, &b.version, &b.path))
    });
    Ok(items)
}

/// An item with the decision that applies to it.
#[derive(Debug)]
pub struct Decided {
    pub item: Item,
    pub action: Action,
    pub file: Option<String>,
}

/// Match `items` against the review file at `path`. Items without a matching entry are
/// pending: without `approve` that's an error listing them; with it they're recorded, keeping
/// the action of an entry for the same place whose content changed. Entries that match
/// nothing are reported, and removed with `approve`.
pub fn check(path: &Path, items: Vec<Item>, approve: bool, root: &Path) -> Result<Vec<Decided>> {
    let entries = load(path)?;
    let stale: Vec<&Entry> = entries
        .iter()
        .filter(|e| !items.iter().any(|i| i.matches(e)))
        .collect();
    let mut decided = Vec::new();
    let mut pending = Vec::new();
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
    let shown = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();

    if !approve {
        for e in &stale {
            if !pending.iter().any(|(_, prev)| *prev == Some(*e)) {
                println!(
                    "  warning: unused [[{}]] {}{} in {}; --approve-build removes it",
                    e.kind,
                    e.package,
                    e.path,
                    shown(path)
                );
            }
        }
        if !pending.is_empty() {
            println!("  build-time code to review:");
            for (item, previous) in &pending {
                let note = match previous {
                    Some(p) => format!(" (changed; was {})", p.action.as_str()),
                    None => String::new(),
                };
                println!(
                    "    {:<12} {}{}  {}",
                    item.kind,
                    item.describe(),
                    note,
                    shown(&item.location)
                );
            }
            let what = match pending.len() {
                1 => "1 piece of build-time code isn't".to_string(),
                n => format!("{n} pieces of build-time code aren't"),
            };
            bail!(
                "{what} recorded in {}; review it, then rerun with --approve-build",
                shown(path)
            );
        }
        return Ok(decided);
    }

    if pending.is_empty() && stale.is_empty() {
        return Ok(decided);
    }
    // A new file gets the header; it's added after editing, as a comment-only document's
    // text would end up after the tables.
    let (text, header) = match fs::read_to_string(path) {
        Ok(t) => (t, ""),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), TEMPLATE),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text.parse()?;
    // An entry whose content changed is updated in place, so comments on it stay.
    let mut reused: Vec<&Entry> = Vec::new();
    for (item, previous) in pending {
        let previous = previous.filter(|p| !reused.contains(p));
        let (action, file) = match previous {
            Some(p) => {
                set_hash(&mut doc, p, &item.hash);
                reused.push(p);
                (p.action, p.file.clone())
            }
            None => {
                add_entry(&mut doc, &item, Action::Approve, None);
                (Action::Approve, None)
            }
        };
        println!(
            "  recorded {:<12} {} ({})",
            item.kind,
            item.describe(),
            action.as_str()
        );
        decided.push(Decided { item, action, file });
    }
    for e in stale.iter().filter(|e| !reused.contains(e)) {
        remove_entry(&mut doc, e);
        println!("  removed unused [[{}]] {}{}", e.kind, e.package, e.path);
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
    let get = |k: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or("");
    get("package") == e.package && get("path") == e.path && get(e.kind.hash_key()) == e.hash
}

fn set_hash(doc: &mut toml_edit::DocumentMut, e: &Entry, hash: &str) {
    if let Some(t) = tables_of(doc, e.kind).iter_mut().find(|t| is_entry(t, e)) {
        t[e.kind.hash_key()] = toml_edit::value(hash);
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
                    Kind::ProcMacro => bail!("a proc macro can't be stripped"),
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
    features: &[String],
    checkout: &Checkout,
    vendored: &Vendored,
    offline: bool,
    approve: bool,
) -> Result<String> {
    let items = collect(name, features, checkout, vendored, offline)?;
    let decided = check(&project.review_path(name), items, approve, &project.root)?;
    apply(&decided, &project.crate_dir(name))
}

/// Digest of the review file and its override files, for the install stamp.
pub fn file_digest(project: &Project, name: &str) -> Result<String> {
    let path = project.review_path(name);
    let mut parts = Vec::new();
    match fs::read(&path) {
        Ok(bytes) => parts.push(sha256_hex(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    }
    for e in load(&path)? {
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
