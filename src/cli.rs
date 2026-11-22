use clap::{Args, Parser, Subcommand};

/// Name used when invoked as `cargo <SUBCOMMAND>`.
pub const SUBCOMMAND: &str = "project-tools";

#[derive(Debug, Parser)]
#[command(
    name = "cargo-project-tools",
    bin_name = "cargo project-tools",
    version,
    about = "Manage a project's cargo-installed tools with project-owned Cargo.lock files"
)]
pub struct Cli {
    /// Don't run network commands through sfw (Socket Firewall Free).
    #[arg(long, global = true)]
    pub no_sfw: bool,

    /// `json`: one JSON record per line on stdout (findings, pending reviews, missing
    /// targets, a summary), and the human text on stderr.
    #[arg(long, global = true, value_enum, default_value_t)]
    pub message_format: crate::output::Format,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create project-tools.toml and the .project-tools directory.
    Init(InitArgs),
    /// Add a tool, starting from the Cargo.lock it ships with, or a generated one.
    Add(AddArgs),
    /// Remove a tool, its lock, build dir and binaries.
    Remove { name: String },
    /// Deprecated: use `update <tool> --latest` or `--version <req>`.
    #[command(hide = true)]
    Upgrade(UpgradeArgs),
    /// Update dependencies in a tool's Cargo.lock, or move the tool to another version.
    Update(UpdateArgs),
    /// Build and install tools into .project-tools/bin using the project's locks.
    Install(InstallArgs),
    /// Check installed binaries against the checksums `install` recorded.
    Verify(VerifyArgs),
    /// Audit every tool's Cargo.lock against the RustSec advisory database.
    Audit(AuditArgs),
    /// Manage the advisory database used by `audit`.
    #[command(subcommand)]
    AuditDb(AuditDbCommand),
    /// List configured tools.
    List,
    /// Show what a tool builds with its features, what each other feature would add, and
    /// the advisories and build-time code that come with it. Records nothing.
    Inspect(InspectArgs),
    /// Install a release of cargo-project-tools under `~/.cargo-project-tools` and make it active.
    ///
    /// Without a version, installs the latest release. Each version is kept as
    /// `~/.cargo-project-tools/bin/cargo-project-tools-X.Y.Z`, and
    /// `~/.cargo-project-tools/bin/cargo-project-tools` is a symlink to the active one, so put
    /// `~/.cargo-project-tools/bin` first on `PATH`. A version that is already installed is
    /// only switched to.
    ///
    /// Releases come from the `repo` in `~/.cargo-project-tools/config.toml` (default
    /// `osseco/cargo-project-tools`). Each download must match the release's `SHA256SUMS`,
    /// carry a cosign signature by the `signer` in the same file (default the `release.yml`
    /// workflow on `main` of `repo`), and report the version it was downloaded as. Needs
    /// `cosign`.
    ///
    /// `CARGO_PROJECT_TOOLS_HOME` replaces `~/.cargo-project-tools`. macOS and Linux only.
    #[command(name = "self-update")]
    SelfUpdate {
        /// Version to install: `X.Y.Z`, `vX.Y.Z` or `latest`.
        version: Option<String>,
        /// Install without making it the active version.
        #[arg(long)]
        no_switch: bool,
    },
    /// Make an installed version of cargo-project-tools the active one.
    ///
    /// Nothing is downloaded; install a version first with `cargo project-tools self-update`.
    #[command(name = "self-use")]
    SelfUse {
        /// Installed version: `X.Y.Z` or `vX.Y.Z`.
        version: String,
    },
    /// List the versions of cargo-project-tools installed under `~/.cargo-project-tools`.
    ///
    /// The active one is marked `*`. Also prints the repository and signer `self-update` uses.
    #[command(name = "self-list")]
    SelfList,
    /// List the ten latest releases of the configured repository.
    ///
    /// Newest first, with the publish date, and which are installed or active. Drafts and
    /// prereleases are left out. Asks the GitHub API without a token; nothing is downloaded.
    #[command(name = "self-available")]
    SelfAvailable,
    /// Tell whether a newer cargo-project-tools than the active one exists.
    ///
    /// Compares the active version with the newest one installed and with the latest release
    /// of the configured repository, and names the command that switches or installs it.
    /// Asks the GitHub API without a token; nothing is downloaded.
    #[command(name = "self-check")]
    SelfCheck,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Target triple to review, audit and install tools for (repeatable). Without it,
    /// init offers this machine and the runners of the project's GitHub workflows.
    #[arg(long = "target")]
    pub targets: Vec<String>,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Crate name, optionally with a version requirement: `name` or `name@1.2.3`.
    pub spec: String,
    #[command(flatten)]
    pub source: SourceArgs,
    /// Features to enable.
    #[arg(long, short = 'F', value_delimiter = ',')]
    pub features: Vec<String>,
    #[arg(long)]
    pub no_default_features: bool,
    /// Binaries to install (default: all).
    #[arg(long = "bin")]
    pub bins: Vec<String>,
    /// Choose features and dependency versions on a setup screen, with the dependency tree
    /// and its advisories, before the build review.
    #[arg(long, short = 'i')]
    pub interactive: bool,
    /// Record all new or changed build scripts, proc macros and cargo config in
    /// build-review.toml at once. Review them first. In a terminal, without this flag,
    /// a review screen lets you decide on each.
    #[arg(long)]
    pub auto_approve: bool,
}

/// A git source instead of crates.io.
#[derive(Debug, Args)]
pub struct SourceArgs {
    /// Install from a git repository (https only).
    #[arg(long)]
    pub git: Option<String>,
    #[arg(long, requires = "git", conflicts_with_all = ["tag", "rev"])]
    pub branch: Option<String>,
    #[arg(long, requires = "git", conflicts_with = "rev")]
    pub tag: Option<String>,
    #[arg(long, requires = "git")]
    pub rev: Option<String>,
}

#[derive(Debug, Args)]
pub struct InspectArgs {
    /// A configured tool, or a crate to inspect before adding it: `name` or `name@1.2.3`.
    pub spec: String,
    #[command(flatten)]
    pub source: SourceArgs,
    /// Features to enable, instead of the tool's.
    #[arg(long, short = 'F', value_delimiter = ',')]
    pub features: Vec<String>,
    /// Without the default features, instead of the tool's setting.
    #[arg(long)]
    pub no_default_features: bool,
}

#[derive(Debug, Args)]
pub struct UpgradeArgs {
    pub name: String,
    /// Version requirement (default: latest stable).
    #[arg(long)]
    pub version: Option<String>,
    /// Record all new or changed build scripts, proc macros and cargo config in
    /// build-review.toml at once. Review them first. In a terminal, without this flag,
    /// a review screen lets you decide on each.
    #[arg(long)]
    pub auto_approve: bool,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Tool whose lock to update.
    pub name: String,
    /// Dependencies to update (`name` or `name@version`); all when omitted.
    pub deps: Vec<String>,
    /// Update a single dependency to exactly this version.
    #[arg(long)]
    pub precise: Option<String>,
    /// Also update the dependencies of the given dependencies.
    #[arg(long)]
    pub recursive: bool,
    /// Move the tool to this version requirement (registry tools), keeping your dependency
    /// bumps where they still resolve.
    #[arg(long, conflicts_with = "latest")]
    pub version: Option<String>,
    /// Move the tool to its latest stable version, or for a git tool, to the head of its
    /// branch or tag again.
    #[arg(long)]
    pub latest: bool,
    /// Features to enable, replacing the tool's list (`-F ""` clears it). Without
    /// dependencies named, only the features change.
    #[arg(long, short = 'F', value_delimiter = ',')]
    pub features: Option<Vec<String>>,
    /// Build without the default features.
    #[arg(long, conflicts_with = "default_features")]
    pub no_default_features: bool,
    /// Build with the default features again.
    #[arg(long)]
    pub default_features: bool,
    /// Change the tool's version, features and dependency versions on a setup screen, with
    /// the dependency tree and its advisories, before the build review.
    #[arg(
        long,
        short = 'i',
        conflicts_with_all = [
            "deps", "precise", "recursive", "version", "latest", "features",
            "no_default_features", "default_features"
        ]
    )]
    pub interactive: bool,
    /// Record all new or changed build scripts, proc macros and cargo config in
    /// build-review.toml at once. Review them first. In a terminal, without this flag,
    /// a review screen lets you decide on each.
    #[arg(long)]
    pub auto_approve: bool,
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Tools to install (default: all).
    pub names: Vec<String>,
    /// Rebuild even if the install stamp is up to date.
    #[arg(long)]
    pub force: bool,
    /// Check targets, the audit and the build review, then stop: build and record nothing.
    /// Fails when anything is missing or not reviewed.
    #[arg(long, conflicts_with_all = ["force", "auto_approve", "approve_target"])]
    pub check: bool,
    /// Record all new or changed build scripts, proc macros and cargo config in
    /// build-review.toml at once. Review them first. In a terminal, without this flag,
    /// a review screen lets you decide on each.
    #[arg(long)]
    pub auto_approve: bool,
    /// Add this machine (and --target) to the tool's targets when it isn't one of them.
    #[arg(long)]
    pub approve_target: bool,
    /// Don't audit the locks before building.
    #[arg(long)]
    pub no_audit: bool,
    /// Also refuse to build on warnings (unmaintained, unsound, yanked, git-source) that
    /// aren't ignored.
    #[arg(long, conflicts_with = "no_audit")]
    pub deny_warnings: bool,
    #[command(flatten)]
    pub build: BuildArgs,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    /// Tools to verify (default: all).
    pub names: Vec<String>,
    /// Rebuild tools that don't match; fail if the rebuild doesn't match the recorded checksum.
    #[arg(long)]
    pub repair: bool,
    /// Don't audit the locks before repairing.
    #[arg(long, requires = "repair")]
    pub no_audit: bool,
    /// Also refuse to repair on warnings (unmaintained, unsound, yanked, git-source) that
    /// aren't ignored.
    #[arg(long, requires = "repair", conflicts_with = "no_audit")]
    pub deny_warnings: bool,
    #[command(flatten)]
    pub build: BuildArgs,
}

/// Flags passed on to `cargo install`.
#[derive(Debug, Args)]
pub struct BuildArgs {
    #[arg(long, short = 'j')]
    pub jobs: Option<String>,
    #[arg(long)]
    pub offline: bool,
    #[arg(long)]
    pub frozen: bool,
    /// Override [build].profile.
    #[arg(long)]
    pub profile: Option<String>,
    #[arg(long, conflicts_with = "profile")]
    pub debug: bool,
    /// Override [build].target.
    #[arg(long)]
    pub target: Option<String>,
    /// Extra cargo `--config` values.
    #[arg(long)]
    pub config: Vec<String>,
    #[arg(long, short = 'q')]
    pub quiet: bool,
    #[arg(long, short = 'v', action = clap::ArgAction::Count)]
    pub verbose: u8,
    #[arg(long)]
    pub color: Option<String>,
    #[arg(long)]
    pub keep_going: bool,
    #[arg(long)]
    pub timings: bool,
}

#[derive(Debug, Args)]
pub struct AuditArgs {
    /// Tools to audit (default: all).
    pub names: Vec<String>,
    /// Fail on warnings (unmaintained, unsound, yanked, git-source) that aren't ignored.
    #[arg(long)]
    pub deny_warnings: bool,
    /// Revalidate the advisory database before auditing, whatever its age.
    #[arg(long)]
    pub update_db: bool,
}

#[derive(Debug, Subcommand)]
pub enum AuditDbCommand {
    /// Fetch or revalidate the advisory database.
    Update,
}

impl Command {
    /// Commands whose JSON output ends with a `summary` record.
    pub fn has_summary(&self) -> bool {
        matches!(
            self,
            Command::Add(_)
                | Command::Upgrade(_)
                | Command::Update(_)
                | Command::Install(_)
                | Command::Audit(_)
        )
    }

    /// Commands that download packages and should run through sfw. `audit` and `audit-db`
    /// only fetch the advisory database, which isn't a package. `verify` only builds when
    /// repairing.
    pub fn downloads_packages(&self) -> bool {
        match self {
            Command::Verify(args) => args.repair,
            _ => matches!(
                self,
                Command::Add(_)
                    | Command::Upgrade(_)
                    | Command::Update(_)
                    | Command::Install(_)
                    | Command::Audit(_)
                    | Command::Inspect(_)
            ),
        }
    }
}
