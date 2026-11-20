mod advisory_db;
mod audit;
mod checksum;
mod cli;
mod config;
mod graph;
mod install;
mod lock;
mod net;
mod process;
mod project;
mod review;
mod selfinstall;
mod sfw;
mod source;
mod targets;
#[cfg(test)]
mod test_registry;
mod vendor;

use std::fs;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;

use cli::{
    AddArgs, AuditArgs, AuditDbCommand, Cli, Command, InitArgs, InstallArgs, UpdateArgs,
    UpgradeArgs, VerifyArgs,
};
use config::{AuditIgnore, Config, ConfigEditor, SfwMode};
use project::Project;
use selfinstall::SelfEnv;
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

    if let Command::Init(args) = &cli.command {
        return init(args).map(|()| ExitCode::SUCCESS);
    }
    // Self-management works outside a project.
    let self_out = match &cli.command {
        Command::SelfUpdate { version, no_switch } => Some(selfinstall::self_update(
            &SelfEnv::from_env()?,
            version.as_deref(),
            *no_switch,
        )?),
        Command::SelfUse { version } => {
            Some(selfinstall::self_use(&SelfEnv::from_env()?, version)?)
        }
        Command::SelfList => Some(selfinstall::self_list(&SelfEnv::from_env()?)?),
        Command::SelfAvailable => Some(selfinstall::self_available(&SelfEnv::from_env()?)?),
        Command::SelfCheck => Some(selfinstall::self_check(&SelfEnv::from_env()?)?),
        _ => None,
    };
    if let Some(out) = self_out {
        println!("{out}");
        return Ok(ExitCode::SUCCESS);
    }
    let project = Project::discover()?;
    // Cargo and rustup look for config and toolchain files from the working directory up, so
    // they must never start inside a tool's source.
    std::env::set_current_dir(&project.root)
        .with_context(|| format!("entering {}", project.root.display()))?;
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
        Command::Init(_) => unreachable!(),
        Command::Add(args) => add(&project, &config, args)?,
        Command::Remove { name } => remove(&project, &config, &name)?,
        Command::Upgrade(args) => upgrade(&project, &config, args)?,
        Command::Update(args) => update(&project, &config, args)?,
        Command::Install(args) => install(&project, &config, &args)?,
        Command::Verify(args) => return verify(&project, &config, &args),
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
        Command::SelfUpdate { .. }
        | Command::SelfUse { .. }
        | Command::SelfList
        | Command::SelfAvailable
        | Command::SelfCheck => unreachable!(),
    }
    Ok(ExitCode::SUCCESS)
}

fn init(args: &InitArgs) -> Result<()> {
    let project = Project::for_init()?;
    let path = project.config_path();
    if path.exists() {
        println!("{} already exists", path.display());
    } else {
        let targets = if args.targets.is_empty() {
            choose_targets(&project)?
        } else {
            Some(args.targets.clone())
        };
        fs::write(&path, config::template_with(targets.as_deref()))?;
        println!("Created {}", path.display());
        match targets {
            Some(t) => println!("Targets: {}", t.join(", ")),
            None => println!("Targets: all"),
        }
    }
    project.ensure_layout()?;
    println!("Prepared {}", project.tools_dir().display());
    Ok(())
}

/// Offer this machine and the workflow runners as targets. Without a terminal to ask on,
/// all of them are taken.
fn choose_targets(project: &Project) -> Result<Option<Vec<String>>> {
    use std::io::{BufRead, IsTerminal, Write};
    let suggestions = targets::suggestions(&project.root);
    if suggestions.is_empty() {
        return Ok(None);
    }
    let all: Vec<String> = suggestions.iter().map(|s| s.triple.clone()).collect();
    if !std::io::stdin().is_terminal() {
        println!(
            "note: no terminal to ask on; targets are {} (change `targets` in {} or pass --target)",
            all.join(", "),
            project::CONFIG_FILE
        );
        return Ok(Some(all));
    }
    println!("Targets to review, audit and install tools for:");
    for (i, s) in suggestions.iter().enumerate() {
        println!("  {}) {}  ({})", i + 1, s.triple, s.sources.join(", "));
    }
    loop {
        print!(
            "Numbers or triples, `all` for every target [Enter: 1-{}]: ",
            suggestions.len()
        );
        std::io::stdout().flush()?;
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line)? == 0 {
            return Ok(Some(all));
        }
        match targets::parse_choice(&line, &suggestions) {
            Ok(choice) => return Ok(choice),
            Err(e) => println!("  {e}"),
        }
    }
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

fn add(project: &Project, config: &Config, args: AddArgs) -> Result<()> {
    let (name, req) = split_spec(&args.spec);
    config::validate_tool_name(name)?;
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
    let checkout = source::checkout(project, name, &origin, review_dest(project, name))?;
    let (lock_text, start) =
        lock::shipped_or_generated(&checkout.manifest_path(), &checkout.lock_path(), &[])?;
    let kind = match start {
        lock::Start::Shipped => "its published Cargo.lock",
        lock::Start::Generated => "a generated Cargo.lock (it ships none)",
    };
    lock::check_root(&lock::parse(&lock_text)?, name, &checkout.version)?;
    let features = install::feature_flags(&args.features, !args.no_default_features);
    review_build(
        project,
        name,
        &features,
        config.settings.targets.as_deref(),
        &checkout,
        args.approve_build,
    )?;
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
    for file in [checksum::path(project, name), project.stamp_path(name)] {
        if file.exists() {
            fs::remove_file(&file).with_context(|| format!("removing {}", file.display()))?;
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
    let checkout = source::checkout(project, name, &origin, review_dest(project, name))?;
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
    review_build(
        project,
        name,
        &install::feature_args(tool),
        config.targets_for(tool),
        &checkout,
        args.approve_build,
    )?;
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

    // A full checkout rather than a scratch project: the review needs the real build scripts.
    let checkout = source::checkout(project, name, &Origin::of(tool), review_dest(project, name))?;
    checkout.put_lock(&ours_path)?;
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
    lock::cargo_update(&checkout.manifest_path(), &cargo_args)?;

    let updated = lock::load(&checkout.lock_path())?;
    lock::check_root(&updated, name, &tool.version)?;
    let changes = lock::diff(&ours, &updated);
    println!("Changes to {name}'s lock:");
    lock::print_diff(&changes);
    if !changes.is_empty() {
        review_build(
            project,
            name,
            &install::feature_args(tool),
            config.targets_for(tool),
            &checkout,
            args.approve_build,
        )?;
        fs::copy(checkout.lock_path(), &ours_path)?;
    }
    Ok(())
}

/// Where `add`, `upgrade` and `update` check out the tool: the place `install` builds from,
/// so the files a review lists are still there afterwards.
fn review_dest(project: &Project, name: &str) -> Dest {
    Dest::Fixed(project.target_dir(name).join("src"))
}

/// Review the build-time code that the lock in `checkout` brings in, before it's recorded.
fn review_build(
    project: &Project,
    name: &str,
    features: &[String],
    targets: Option<&[String]>,
    checkout: &source::Checkout,
    approve: bool,
) -> Result<()> {
    let vendored = vendor::vendor(project, name, checkout, false, &[])?;
    let selection = graph::Selection { features, targets };
    review::review(
        project, name, selection, checkout, &vendored, false, approve,
    )?;
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
    let found = auditor.self_check();
    let ignore = AuditIgnore::load_self(&project.self_audit_ignore_path())?;
    if found.is_empty() && ignore.ignore.is_empty() {
        return Ok((auditor, 0));
    }
    println!("cargo-{}:", cli::SUBCOMMAND);
    let open = audit::evaluate_self(&found, &ignore, &audit::today());
    Ok((auditor, open))
}

/// The packages a tool builds with its features: `graph::built_packages` outside tests.
type BuiltOf<'a> = dyn FnMut(&str, &config::Tool) -> Result<graph::Built> + 'a;

fn audit_tool(
    project: &Project,
    auditor: &mut audit::Auditor,
    name: &str,
    tool: &config::Tool,
    today: &str,
    built_of: &mut BuiltOf,
) -> Result<audit::Outcome> {
    let built = built_of(name, tool)?;
    println!("{name}:");
    let report = auditor.audit(&project.lock_path(name), name, tool.git.is_some(), &built)?;
    if report.skipped > 0 {
        println!(
            "  skipped {} packages not built with these features",
            report.skipped
        );
    }
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
    let mut built_of = |name: &str, tool: &config::Tool| {
        graph::built_packages(project, name, tool, config.targets_for(tool), false)
    };
    for (name, tool) in tools {
        let outcome = audit_tool(project, &mut auditor, name, tool, &today, &mut built_of)?;
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
    let reloaded;
    let config = if check_targets(project, config, &tools, &args.build, args.approve_target)? {
        reloaded = Config::load(&project.config_path())?;
        &reloaded
    } else {
        config
    };
    let tools = config.select(&args.names)?;
    if !args.no_audit {
        audit_gate(
            project,
            config,
            &tools,
            args.build.offline,
            args.deny_warnings,
        )?;
    }
    for (name, tool) in tools {
        let build = config.build.merged(tool.build.as_ref());
        install::install(
            project,
            name,
            tool,
            &build,
            &args.build,
            install::Options {
                force: args.force,
                approve_build: args.approve_build,
                checksums: install::Checksums::Write,
                targets: config.targets_for(tool),
            },
        )?;
    }
    Ok(())
}

/// Make sure this machine, and `--target` or `[build].target`, are among each tool's
/// targets. Without `approve` a missing one is an error; with it, it's added to
/// project-tools.toml, to the tool's own list when it has one. Returns whether it changed.
fn check_targets(
    project: &Project,
    config: &Config,
    tools: &[(&str, &config::Tool)],
    build_args: &cli::BuildArgs,
    approve: bool,
) -> Result<bool> {
    let host = targets::host()?;
    let mut missing: Vec<(Option<&str>, Vec<String>)> = Vec::new();
    for (name, tool) in tools {
        let build = config.build.merged(tool.build.as_ref());
        let build_target = build_args.target.as_deref().or(build.target.as_deref());
        let absent = targets::missing(&host, build_target, config.targets_for(tool));
        if absent.is_empty() {
            continue;
        }
        if !approve {
            bail!(
                "{} isn't an approved target for {name}; rerun with --approve-target to add it",
                absent.join(", ")
            );
        }
        missing.push((tool.targets.is_some().then_some(*name), absent));
    }
    if missing.is_empty() {
        return Ok(false);
    }
    let mut editor = ConfigEditor::open(&project.config_path())?;
    for (tool, absent) in &missing {
        editor.add_targets(*tool, absent);
        let place = tool.map_or("[settings]".to_string(), |t| format!("[tool.{t}]"));
        println!("Added {} to {place} targets", absent.join(", "));
    }
    editor.save()?;
    Ok(true)
}

/// Check every selected tool's binaries; with `--repair`, rebuild the ones that don't match
/// and require the rebuild to match the recorded checksum.
fn verify(project: &Project, config: &Config, args: &VerifyArgs) -> Result<ExitCode> {
    let tools = config.select(&args.names)?;
    let mut failed = Vec::new();
    let mut repairable = Vec::new();
    for (name, tool) in tools {
        match checksum::check(project, name) {
            Ok(mismatches) if mismatches.is_empty() => println!("{name}: ok"),
            Ok(mismatches) => {
                println!("{name}:");
                for m in &mismatches {
                    println!("  {m}");
                }
                failed.push(name);
                repairable.push((name, tool));
            }
            Err(e) => {
                println!("{name}: {e:#}");
                failed.push(name);
            }
        }
    }
    if args.repair && !repairable.is_empty() {
        check_targets(project, config, &repairable, &args.build, false)?;
        if !args.no_audit {
            audit_gate(
                project,
                config,
                &repairable,
                args.build.offline,
                args.deny_warnings,
            )?;
        }
        for (name, tool) in repairable {
            let build = config.build.merged(tool.build.as_ref());
            let repaired = install::install(
                project,
                name,
                tool,
                &build,
                &args.build,
                install::Options {
                    force: true,
                    approve_build: false,
                    checksums: install::Checksums::Expect,
                    targets: config.targets_for(tool),
                },
            );
            match repaired {
                Ok(()) => {
                    println!("{name}: repaired");
                    failed.retain(|f| *f != name);
                }
                Err(e) => eprintln!("error: {e:#}"),
            }
        }
    }
    if failed.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    println!("verification failed for {}", failed.join(", "));
    Ok(ExitCode::FAILURE)
}

/// Audit the selected tools' locks before building them.
fn audit_gate(
    project: &Project,
    config: &Config,
    tools: &[(&str, &config::Tool)],
    offline: bool,
    deny_warnings: bool,
) -> Result<()> {
    let policy = if offline {
        advisory_db::Policy::Offline
    } else {
        advisory_db::Policy::Auto
    };
    let (mut auditor, own) = load_auditor(project, config, policy)?;
    if own > 0 {
        bail!(
            "cargo-{} itself has open advisories; upgrade it, ignore them in .project-tools/{}, or pass --no-audit",
            cli::SUBCOMMAND,
            config::SELF_AUDIT_IGNORE
        );
    }
    let mut built_of = |name: &str, tool: &config::Tool| {
        graph::built_packages(project, name, tool, config.targets_for(tool), offline)
    };
    audit_before_install(
        project,
        &mut auditor,
        tools,
        deny_warnings,
        &audit::today(),
        &mut built_of,
    )
}

/// Fails, naming every tool that didn't pass, when a lock has a vulnerability that isn't
/// ignored, or with `deny_warnings` any warning.
fn audit_before_install(
    project: &Project,
    auditor: &mut audit::Auditor,
    tools: &[(&str, &config::Tool)],
    deny_warnings: bool,
    today: &str,
    built_of: &mut BuiltOf,
) -> Result<()> {
    let mut failed = Vec::new();
    for (name, tool) in tools {
        let outcome = audit_tool(project, auditor, name, tool, today, built_of)?;
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
        let built = project.stamp_path(name).is_file();
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
