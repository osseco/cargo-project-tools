use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Profile keys that may be set from `[build.profile-settings]`.
const PROFILE_KEYS: &[&str] = &[
    "inherits",
    "opt-level",
    "debug",
    "strip",
    "lto",
    "codegen-units",
    "panic",
    "incremental",
    "debug-assertions",
    "overflow-checks",
    "split-debuginfo",
    "rpath",
];

pub const TEMPLATE: &str = r#"# Tools managed by `cargo project-tools`.

[settings]
# Run network commands through sfw (Socket Firewall Free): "auto", "require" or "off".
sfw = "auto"
# Revalidate the advisory database when it was last checked longer ago than this.
# advisory-db-max-age = "1d"

[build]
profile = "release"

[build.profile-settings]
strip = "symbols"
lto = "thin"
codegen-units = 1

# [tool.cargo-nextest]
# version = "0.9.80"
"#;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub build: Build,
    #[serde(default)]
    pub tool: BTreeMap<String, Tool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Settings {
    #[serde(default)]
    pub sfw: SfwMode,
    /// How old the advisory database may get before it's revalidated, e.g. `"12h"`, `"1d"`.
    pub advisory_db_max_age: Option<String>,
    /// Target triples tools are reviewed, audited and installed for; all when unset.
    pub targets: Option<Vec<String>>,
}

const DEFAULT_ADVISORY_DB_MAX_AGE: Duration = Duration::from_secs(86_400);

impl Settings {
    pub fn advisory_db_max_age(&self) -> Result<Duration> {
        self.advisory_db_max_age
            .as_deref()
            .map_or(Ok(DEFAULT_ADVISORY_DB_MAX_AGE), parse_max_age)
    }
}

/// `<n>h` or `<n>d`.
pub fn parse_max_age(s: &str) -> Result<Duration> {
    let (num, secs) = if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('d') {
        (n, 86_400)
    } else {
        bail!("advisory-db-max-age `{s}`: use hours or days, like \"12h\" or \"1d\"");
    };
    let n: u64 = num
        .parse()
        .with_context(|| format!("advisory-db-max-age `{s}`: not a number of hours or days"))?;
    Ok(Duration::from_secs(n * secs))
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SfwMode {
    #[default]
    Auto,
    Require,
    Off,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Build {
    pub profile: Option<String>,
    pub target: Option<String>,
    pub rustflags: Option<Vec<String>>,
    #[serde(default)]
    pub profile_settings: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Tool {
    pub version: String,
    pub git: Option<String>,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub rev: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    pub default_features: Option<bool>,
    #[serde(default)]
    pub bins: Vec<String>,
    pub build: Option<Build>,
    /// Replaces `[settings].targets` for this tool.
    pub targets: Option<Vec<String>>,
}

impl Build {
    /// `self` with `over` applied on top.
    pub fn merged(&self, over: Option<&Build>) -> Build {
        let mut out = self.clone();
        if let Some(over) = over {
            if over.profile.is_some() {
                out.profile.clone_from(&over.profile);
            }
            if over.target.is_some() {
                out.target.clone_from(&over.target);
            }
            if over.rustflags.is_some() {
                out.rustflags.clone_from(&over.rustflags);
            }
            for (k, v) in &over.profile_settings {
                out.profile_settings.insert(k.clone(), v.clone());
            }
        }
        out
    }

    fn validate(&self, context: &str) -> Result<()> {
        for key in self.profile_settings.keys() {
            if !PROFILE_KEYS.contains(&key.as_str()) {
                bail!(
                    "{context}: unknown profile setting `{key}` (allowed: {})",
                    PROFILE_KEYS.join(", ")
                );
            }
        }
        Ok(())
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: Config =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.settings.advisory_db_max_age()?;
        config.build.validate("[build]")?;
        validate_targets(config.settings.targets.as_deref(), "[settings]")?;
        for (name, tool) in &config.tool {
            if let Some(build) = &tool.build {
                build.validate(&format!("[tool.{name}.build]"))?;
            }
            validate_targets(tool.targets.as_deref(), &format!("[tool.{name}]"))?;
            if let Some(url) = &tool.git {
                ensure_https(url)?;
            }
        }
        Ok(config)
    }

    /// The targets `tool` is reviewed and installed for: its own list, else the project's.
    /// `None` means every target.
    pub fn targets_for<'a>(&'a self, tool: &'a Tool) -> Option<&'a [String]> {
        tool.targets.as_deref().or(self.settings.targets.as_deref())
    }

    pub fn tool(&self, name: &str) -> Result<&Tool> {
        self.tool
            .get(name)
            .with_context(|| format!("no tool `{name}` in project-tools.toml"))
    }

    /// Tools selected by name, or all when `names` is empty.
    pub fn select<'a>(&'a self, names: &[String]) -> Result<Vec<(&'a str, &'a Tool)>> {
        if names.is_empty() {
            return Ok(self.tool.iter().map(|(k, v)| (k.as_str(), v)).collect());
        }
        names
            .iter()
            .map(|n| {
                self.tool
                    .get_key_value(n.as_str())
                    .map(|(k, v)| (k.as_str(), v))
                    .with_context(|| format!("no tool `{n}` in project-tools.toml"))
            })
            .collect()
    }
}

fn validate_targets(targets: Option<&[String]>, context: &str) -> Result<()> {
    if let Some(list) = targets {
        if list.is_empty() {
            bail!(
                "{context}: `targets` is empty; list at least one target triple, or remove it to allow all"
            );
        }
        if let Some(t) = list
            .iter()
            .find(|t| t.is_empty() || t.starts_with('-') || t.contains(char::is_whitespace))
        {
            bail!("{context}: `{t}` is not a target triple");
        }
    }
    Ok(())
}

pub fn ensure_https(url: &str) -> Result<()> {
    if !url.starts_with("https://") {
        bail!("only https:// git URLs are allowed, got `{url}`");
    }
    Ok(())
}

/// Edits to `project-tools.toml` that keep comments and formatting.
pub struct ConfigEditor {
    path: std::path::PathBuf,
    doc: toml_edit::DocumentMut,
}

impl ConfigEditor {
    pub fn open(path: &Path) -> Result<Self> {
        let text =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let doc = text
            .parse()
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(ConfigEditor {
            path: path.to_path_buf(),
            doc,
        })
    }

    fn tools(&mut self) -> &mut toml_edit::Table {
        let item = self.doc.entry("tool").or_insert_with(|| {
            let mut t = toml_edit::Table::new();
            t.set_implicit(true);
            toml_edit::Item::Table(t)
        });
        item.as_table_mut().expect("[tool] must be a table")
    }

    pub fn has_tool(&mut self, name: &str) -> bool {
        self.tools().contains_key(name)
    }

    pub fn set_tool(&mut self, name: &str, entries: &[(&str, toml_edit::Value)]) {
        let tools = self.tools();
        let table = tools
            .entry(name)
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
            .as_table_mut()
            .expect("[tool.<name>] must be a table");
        for (k, v) in entries {
            table.insert(k, toml_edit::Item::Value(v.clone()));
        }
    }

    /// Add `triples` to `[tool.<tool>].targets` when given, else to `[settings].targets`.
    pub fn add_targets(&mut self, tool: Option<&str>, triples: &[String]) {
        let table = match tool {
            Some(name) => self
                .tools()
                .entry(name)
                .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
                .as_table_mut()
                .expect("[tool.<name>] must be a table"),
            None => self
                .doc
                .entry("settings")
                .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
                .as_table_mut()
                .expect("[settings] must be a table"),
        };
        let item = table
            .entry("targets")
            .or_insert_with(|| toml_edit::value(toml_edit::Array::new()));
        if let Some(list) = item.as_array_mut() {
            for t in triples {
                if !list.iter().any(|v| v.as_str() == Some(t)) {
                    list.push(t.as_str());
                }
            }
        }
    }

    pub fn remove_tool(&mut self, name: &str) -> bool {
        self.tools().remove(name).is_some()
    }

    pub fn save(&self) -> Result<()> {
        fs::write(&self.path, self.doc.to_string())
            .with_context(|| format!("writing {}", self.path.display()))
    }
}

pub fn string_array(values: &[String]) -> toml_edit::Value {
    toml_edit::Value::Array(values.iter().collect())
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditIgnore {
    #[serde(default)]
    pub ignore: Vec<IgnoreEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IgnoreEntry {
    pub id: String,
    pub reason: String,
    /// `YYYY-MM-DD`; after this date the entry no longer applies.
    pub expires: Option<String>,
}

pub const AUDIT_IGNORE_TEMPLATE: &str = r#"# Advisories to ignore when running `cargo project-tools audit` for this tool.
#
# [[ignore]]
# id = "RUSTSEC-0000-0000"
# reason = "why this does not affect us"
# expires = "2027-01-01"
"#;

/// File name of the ignore list for advisories against cargo-project-tools itself.
pub const SELF_AUDIT_IGNORE: &str = "self-audit-ignore.toml";

impl AuditIgnore {
    /// Like `load`, but every entry needs `expires`, so an ignored advisory against the tool
    /// itself can't be forgotten.
    pub fn load_self(path: &Path) -> Result<AuditIgnore> {
        let file = AuditIgnore::load(path)?;
        if let Some(e) = file.ignore.iter().find(|e| e.expires.is_none()) {
            bail!(
                "{}: ignore for {} needs an `expires` date; ignores for cargo-{} itself must end",
                path.display(),
                e.id,
                crate::cli::SUBCOMMAND
            );
        }
        Ok(file)
    }

    pub fn load(path: &Path) -> Result<AuditIgnore> {
        match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AuditIgnore::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod tests;
