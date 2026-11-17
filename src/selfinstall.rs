//! Versioned installs of the binary itself under `~/.cargo-project-tools`.
//!
//! Each version is kept as `bin/cargo-project-tools-X.Y.Z`, and `bin/cargo-project-tools` is
//! a relative symlink to the active one. A download is checked like `docs/verifying-releases.md`
//! describes: the sha256 against the release's `SHA256SUMS`, the cosign bundle against the
//! signer identity in `config.toml`, and the version the binary reports against the tag.

use std::env;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::checksum::file_sha256;
use crate::net;

pub const DEFAULT_REPO: &str = "osseco/cargo-project-tools";
const OIDC_ISSUER: &str = "https://token.actions.githubusercontent.com";
const LINK: &str = "cargo-project-tools";
const PREFIX: &str = "cargo-project-tools-";
const AVAILABLE: usize = 10;
const COSIGN_HINT: &str = "https://docs.sigstore.dev/cosign/system_config/installation/";

/// Where installs live and where releases are fetched from.
#[derive(Debug, Clone)]
pub struct SelfEnv {
    pub home: PathBuf,
    pub releases_base: String,
    pub api_base: String,
}

impl SelfEnv {
    /// `CARGO_PROJECT_TOOLS_HOME` or `~/.cargo-project-tools`.
    /// `CARGO_PROJECT_TOOLS_RELEASES_BASE` and `CARGO_PROJECT_TOOLS_API_BASE` replace the
    /// GitHub hosts; they must be https too.
    pub fn from_env() -> Result<Self> {
        let home = match env::var_os("CARGO_PROJECT_TOOLS_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => env::home_dir()
                .context("cannot find the home directory; set CARGO_PROJECT_TOOLS_HOME")?
                .join(".cargo-project-tools"),
        };
        let base = |name: &str, default: &str| -> Result<String> {
            let value = env::var(name)
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| default.to_string());
            if !value.starts_with("https://") {
                bail!("{name} must be an https:// URL, got {value:?}");
            }
            Ok(value.trim_end_matches('/').to_string())
        };
        Ok(Self {
            home,
            releases_base: base("CARGO_PROJECT_TOOLS_RELEASES_BASE", "https://github.com")?,
            api_base: base("CARGO_PROJECT_TOOLS_API_BASE", "https://api.github.com")?,
        })
    }

    fn bin(&self) -> PathBuf {
        self.home.join("bin")
    }
}

/// The repository releases come from, and the identity that must have signed them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfConfig {
    pub repo: String,
    pub signer: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    repo: Option<String>,
    signer: Option<String>,
}

/// The release workflow identity of `repo`.
pub fn default_signer(repo: &str) -> String {
    format!("https://github.com/{repo}/.github/workflows/release.yml@refs/heads/main")
}

impl SelfConfig {
    /// Parses `config.toml`; missing keys fall back to the defaults.
    pub fn parse(text: &str) -> Result<Self> {
        let file: ConfigFile = toml::from_str(text).context("config.toml")?;
        let repo = file.repo.unwrap_or_else(|| DEFAULT_REPO.to_string());
        if !valid_repo(&repo) {
            bail!("config.toml: repo must be owner/name, got {repo:?}");
        }
        let signer = file.signer.unwrap_or_else(|| default_signer(&repo));
        if !signer.starts_with("https://") {
            bail!("config.toml: signer must be an https:// workflow identity, got {signer:?}");
        }
        Ok(Self { repo, signer })
    }

    fn load(env: &SelfEnv) -> Result<Self> {
        let path = env.home.join("config.toml");
        match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == ErrorKind::NotFound => Self::parse(""),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
}

fn valid_repo(repo: &str) -> bool {
    let ok = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    matches!(repo.split_once('/'), Some((owner, name)) if ok(owner) && ok(name))
}

/// `X.Y.Z` of `X.Y.Z` or `vX.Y.Z`; anything else is an error.
pub fn normalize_version(input: &str) -> Result<String> {
    let version = input.strip_prefix('v').unwrap_or(input);
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
    {
        Ok(version.to_string())
    } else {
        bail!("not a version: {input:?} (expected X.Y.Z or vX.Y.Z)")
    }
}

fn version_key(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.').map(|p| p.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// The release asset target of the platform this binary runs on.
pub fn host_target() -> Result<&'static str> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        (os, arch) => bail!(
            "versioned installs are not supported on {os}/{arch}; install cargo-project-tools manually"
        ),
    }
}

/// The hash `SHA256SUMS` lists for `asset`.
pub fn expected_sha256(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == asset).then(|| hash.to_ascii_lowercase())
    })
}

/// Whether `cargo-project-tools --version` output is for `version`.
pub fn reports_version(output: &str, version: &str) -> bool {
    output
        .trim_start()
        .strip_prefix("cargo-project-tools ")
        .and_then(|rest| rest.strip_prefix(version))
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\n']))
}

/// Installed versions, newest first.
pub fn installed(env: &SelfEnv) -> Result<Vec<String>> {
    let mut versions = Vec::new();
    match fs::read_dir(env.bin()) {
        Ok(entries) => {
            for entry in entries {
                let name = entry?.file_name();
                if let Some(v) = name.to_str().and_then(|n| n.strip_prefix(PREFIX))
                    && normalize_version(v).is_ok_and(|n| n == v)
                {
                    versions.push(v.to_string());
                }
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("reading {}", env.bin().display())),
    }
    versions.sort_by_key(|v| std::cmp::Reverse(version_key(v)));
    Ok(versions)
}

/// The version `bin/cargo-project-tools` points at.
pub fn active(env: &SelfEnv) -> Option<String> {
    let target = fs::read_link(env.bin().join(LINK)).ok()?;
    let name = target.file_name()?.to_str()?;
    Some(name.strip_prefix(PREFIX)?.to_string())
}

#[cfg(unix)]
fn switch_link(env: &SelfEnv, version: &str) -> Result<()> {
    let bin = env.bin();
    let link = bin.join(LINK);
    if let Ok(meta) = fs::symlink_metadata(&link)
        && !meta.file_type().is_symlink()
    {
        bail!(
            "{} is not a symlink; move it away and retry",
            link.display()
        );
    }
    let tmp = bin.join(format!(".{LINK}.{}", std::process::id()));
    let _ = fs::remove_file(&tmp);
    std::os::unix::fs::symlink(format!("{PREFIX}{version}"), &tmp)
        .with_context(|| format!("creating {}", tmp.display()))?;
    fs::rename(&tmp, &link)
        .inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
        .with_context(|| format!("replacing {}", link.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn switch_link(_env: &SelfEnv, _version: &str) -> Result<()> {
    bail!("versioned installs need symlinks; install cargo-project-tools manually on this platform")
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

fn api_get<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    let body = net::get_bytes(url)?;
    serde_json::from_slice(&body).with_context(|| format!("parsing {url}"))
}

/// The version of the repository's latest release.
fn latest_release(env: &SelfEnv, cfg: &SelfConfig) -> Result<String> {
    let url = format!("{}/repos/{}/releases/latest", env.api_base, cfg.repo);
    let release: Release = api_get(&url)?;
    normalize_version(&release.tag_name)
}

/// Removes the download directory however the install ends.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn download(url: &str, dest: &Path) -> Result<()> {
    let bytes = net::get_bytes(url)?;
    fs::write(dest, bytes).with_context(|| format!("writing {}", dest.display()))
}

fn fetch_and_verify(env: &SelfEnv, cfg: &SelfConfig, version: &str) -> Result<()> {
    let target = host_target()?;
    let asset = format!("{PREFIX}{target}");
    let tmp = TempDir(
        env.home
            .join("tmp")
            .join(format!("v{version}-{}", std::process::id())),
    );
    let _ = fs::remove_dir_all(&tmp.0);
    fs::create_dir_all(&tmp.0).with_context(|| format!("creating {}", tmp.0.display()))?;
    let base = format!(
        "{}/{}/releases/download/v{version}",
        env.releases_base, cfg.repo
    );
    eprintln!("Downloading v{version} ({target}) from {}", cfg.repo);
    let file = tmp.0.join(&asset);
    let bundle = tmp.0.join(format!("{asset}.sigstore.json"));
    let sums = tmp.0.join("SHA256SUMS");
    download(&format!("{base}/{asset}"), &file)?;
    download(&format!("{base}/{asset}.sigstore.json"), &bundle)?;
    download(&format!("{base}/SHA256SUMS"), &sums)?;

    let expected = expected_sha256(&fs::read_to_string(&sums)?, &asset)
        .with_context(|| format!("SHA256SUMS of v{version} does not list {asset}"))?;
    let actual = file_sha256(&file)?;
    if actual != expected {
        bail!("sha256 mismatch for {asset}: SHA256SUMS lists {expected}, got {actual}");
    }
    eprintln!("sha256 matches SHA256SUMS");

    let output = Command::new("cosign")
        .arg("verify-blob")
        .arg(&file)
        .arg("--bundle")
        .arg(&bundle)
        .arg("--certificate-identity")
        .arg(&cfg.signer)
        .arg("--certificate-oidc-issuer")
        .arg(OIDC_ISSUER)
        .output()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => {
                anyhow::anyhow!("cosign is required but was not found on PATH; see {COSIGN_HINT}")
            }
            _ => anyhow::Error::new(e).context("failed to start cosign"),
        })?;
    if !output.status.success() {
        bail!(
            "cosign could not verify {asset} as signed by {}: {}",
            cfg.signer,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    eprintln!("cosign signature by {}", cfg.signer);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755))?;
    }
    let output = Command::new(&file)
        .arg("--version")
        .output()
        .with_context(|| format!("running {} --version", file.display()))?;
    let reported = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !reports_version(&reported, version) {
        bail!(
            "the v{version} download reports {:?}, not version {version}",
            reported.trim()
        );
    }

    fs::create_dir_all(env.bin()).with_context(|| format!("creating {}", env.bin().display()))?;
    let dest = env.bin().join(format!("{PREFIX}{version}"));
    fs::rename(&file, &dest).with_context(|| format!("moving download to {}", dest.display()))?;
    Ok(())
}

fn path_hint(env: &SelfEnv) -> Option<String> {
    let bin = env.bin();
    let on_path = env::var_os("PATH").is_some_and(|p| env::split_paths(&p).any(|d| d == bin));
    (!on_path).then(|| {
        format!(
            "note: {} is not on PATH; see https://osseco.github.io/cargo-project-tools/docs/getting-started",
            bin.display()
        )
    })
}

/// `self-update`: installs `version` (default latest) and makes it active.
pub fn self_update(env: &SelfEnv, version: Option<&str>, no_switch: bool) -> Result<String> {
    let cfg = SelfConfig::load(env)?;
    let version = match version {
        None | Some("latest") => latest_release(env, &cfg)?,
        Some(v) => normalize_version(v)?,
    };
    let fresh = !installed(env)?.contains(&version);
    if fresh {
        fetch_and_verify(env, &cfg, &version)?;
    }
    let mut out = if fresh {
        format!("Installed {version}")
    } else {
        format!("{version} is already installed")
    };
    if !no_switch {
        switch_link(env, &version)?;
        out.push_str(&format!("; active version is now {version}"));
    }
    if let Some(hint) = path_hint(env) {
        eprintln!("{hint}");
    }
    Ok(out)
}

/// `self-use`: makes an installed version active.
pub fn self_use(env: &SelfEnv, version: &str) -> Result<String> {
    let version = normalize_version(version)?;
    let versions = installed(env)?;
    if !versions.contains(&version) {
        let have = if versions.is_empty() {
            "none".to_string()
        } else {
            versions.join(", ")
        };
        bail!(
            "{version} is not installed (installed: {have}); run `cargo project-tools self-update {version}`"
        );
    }
    switch_link(env, &version)?;
    Ok(format!("Active version is now {version}"))
}

/// `self-list`: installed versions, the active one marked with `*`.
pub fn self_list(env: &SelfEnv) -> Result<String> {
    let cfg = SelfConfig::load(env)?;
    let versions = installed(env)?;
    let current = active(env);
    let mut out = format!("Repo:   {}\nSigner: {}\n", cfg.repo, cfg.signer);
    if versions.is_empty() {
        out.push_str(&format!(
            "No versions installed in {}; run `cargo project-tools self-update`.",
            env.bin().display()
        ));
    }
    for v in &versions {
        let mark = if current.as_ref() == Some(v) {
            '*'
        } else {
            ' '
        };
        out.push_str(&format!("{mark} {v}\n"));
    }
    Ok(out.trim_end().to_string())
}

/// `self-available`: the latest published releases of the configured repo.
pub fn self_available(env: &SelfEnv) -> Result<String> {
    let cfg = SelfConfig::load(env)?;
    let url = format!("{}/repos/{}/releases", env.api_base, cfg.repo);
    let releases: Vec<Release> = api_get(&url)?;
    let versions = installed(env)?;
    let current = active(env);
    let mut out = format!("Releases of {}:\n", cfg.repo);
    let shown: Vec<_> = releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .take(AVAILABLE)
        .collect();
    if shown.is_empty() {
        out.push_str("none published");
    }
    for r in shown {
        let version = r.tag_name.strip_prefix('v').unwrap_or(&r.tag_name);
        let date = r
            .published_at
            .as_deref()
            .map_or("", |d| d.get(..10).unwrap_or(d));
        let (mark, state) = if current.as_deref() == Some(version) {
            ('*', "active")
        } else if versions.iter().any(|v| v == version) {
            (' ', "installed")
        } else {
            (' ', "")
        };
        let line = format!("{mark} {:<10} {date:<10} {state}", r.tag_name);
        out.push_str(line.trim_end());
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

/// `self-check`: whether a newer version than the active one is installed or released.
pub fn self_check(env: &SelfEnv) -> Result<String> {
    let cfg = SelfConfig::load(env)?;
    let latest = latest_release(env, &cfg)?;
    let newest_installed = installed(env)?.into_iter().next();
    let Some(current) = active(env) else {
        return Ok(format!(
            "No version is active; run `cargo project-tools self-update` to install {latest}."
        ));
    };
    Ok(check_report(&current, newest_installed.as_deref(), &latest))
}

fn check_report(current: &str, newest_installed: Option<&str>, latest: &str) -> String {
    let newer = |v: &str| version_key(v) > version_key(current);
    // A release that is already installed is reported as installed only.
    let release_is_new =
        newer(latest) && newest_installed.is_none_or(|v| version_key(latest) > version_key(v));
    let mut lines = Vec::new();
    if let Some(v) = newest_installed.filter(|v| newer(v)) {
        lines.push(format!(
            "Newer installed: {v} (active {current}); run `cargo project-tools self-use {v}`"
        ));
    }
    if release_is_new {
        lines.push(format!(
            "Newer release: {latest} (active {current}); run `cargo project-tools self-update`"
        ));
    }
    if lines.is_empty() {
        lines.push(format!("{current} is active and the latest release."));
    }
    lines.join("\n")
}

#[cfg(test)]
#[path = "selfinstall_test.rs"]
mod tests;
