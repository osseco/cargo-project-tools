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

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create project-tools.toml and the .project-tools directory.
    Init,
    /// Add a tool, starting from the Cargo.lock it ships with, or a generated one.
    Add(AddArgs),
    /// Remove a tool, its lock, build dir and binaries.
    Remove { name: String },
    /// Move a tool to a new version, carrying forward newer dependency versions where possible.
    Upgrade(UpgradeArgs),
    /// Update dependencies in a tool's Cargo.lock.
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
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Crate name, optionally with a version requirement: `name` or `name@1.2.3`.
    pub spec: String,
    /// Install from a git repository (https only).
    #[arg(long)]
    pub git: Option<String>,
    #[arg(long, requires = "git", conflicts_with_all = ["tag", "rev"])]
    pub branch: Option<String>,
    #[arg(long, requires = "git", conflicts_with = "rev")]
    pub tag: Option<String>,
    #[arg(long, requires = "git")]
    pub rev: Option<String>,
    /// Features to enable.
    #[arg(long, short = 'F', value_delimiter = ',')]
    pub features: Vec<String>,
    #[arg(long)]
    pub no_default_features: bool,
    /// Binaries to install (default: all).
    #[arg(long = "bin")]
    pub bins: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UpgradeArgs {
    pub name: String,
    /// Version requirement (default: latest stable).
    #[arg(long)]
    pub version: Option<String>,
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
}

#[derive(Debug, Args)]
pub struct InstallArgs {
    /// Tools to install (default: all).
    pub names: Vec<String>,
    /// Rebuild even if the install stamp is up to date.
    #[arg(long)]
    pub force: bool,
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
    /// Commands that download packages and should run through sfw. `audit` and `audit-db`
    /// only fetch the advisory database, which isn't a package. `verify` only builds when
    /// repairing.
    pub fn downloads_packages(&self) -> bool {
        match self {
            Command::Verify(args) => args.repair,
            _ => matches!(
                self,
                Command::Add(_) | Command::Upgrade(_) | Command::Update(_) | Command::Install(_)
            ),
        }
    }
}
