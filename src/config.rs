use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

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
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default)]
    pub sfw: SfwMode,
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
        config.build.validate("[build]")?;
        for (name, tool) in &config.tool {
            if let Some(build) = &tool.build {
                build.validate(&format!("[tool.{name}.build]"))?;
            }
            if let Some(url) = &tool.git {
                ensure_https(url)?;
            }
        }
        Ok(config)
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

impl AuditIgnore {
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
mod tests {
    use super::*;

    #[test]
    fn template_parses() {
        let config: Config = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(config.settings.sfw, SfwMode::Auto);
        config.build.validate("[build]").unwrap();
    }

    #[test]
    fn tool_build_overrides_merge() {
        let config: Config = toml::from_str(
            r#"
            [build]
            profile = "release"
            [build.profile-settings]
            panic = "abort"
            lto = "fat"
            [tool.x]
            version = "1.0.0"
            [tool.x.build.profile-settings]
            panic = "unwind"
            "#,
        )
        .unwrap();
        let merged = config.build.merged(config.tool["x"].build.as_ref());
        assert_eq!(merged.profile.as_deref(), Some("release"));
        assert_eq!(merged.profile_settings["panic"].as_str(), Some("unwind"));
        assert_eq!(merged.profile_settings["lto"].as_str(), Some("fat"));
    }

    #[test]
    fn unknown_profile_key_rejected() {
        let config: Config = toml::from_str("[build.profile-settings]\nopt_level = 3\n").unwrap();
        assert!(config.build.validate("[build]").is_err());
    }

    #[test]
    fn editor_keeps_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("project-tools.toml");
        fs::write(&path, TEMPLATE).unwrap();
        let mut ed = ConfigEditor::open(&path).unwrap();
        ed.set_tool("foo", &[("version", "1.2.3".into())]);
        ed.save().unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Tools managed by"));
        assert!(text.contains("[tool.foo]\nversion = \"1.2.3\""));
        let config = Config::load(&path).unwrap();
        assert_eq!(config.tool["foo"].version, "1.2.3");

        let mut ed = ConfigEditor::open(&path).unwrap();
        assert!(ed.remove_tool("foo"));
        ed.save().unwrap();
        assert!(Config::load(&path).unwrap().tool.is_empty());
    }
}
