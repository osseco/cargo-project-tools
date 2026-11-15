mod advisory_db;
mod audit;
mod cli;
mod config;
mod install;
mod lock;
mod net;
mod process;
mod project;
mod sfw;
mod source;

use std::fs;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;

use cli::{AddArgs, AuditArgs, AuditDbCommand, Cli, Command, InstallArgs, UpdateArgs, UpgradeArgs};
use config::{AuditIgnore, Config, ConfigEditor, SfwMode};
use project::Project;
use source::{Dest, Origin, git::GitRef};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    // Invoked as `cargo project-tools …`, cargo passes the subcommand name as the first arg.
    let mut argv: Vec<_> = std::env::args_os().collect();
    if argv.get(1).is_some_and(|a| a == cli::SUBCOMMAND) {
        argv.remove(1);
    }
    let cli = Cli::parse_from(argv);

    if let Command::Init = cli.command {
        return init().map(|()| ExitCode::SUCCESS);
    }
    let project = Project::discover()?;
    let config = Config::load(&project.config_path())?;

    if cli.command.downloads_packages() {
        let mode = if cli.no_sfw {
            SfwMode::Off
        } else {
            config.settings.sfw
        };
        if let Some(code) = sfw::reexec(mode)? {
            return Ok(ExitCode::from(u8::try_from(code).unwrap_or(1)));
        }
    }

    match cli.command {
        Command::Init => unreachable!(),
        Command::Add(args) => add(&project, args)?,
        Command::Remove { name } => remove(&project, &config, &name)?,
        Command::Upgrade(args) => upgrade(&project, &config, args)?,
        Command::Update(args) => update(&project, &config, args)?,
        Command::Install(args) => install(&project, &config, &args)?,
        Command::Audit(args) => return audit(&project, &config, args),
        Command::AuditDb(AuditDbCommand::Update) => {
            let max_age = config.settings.advisory_db_max_age()?;
            advisory_db::ensure(&project.advisory_db(), advisory_db::Policy::Force, max_age)?;
            println!(
                "Advisory database is up to date in {}",
                project.advisory_db().display()
            );
        }
        Command::List => list(&project, &config)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn init() -> Result<()> {
    let project = Project::for_init()?;
    let path = project.config_path();
    if path.exists() {
        println!("{} already exists", path.display());
    } else {
        fs::write(&path, config::TEMPLATE)?;
        println!("Created {}", path.display());
    }
    project.ensure_layout()?;
    println!("Prepared {}", project.tools_dir().display());
    Ok(())
}

fn split_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.split_once('@') {
        Some((n, v)) => (n, Some(v)),
        None => (spec, None),
    }
}

fn write_new_lock(project: &Project, name: &str, lock_text: &str) -> Result<()> {
    fs::create_dir_all(project.crate_dir(name))?;
    fs::write(project.lock_path(name), lock_text)?;
    let ignore = project.audit_ignore_path(name);
    if !ignore.exists() {
        fs::write(ignore, config::AUDIT_IGNORE_TEMPLATE)?;
    }
    Ok(())
}

fn add(project: &Project, args: AddArgs) -> Result<()> {
    let (name, req) = split_spec(&args.spec);
    let mut editor = ConfigEditor::open(&project.config_path())?;
    if editor.has_tool(name) {
        bail!("`{name}` is already in project-tools.toml; use `upgrade` to change its version");
    }
    let origin = match &args.git {
        Some(url) => {
            if req.is_some() {
                bail!("use --tag/--rev/--branch instead of @version with --git");
            }
            Origin::Git {
                url: url.clone(),
                reference: GitRef {
                    branch: args.branch.clone(),
                    tag: args.tag.clone(),
                    rev: args.rev.clone(),
                },
            }
        }
        None => Origin::Registry {
            version_req: req.map(str::to_string),
        },
    };
    let checkout = source::checkout(project, name, &origin, Dest::Temp)?;
    let (lock_text, start) =
        lock::shipped_or_generated(&checkout.manifest_path(), &checkout.lock_path(), &[])?;
    let kind = match start {
        lock::Start::Shipped => "its published Cargo.lock",
        lock::Start::Generated => "a generated Cargo.lock (it ships none)",
    };
    lock::check_root(&lock::parse(&lock_text)?, name, &checkout.version)?;
    write_new_lock(project, name, &lock_text)?;

    let mut entries: Vec<(&str, toml_edit::Value)> =
        vec![("version", checkout.version.as_str().into())];
    if let Some(url) = &args.git {
        entries.push(("git", url.as_str().into()));
        if let Some(b) = &args.branch {
            entries.push(("branch", b.as_str().into()));
        }
        if let Some(t) = &args.tag {
            entries.push(("tag", t.as_str().into()));
        }
        if let Some(commit) = &checkout.commit {
            entries.push(("rev", commit.as_str().into()));
        }
    }
    if !args.features.is_empty() {
        entries.push(("features", config::string_array(&args.features)));
    }
    if args.no_default_features {
        entries.push(("default-features", false.into()));
    }
    if !args.bins.is_empty() {
        entries.push(("bins", config::string_array(&args.bins)));
    }
    editor.set_tool(name, &entries);
    editor.save()?;
    println!(
        "Added {name} {} with {kind} in {}",
        checkout.version,
        project.crate_dir(name).display()
    );
    Ok(())
}

fn remove(project: &Project, config: &Config, name: &str) -> Result<()> {
    config.tool(name)?;
    let status = process::cargo()
        .args(["uninstall", "--quiet", "--root"])
        .arg(project.install_root())
        .arg(name)
        .status();
    if !status.is_ok_and(|s| s.success()) {
        eprintln!("note: no installed binaries for {name} were removed");
    }
    for dir in [project.crate_dir(name), project.target_dir(name)] {
        if dir.exists() {
            fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
        }
    }
    let mut editor = ConfigEditor::open(&project.config_path())?;
    editor.remove_tool(name);
    editor.save()?;
    println!("Removed {name}");
    Ok(())
}

fn upgrade(project: &Project, config: &Config, args: UpgradeArgs) -> Result<()> {
    let tool = config.tool(&args.name)?;
    let name = args.name.as_str();
    let origin = match Origin::of(tool) {
        Origin::Registry { .. } => Origin::Registry {
            version_req: args.version.clone(),
        },
        // Follow the branch/tag again instead of the pinned commit.
        Origin::Git { url, mut reference } => {
            reference.rev = None;
            Origin::Git { url, reference }
        }
    };
    let checkout = source::checkout(project, name, &origin, Dest::Temp)?;
    let ours = lock::load(&project.lock_path(name))?;
    let manifest = checkout.manifest_path();
    let lock_path = checkout.lock_path();

    let shipped = match checkout.shipped_lock()? {
        Some(text) => {
            let shipped = lock::parse(&text)?;
            lock::check_root(&shipped, name, &checkout.version)?;
            shipped
        }
        None => {
            lock::reseed(&manifest, &lock_path, &project.lock_path(name), &[])?;
            lock::load(&lock_path)?
        }
    };

    // Keep dependency bumps we made that the new lock doesn't already include. A lock seeded
    // from ours rarely lacks any, but a forced downgrade is retried here like any other.
    for (dep, from, to) in lock::newer_than(&ours, &shipped) {
        let backup = fs::read_to_string(&lock_path)?;
        let spec = format!("{dep}@{from}");
        let ok = lock::cargo_update(
            &manifest,
            &[
                "--quiet".into(),
                "-p".into(),
                spec,
                "--precise".into(),
                to.to_string(),
            ],
        );
        match ok {
            Ok(()) => println!("  kept {dep} {to} (new lock has {from})"),
            Err(_) => {
                fs::write(&lock_path, backup)?;
                println!(
                    "  dropped {dep} {to}: does not resolve with {name} {}",
                    checkout.version
                );
            }
        }
    }

    let final_lock = lock::load(&lock_path)?;
    lock::check_root(&final_lock, name, &checkout.version)?;
    println!("Changes to {name}'s lock:");
    lock::print_diff(&lock::diff(&ours, &final_lock));
    fs::copy(&lock_path, project.lock_path(name))?;

    let mut editor = ConfigEditor::open(&project.config_path())?;
    let mut entries: Vec<(&str, toml_edit::Value)> =
        vec![("version", checkout.version.as_str().into())];
    if let Some(commit) = &checkout.commit {
        entries.push(("rev", commit.as_str().into()));
    }
    editor.set_tool(name, &entries);
    editor.save()?;
    println!("Upgraded {name} {} -> {}", tool.version, checkout.version);
    Ok(())
}

fn update(project: &Project, config: &Config, args: UpdateArgs) -> Result<()> {
    let tool = config.tool(&args.name)?;
    let name = args.name.as_str();
    if args.precise.is_some() && args.deps.len() != 1 {
        bail!("--precise needs exactly one dependency");
    }
    let ours_path = project.lock_path(name);
    let ours = lock::load(&ours_path)?;
    lock::check_root(&ours, name, &tool.version)?;

    let scratch = source::scratch(project, name, &Origin::of(tool))?;
    scratch.put_lock(&ours_path)?;
    let mut cargo_args = Vec::new();
    for dep in &args.deps {
        cargo_args.extend(["-p".to_string(), dep.clone()]);
    }
    if let Some(p) = &args.precise {
        cargo_args.extend(["--precise".to_string(), p.clone()]);
    }
    if args.recursive {
        cargo_args.push("--recursive".into());
    }
    // Only a successful resolve reaches the copy below, so a failure leaves our lock as is.
    lock::cargo_update(&scratch.manifest_path(), &cargo_args)?;

    let updated = lock::load(&scratch.lock_path())?;
    lock::check_root(&updated, name, &tool.version)?;
    let changes = lock::diff(&ours, &updated);
    println!("Changes to {name}'s lock:");
    lock::print_diff(&changes);
    if !changes.is_empty() {
        fs::copy(scratch.lock_path(), &ours_path)?;
    }
    Ok(())
}

/// Load the advisory database and report advisories against this binary itself. Returns
/// the auditor and how many such advisories there are.
fn load_auditor(
    project: &Project,
    config: &Config,
    policy: advisory_db::Policy,
) -> Result<(audit::Auditor, usize)> {
    let max_age = config.settings.advisory_db_max_age()?;
    let db = advisory_db::ensure(&project.advisory_db(), policy, max_age)?;
    let auditor = audit::Auditor::new(db);
    let own = auditor.self_check();
    for msg in &own {
        println!("error: {msg}");
    }
    Ok((auditor, own.len()))
}

fn audit_tool(
    project: &Project,
    auditor: &mut audit::Auditor,
    name: &str,
    tool: &config::Tool,
    today: &str,
) -> Result<audit::Outcome> {
    println!("{name}:");
    let report = auditor.audit(&project.lock_path(name), name, tool.git.is_some())?;
    let ignore = AuditIgnore::load(&project.audit_ignore_path(name))?;
    let outcome = audit::evaluate(name, &report, &ignore, today, &mut auditor.index);
    if outcome.vulnerabilities == 0 && outcome.warnings == 0 {
        println!("  ok");
    }
    Ok(outcome)
}

fn audit(project: &Project, config: &Config, args: AuditArgs) -> Result<ExitCode> {
    let tools = config.select(&args.names)?;
    let policy = if args.update_db {
        advisory_db::Policy::Force
    } else {
        advisory_db::Policy::Auto
    };
    let (mut auditor, own) = load_auditor(project, config, policy)?;
    let today = audit::today();
    let (mut vulns, mut warns) = (own, 0);
    for (name, tool) in tools {
        let outcome = audit_tool(project, &mut auditor, name, tool, &today)?;
        vulns += outcome.vulnerabilities;
        warns += outcome.warnings;
    }
    println!("{vulns} vulnerabilities, {warns} warnings");
    let failed = vulns > 0 || (args.deny_warnings && warns > 0);
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Audit every selected tool's lock first, so nothing is built when one of them fails.
fn install(project: &Project, config: &Config, args: &InstallArgs) -> Result<()> {
    let tools = config.select(&args.names)?;
    if !args.no_audit {
        let policy = if args.offline {
            advisory_db::Policy::Offline
        } else {
            advisory_db::Policy::Auto
        };
        let (mut auditor, own) = load_auditor(project, config, policy)?;
        if own > 0 {
            bail!(
                "{} itself has open advisories; upgrade it, or pass --no-audit",
                cli::SUBCOMMAND
            );
        }
        audit_before_install(
            project,
            &mut auditor,
            &tools,
            args.deny_warnings,
            &audit::today(),
        )?;
    }
    for (name, tool) in tools {
        let build = config.build.merged(tool.build.as_ref());
        install::install(project, name, tool, &build, args)?;
    }
    Ok(())
}

/// Fails, naming every tool that didn't pass, when a lock has a vulnerability that isn't
/// ignored, or with `deny_warnings` any warning.
fn audit_before_install(
    project: &Project,
    auditor: &mut audit::Auditor,
    tools: &[(&str, &config::Tool)],
    deny_warnings: bool,
    today: &str,
) -> Result<()> {
    let mut failed = Vec::new();
    for (name, tool) in tools {
        let outcome = audit_tool(project, auditor, name, tool, today)?;
        if outcome.vulnerabilities > 0 || (deny_warnings && outcome.warnings > 0) {
            failed.push(*name);
        }
    }
    if !failed.is_empty() {
        bail!(
            "audit failed for {}; nothing was installed. Fix or ignore the findings, or pass --no-audit",
            failed.join(", ")
        );
    }
    Ok(())
}

fn list(project: &Project, config: &Config) -> Result<()> {
    for (name, tool) in &config.tool {
        let source = tool.git.as_deref().unwrap_or("crates.io");
        let has_lock = project.lock_path(name).is_file();
        let built = project
            .target_dir(name)
            .join(".install-stamp.json")
            .is_file();
        println!(
            "{name} {} ({source}){}{}",
            tool.version,
            if has_lock {
                ""
            } else {
                " [missing Cargo.lock]"
            },
            if built { " [installed]" } else { "" }
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "main_test.rs"]
mod tests;
