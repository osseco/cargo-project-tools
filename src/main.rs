mod advisory_db;
mod audit;
mod build_files;
mod checksum;
mod cli;
mod config;
mod graph;
mod highlight;
mod inspect;
mod install;
mod lock;
mod net;
mod output;
mod process;
mod project;
mod review;
mod sanitize;
mod screen;
mod selfinstall;
mod setup;
mod setup_tui;
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
    AddArgs, AuditArgs, AuditDbCommand, CheckArgs, Cli, Command, InitArgs, InspectArgs,
    InstallArgs, UpdateArgs, UpgradeArgs, VerifyArgs,
};
use config::{AuditIgnore, Config, ConfigEditor, SfwMode};
use output::say;
use project::Project;
use selfinstall::SelfEnv;
use source::{Dest, Origin, git::GitRef};

fn main() -> ExitCode {
    let result = run();
    let code = match &result {
        Ok(code) => *code,
        Err(e) => {
            output::print_error(e);
            ExitCode::FAILURE
        }
    };
    output::finish(result.as_ref().err(), code == ExitCode::SUCCESS);
    code
}

fn run() -> Result<ExitCode> {
    // Invoked as `cargo project-tools …`, cargo passes the subcommand name as the first arg.
    let mut argv: Vec<_> = std::env::args_os().collect();
    if argv.get(1).is_some_and(|a| a == cli::SUBCOMMAND) {
        argv.remove(1);
    }
    let cli = Cli::parse_from(argv);
    output::init(cli.message_format, cli.command.has_summary());

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
        say!("{out}");
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
            // The child under sfw wrote the records.
            output::skip_summary();
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
        Command::Check(args) => check(&project, &config, &args)?,
        Command::Verify(args) => return verify(&project, &config, &args),
        Command::Audit(args) => return audit(&project, &config, args),
        Command::AuditDb(AuditDbCommand::Update) => {
            let max_age = config.settings.advisory_db_max_age()?;
            advisory_db::ensure(&project.advisory_db(), advisory_db::Policy::Force, max_age)?;
            say!(
                "Advisory database is up to date in {}",
                project.advisory_db().display()
            );
        }
        Command::List => list(&project, &config)?,
        Command::Inspect(args) => inspect(&project, &config, args)?,
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
        say!("{} already exists", path.display());
    } else {
        let targets = if args.targets.is_empty() {
            choose_targets(&project)?
        } else {
            Some(args.targets.clone())
        };
        fs::write(&path, config::template_with(targets.as_deref()))?;
        say!("Created {}", path.display());
        match targets {
            Some(t) => say!("Targets: {}", t.join(", ")),
            None => say!("Targets: all"),
        }
    }
    project.ensure_layout()?;
    say!("Prepared {}", project.tools_dir().display());
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
        say!(
            "note: no terminal to ask on; targets are {} (change `targets` in {} or pass --target)",
            all.join(", "),
            project::CONFIG_FILE
        );
        return Ok(Some(all));
    }
    say!("Targets to review, audit and install tools for:");
    for (i, s) in suggestions.iter().enumerate() {
        say!("  {}) {}  ({})", i + 1, s.triple, s.sources.join(", "));
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
            Err(e) => say!("  {e}"),
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
        bail!(
            "`{name}` is already in project-tools.toml; use `update {name} --latest` or `--version` to change its version"
        );
    }
    let origin = origin_of(req, &args.source)?;
    let checkout = source::checkout(project, name, &origin, review_dest(project, name))?;
    let (lock_text, start) =
        lock::shipped_or_generated(&checkout.manifest_path(), &checkout.lock_path(), &[])?;
    let kind = match start {
        lock::Start::Shipped => "its published Cargo.lock",
        lock::Start::Generated => "a generated Cargo.lock (it ships none)",
    };
    lock::check_root(&lock::parse(&lock_text)?, name, &checkout.version)?;
    let (checkout, features, default_features, lock_text) = if args.interactive {
        let mover: setup_tui::Mover = Box::new(|to| {
            let origin = moved_to(&origin, to);
            let checkout = source::checkout(project, name, &origin, review_dest(project, name))?;
            lock::shipped_or_generated(&checkout.manifest_path(), &checkout.lock_path(), &[])?;
            Ok(checkout)
        });
        let start = Start {
            features: &args.features,
            default_features: !args.no_default_features,
            targets: config.settings.targets.as_deref(),
        };
        let Some(setup) =
            interactive_setup(project, config, name, checkout, &origin, start, mover)?
        else {
            say!("Cancelled; {name} wasn't added");
            return Ok(());
        };
        let lock_text = fs::read_to_string(setup.lock_path())?;
        lock::check_root(&lock::parse(&lock_text)?, name, &setup.checkout.version)?;
        let features: Vec<String> = setup.selected.iter().cloned().collect();
        (setup.checkout, features, setup.default_features, lock_text)
    } else {
        (
            checkout,
            args.features.clone(),
            !args.no_default_features,
            lock_text,
        )
    };
    review_build(
        project,
        name,
        &install::feature_flags(&features, default_features),
        config.settings.targets.as_deref(),
        &config.build,
        &checkout,
        args.auto_approve,
    )?;
    write_new_lock(project, name, &lock_text)?;

    let mut entries: Vec<(&str, toml_edit::Value)> =
        vec![("version", checkout.version.as_str().into())];
    if let Some(url) = &args.source.git {
        entries.push(("git", url.as_str().into()));
        if let Some(b) = &args.source.branch {
            entries.push(("branch", b.as_str().into()));
        }
        if let Some(t) = &args.source.tag {
            entries.push(("tag", t.as_str().into()));
        }
        if let Some(commit) = &checkout.commit {
            entries.push(("rev", commit.as_str().into()));
        }
    }
    if !features.is_empty() {
        entries.push(("features", config::string_array(&features)));
    }
    if !default_features {
        entries.push(("default-features", false.into()));
    }
    if !args.bins.is_empty() {
        entries.push(("bins", config::string_array(&args.bins)));
    }
    editor.set_tool(name, &entries);
    editor.save()?;
    say!(
        "Added {name} {} with {kind} in {}",
        checkout.version,
        project.crate_dir(name).display()
    );
    Ok(())
}

/// Where `add` and `inspect` take a new tool from: crates.io at `req`, or git.
fn origin_of(req: Option<&str>, source: &cli::SourceArgs) -> Result<Origin> {
    Ok(match &source.git {
        Some(url) => {
            if req.is_some() {
                bail!("use --tag/--rev/--branch instead of @version with --git");
            }
            Origin::Git {
                url: url.clone(),
                reference: GitRef {
                    branch: source.branch.clone(),
                    tag: source.tag.clone(),
                    rev: source.rev.clone(),
                },
            }
        }
        None => Origin::Registry {
            version_req: req.map(str::to_string),
        },
    })
}

/// `inspect`: a configured tool from its own lock and settings, or a crate that isn't added
/// yet from the lock `add` would start with. Checks out into a temporary directory, never the
/// one `install` builds from, and records nothing.
fn inspect(project: &Project, config: &Config, args: InspectArgs) -> Result<()> {
    let (name, req) = split_spec(&args.spec);
    config::validate_tool_name(name)?;
    let configured = config
        .tool
        .get(name)
        .filter(|_| req.is_none() && args.source.git.is_none());
    let (checkout, origin, features, default_features, targets) = match configured {
        Some(tool) => {
            let ours = lock::load(&project.lock_path(name))?;
            lock::check_root(&ours, name, &tool.version)?;
            let origin = Origin::of(tool);
            let checkout = source::checkout(project, name, &origin, Dest::Temp)?;
            checkout.put_lock(&project.lock_path(name))?;
            let (features, default_features) =
                if args.features.is_empty() && !args.no_default_features {
                    (tool.features.clone(), tool.default_features != Some(false))
                } else {
                    (args.features.clone(), !args.no_default_features)
                };
            (
                checkout,
                origin,
                features,
                default_features,
                config.targets_for(tool),
            )
        }
        None => {
            if config.tool.contains_key(name) {
                say!("note: {name} is configured; inspecting the given version or source instead");
            }
            let origin = origin_of(req, &args.source)?;
            let checkout = source::checkout(project, name, &origin, Dest::Temp)?;
            let (lock_text, _) =
                lock::shipped_or_generated(&checkout.manifest_path(), &checkout.lock_path(), &[])?;
            lock::check_root(&lock::parse(&lock_text)?, name, &checkout.version)?;
            (
                checkout,
                origin,
                args.features.clone(),
                !args.no_default_features,
                config.settings.targets.as_deref(),
            )
        }
    };
    let (auditor, _) = load_auditor(project, config, advisory_db::Policy::Auto)?;
    let ignore = AuditIgnore::load(&project.audit_ignore_path(name))?;
    say!("Resolving what {name} builds with each feature…");
    let mut setup = setup::Setup::new(
        name,
        checkout,
        matches!(origin, Origin::Git { .. }),
        features.into_iter().collect(),
        default_features,
        targets.map(<[String]>::to_vec),
        auditor,
        ignore,
        Vec::new(),
    )?;
    let recorded = review::recorded(&project.review_path(name))?;
    let found = inspect::inspect(&mut setup, &recorded)?;
    inspect::print(&found);
    output::emit(output::Record::Inspect(found));
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
    for file in [
        checksum::path(project, name),
        project.stamp_path(name),
        project.built_path(name),
    ] {
        if file.exists() {
            fs::remove_file(&file).with_context(|| format!("removing {}", file.display()))?;
        }
    }
    let mut editor = ConfigEditor::open(&project.config_path())?;
    editor.remove_tool(name);
    editor.save()?;
    say!("Removed {name}");
    Ok(())
}

/// `upgrade`, kept for now: `update --latest` or `update --version`.
fn upgrade(project: &Project, config: &Config, args: UpgradeArgs) -> Result<()> {
    eprintln!(
        "note: `upgrade` is deprecated; use `update {} --latest` or `--version <req>`",
        args.name
    );
    // `upgrade` ignored `--version` for git tools, and always moved to the newest.
    let git = config.tool(&args.name)?.git.is_some();
    let version = args.version.filter(|_| !git);
    update(
        project,
        config,
        UpdateArgs {
            interactive: false,
            latest: version.is_none(),
            version,
            name: args.name,
            deps: Vec::new(),
            precise: None,
            recursive: false,
            features: None,
            no_default_features: false,
            default_features: false,
            auto_approve: args.auto_approve,
        },
    )
}

/// Where a tool from `origin` goes for the setup screen's move: that exact version, or for
/// `None` the newest (a git tool: the head of its branch or tag again).
fn moved_to(origin: &Origin, to: Option<&semver::Version>) -> Origin {
    match origin {
        Origin::Registry { .. } => Origin::Registry {
            version_req: to.map(|v| format!("={v}")),
        },
        Origin::Git { url, reference } => {
            let mut reference = reference.clone();
            reference.rev = None;
            Origin::Git {
                url: url.clone(),
                reference,
            }
        }
    }
}

/// Where the setup screen starts: the features, and the targets the tree is for.
struct Start<'a> {
    features: &'a [String],
    default_features: bool,
    targets: Option<&'a [String]>,
}

/// Open the setup screen for `name` in `checkout`. `None` when cancelled.
fn interactive_setup(
    project: &Project,
    config: &Config,
    name: &str,
    checkout: source::Checkout,
    origin: &Origin,
    start: Start,
    mover: setup_tui::Mover,
) -> Result<Option<setup::Setup>> {
    if !screen::interactive() {
        bail!("--interactive needs a terminal");
    }
    let (auditor, _) = load_auditor(project, config, advisory_db::Policy::Auto)?;
    let ignore = AuditIgnore::load(&project.audit_ignore_path(name))?;
    say!("Resolving what {name} builds…");
    let setup = setup::Setup::new(
        name,
        checkout,
        matches!(origin, Origin::Git { .. }),
        start.features.iter().cloned().collect(),
        start.default_features,
        start.targets.map(<[String]>::to_vec),
        auditor,
        ignore,
        Vec::new(),
    )?;
    setup_tui::run(setup, mover)
}

/// `update -i`: the setup screen from the tool's current version, features and lock.
fn update_interactive(project: &Project, config: &Config, args: UpdateArgs) -> Result<()> {
    let tool = config.tool(&args.name)?;
    let name = args.name.as_str();
    let ours_path = project.lock_path(name);
    let ours = lock::load(&ours_path)?;
    lock::check_root(&ours, name, &tool.version)?;
    let origin = Origin::of(tool);
    let checkout = source::checkout(project, name, &origin, review_dest(project, name))?;
    checkout.put_lock(&ours_path)?;
    let mover: setup_tui::Mover =
        Box::new(|to| move_tool(project, name, &moved_to(&origin, to), &ours));
    let start = Start {
        features: &tool.features,
        default_features: tool.default_features != Some(false),
        targets: config.targets_for(tool),
    };
    let Some(setup) = interactive_setup(project, config, name, checkout, &origin, start, mover)?
    else {
        say!("Cancelled; nothing changed");
        return Ok(());
    };

    let updated = lock::load(&setup.lock_path())?;
    lock::check_root(&updated, name, &setup.checkout.version)?;
    let changes = lock::diff(&ours, &updated);
    let features: Vec<String> = setup.selected.iter().cloned().collect();
    let before: std::collections::BTreeSet<&String> = tool.features.iter().collect();
    let features_changed = setup
        .selected
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        != before
        || setup.default_features != (tool.default_features != Some(false));
    let moved = setup.checkout.version != tool.version
        || setup.checkout.commit.is_some() && setup.checkout.commit != tool.rev;
    say!("Changes to {name}'s lock:");
    lock::print_diff(&changes);
    if changes.is_empty() && !features_changed && !moved {
        say!("Nothing changed");
        return Ok(());
    }
    review_build(
        project,
        name,
        &setup.feature_args(),
        config.targets_for(tool),
        &config.build.merged(tool.build.as_ref()),
        &setup.checkout,
        args.auto_approve,
    )?;
    fs::copy(setup.lock_path(), &ours_path)?;
    let mut editor = ConfigEditor::open(&project.config_path())?;
    let mut entries: Vec<(&str, toml_edit::Value)> = Vec::new();
    if moved {
        entries.push(("version", setup.checkout.version.as_str().into()));
        if let Some(commit) = &setup.checkout.commit {
            entries.push(("rev", commit.as_str().into()));
        }
    }
    if features_changed {
        set_features(
            &mut editor,
            name,
            &mut entries,
            &features,
            setup.default_features,
        );
    }
    editor.set_tool(name, &entries);
    editor.save()?;
    if moved {
        say!(
            "Moved {name} {} -> {}",
            tool.version,
            setup.checkout.version
        );
    }
    Ok(())
}

/// Write `features` and `default-features` for `name`: into `entries` when they differ
/// from cargo's defaults, else removed from project-tools.toml.
fn set_features(
    editor: &mut ConfigEditor,
    name: &str,
    entries: &mut Vec<(&str, toml_edit::Value)>,
    features: &[String],
    default_features: bool,
) {
    let mut unset = Vec::new();
    if features.is_empty() {
        unset.push("features");
    } else {
        entries.push(("features", config::string_array(features)));
    }
    if default_features {
        unset.push("default-features");
    } else {
        entries.push(("default-features", false.into()));
    }
    editor.unset_tool(name, &unset);
}

/// The features `update` builds with: `-F` replaces the list, and the default-features flags
/// override the tool's setting. Returns them and whether they differ from the tool's.
fn updated_features(tool: &config::Tool, args: &UpdateArgs) -> (Vec<String>, bool, bool) {
    let before = tool.default_features != Some(false);
    let features: Vec<String> = match &args.features {
        Some(list) => list.iter().filter(|f| !f.is_empty()).cloned().collect(),
        None => tool.features.clone(),
    };
    let default_features = if args.no_default_features {
        false
    } else {
        args.default_features || before
    };
    let sorted = |l: &[String]| l.iter().cloned().collect::<std::collections::BTreeSet<_>>();
    let changed = sorted(&features) != sorted(&tool.features) || default_features != before;
    (features, default_features, changed)
}

/// Where `update --version`/`--latest` moves the tool, or `None` when it stays.
fn moved_origin(
    tool: &config::Tool,
    version: Option<&str>,
    latest: bool,
) -> Result<Option<Origin>> {
    if version.is_none() && !latest {
        return Ok(None);
    }
    Ok(Some(match Origin::of(tool) {
        Origin::Registry { .. } => Origin::Registry {
            version_req: version.map(str::to_string),
        },
        Origin::Git { .. } if version.is_some() => {
            bail!("--version is for registry tools; use --latest to follow the branch or tag again")
        }
        // Follow the branch/tag again instead of the pinned commit.
        Origin::Git { url, mut reference } => {
            reference.rev = None;
            Origin::Git { url, reference }
        }
    }))
}

/// Check out the tool at `origin` with the lock it ships (or one seeded from ours), keeping
/// the dependency bumps in `ours` that still resolve.
fn move_tool(
    project: &Project,
    name: &str,
    origin: &Origin,
    ours: &cargo_lock::Lockfile,
) -> Result<source::Checkout> {
    let checkout = source::checkout(project, name, origin, review_dest(project, name))?;
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
    for (dep, from, to) in lock::newer_than(ours, &shipped) {
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
            Ok(()) => say!(
                "  kept {} {to} (new lock has {from})",
                sanitize::shown_line(&dep)
            ),
            Err(_) => {
                fs::write(&lock_path, backup)?;
                say!(
                    "  dropped {} {to}: does not resolve with {name} {}",
                    sanitize::shown_line(&dep),
                    checkout.version
                );
            }
        }
    }
    Ok(checkout)
}

fn update(project: &Project, config: &Config, args: UpdateArgs) -> Result<()> {
    if args.interactive {
        return update_interactive(project, config, args);
    }
    let tool = config.tool(&args.name)?;
    let name = args.name.as_str();
    if args.precise.is_some() && args.deps.len() != 1 {
        bail!("--precise needs exactly one dependency");
    }
    let ours_path = project.lock_path(name);
    let ours = lock::load(&ours_path)?;
    lock::check_root(&ours, name, &tool.version)?;

    let moved = moved_origin(tool, args.version.as_deref(), args.latest)?;
    // A full checkout rather than a scratch project: the review needs the real build scripts.
    let checkout = match &moved {
        Some(origin) => move_tool(project, name, origin, &ours)?,
        None => {
            let checkout =
                source::checkout(project, name, &Origin::of(tool), review_dest(project, name))?;
            checkout.put_lock(&ours_path)?;
            checkout
        }
    };
    let (features, default_features, features_changed) = updated_features(tool, &args);
    let features_only =
        args.features.is_some() || args.no_default_features || args.default_features;
    // Without dependencies named, a plain `update` updates them all; a move or a change of
    // features leaves them be.
    if !args.deps.is_empty() || (moved.is_none() && !features_only) {
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
    }

    let updated = lock::load(&checkout.lock_path())?;
    lock::check_root(&updated, name, &checkout.version)?;
    let changes = lock::diff(&ours, &updated);
    say!("Changes to {name}'s lock:");
    lock::print_diff(&changes);
    let moved = moved.is_some()
        && (checkout.version != tool.version
            || checkout.commit.is_some() && checkout.commit != tool.rev);
    if changes.is_empty() && !moved && !features_changed {
        if args.version.is_some() || args.latest {
            say!("{name} is already at {}", checkout.version);
        } else if features_only {
            say!("{name} already builds with these features");
        }
        return Ok(());
    }
    review_build(
        project,
        name,
        &install::feature_flags(&features, default_features),
        config.targets_for(tool),
        &config.build.merged(tool.build.as_ref()),
        &checkout,
        args.auto_approve,
    )?;
    fs::copy(checkout.lock_path(), &ours_path)?;
    if moved || features_changed {
        let mut editor = ConfigEditor::open(&project.config_path())?;
        let mut entries: Vec<(&str, toml_edit::Value)> = Vec::new();
        if moved {
            entries.push(("version", checkout.version.as_str().into()));
            if let Some(commit) = &checkout.commit {
                entries.push(("rev", commit.as_str().into()));
            }
        }
        if features_changed {
            set_features(&mut editor, name, &mut entries, &features, default_features);
        }
        editor.set_tool(name, &entries);
        editor.save()?;
    }
    if moved {
        say!("Moved {name} {} -> {}", tool.version, checkout.version);
    }
    if features_changed {
        let list = if features.is_empty() {
            "none".to_string()
        } else {
            features.join(", ")
        };
        let defaults = if default_features { "on" } else { "off" };
        say!("{name} now builds with features: {list}; default features {defaults}");
    }
    Ok(())
}

/// Where `add` and `update` check out the tool: the place `install` builds from,
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
    build: &config::Build,
    checkout: &source::Checkout,
    approve: bool,
) -> Result<()> {
    let vendored = vendor::vendor(project, name, checkout, false, &[])?;
    let config = install::resolve_config(build, &[]);
    let selection = graph::Selection {
        features,
        targets,
        config: &config,
    };
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
    say!("cargo-{}:", cli::SUBCOMMAND);
    let open = audit::evaluate_self(&found, &ignore, &audit::today());
    Ok((auditor, open))
}

/// The packages a tool builds with its features: `built_for` outside tests.
type BuiltOf<'a> = dyn FnMut(&str, &config::Tool) -> Result<graph::Built> + 'a;

/// The packages `tool` builds for its targets, which are named when they're limited.
fn built_for(
    project: &Project,
    config: &Config,
    name: &str,
    tool: &config::Tool,
    offline: bool,
) -> Result<graph::Built> {
    let targets = config.targets_for(tool);
    if let Some(list) = targets {
        say!("  targets: {}", list.join(", "));
    }
    let build = config.build.merged(tool.build.as_ref());
    let resolve = install::resolve_config(&build, &[]);
    graph::built_packages(project, name, tool, targets, &resolve, offline)
}

fn audit_tool(
    project: &Project,
    auditor: &mut audit::Auditor,
    name: &str,
    tool: &config::Tool,
    today: &str,
    built_of: &mut BuiltOf,
) -> Result<audit::Outcome> {
    say!("{name}:");
    let built = built_of(name, tool)?;
    let report = auditor.audit(&project.lock_path(name), name, tool.git.is_some(), &built)?;
    if report.skipped > 0 {
        say!(
            "  skipped {} packages not built with these features",
            report.skipped
        );
    }
    let ignore = AuditIgnore::load(&project.audit_ignore_path(name))?;
    let outcome = audit::evaluate(name, &report, &ignore, today, &mut auditor.index);
    if outcome.vulnerabilities == 0 && outcome.warnings == 0 {
        say!("  ok");
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
    let mut built_of =
        |name: &str, tool: &config::Tool| built_for(project, config, name, tool, false);
    for (name, tool) in tools {
        let outcome = audit_tool(project, &mut auditor, name, tool, &today, &mut built_of)?;
        vulns += outcome.vulnerabilities;
        warns += outcome.warnings;
    }
    say!("{vulns} vulnerabilities, {warns} warnings");
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
                auto_approve: args.auto_approve,
                stage: install::Stage::Install,
                checksums: install::Checksums::Write,
                targets: config.targets_for(tool),
            },
        )?;
    }
    Ok(())
}

/// `check`: the targets, the audit and each tool's build review, as `install` checks them,
/// without building or recording anything. Keeps going after a failure, so one run reports
/// everything.
fn check(project: &Project, config: &Config, args: &CheckArgs) -> Result<()> {
    let tools = config.select(&args.names)?;
    let mut failed: Vec<String> = Vec::new();
    if let Err(e) = check_targets(project, config, &tools, &args.build, false) {
        output::print_error(&e);
        failed.push("targets".to_string());
    }
    if !args.no_audit
        && let Err(e) = audit_gate(
            project,
            config,
            &tools,
            args.build.offline,
            args.deny_warnings,
        )
    {
        output::print_error(&e);
        failed.push("audit".to_string());
    }
    for (name, tool) in &tools {
        let build = config.build.merged(tool.build.as_ref());
        let checked = install::install(
            project,
            name,
            tool,
            &build,
            &args.build,
            install::Options {
                force: false,
                auto_approve: false,
                stage: install::Stage::Check,
                checksums: install::Checksums::Write,
                targets: config.targets_for(tool),
            },
        );
        if let Err(e) = checked {
            output::print_error(&e);
            failed.push(name.to_string());
        }
    }
    if !failed.is_empty() {
        bail!("check failed ({}); nothing was built", failed.join(", "));
    }
    say!("All checks passed; nothing was built");
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
    // Only `--target` and `[build].target` are checked against the approved targets.
    if std::env::var_os("CARGO_BUILD_TARGET").is_some() {
        bail!("CARGO_BUILD_TARGET is set; use --target or [build].target instead");
    }
    if build_args
        .config
        .iter()
        .any(|c| c.trim_start().starts_with("build.target"))
    {
        bail!("--config build.target isn't checked against the targets; use --target instead");
    }
    let host = targets::host()?;
    let mut missing: Vec<(&str, Option<&str>, Vec<String>)> = Vec::new();
    for (name, tool) in tools {
        let build = config.build.merged(tool.build.as_ref());
        let build_target = build_args.target.as_deref().or(build.target.as_deref());
        let absent = targets::missing(&host, build_target, config.targets_for(tool));
        if !absent.is_empty() {
            missing.push((name, tool.targets.is_some().then_some(*name), absent));
        }
    }
    if missing.is_empty() {
        return Ok(false);
    }
    if !approve {
        // On stderr, so the list comes before the error on the same stream.
        for (name, _, absent) in &missing {
            eprintln!("  {name}: {} isn't an approved target", absent.join(", "));
            output::emit(output::Record::TargetMissing {
                tool: name.to_string(),
                file: project::CONFIG_FILE.to_string(),
                targets: absent.clone(),
            });
        }
        if screen::interactive() {
            bail!("rerun with --approve-target to add the missing targets");
        }
        bail!(
            "add the missing targets to project-tools.toml where you review (by hand, or by \
             rerunning there with --approve-target), review what they add, then commit \
             project-tools.toml"
        );
    }
    let mut editor = ConfigEditor::open(&project.config_path())?;
    for (_, tool, absent) in &missing {
        editor.add_targets(*tool, absent);
        let place = tool.map_or("[settings]".to_string(), |t| format!("[tool.{t}]"));
        say!("Added {} to {place} targets", absent.join(", "));
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
            Ok(mismatches) if mismatches.is_empty() => say!("{name}: ok"),
            Ok(mismatches) => {
                say!("{name}:");
                for m in &mismatches {
                    say!("  {m}");
                }
                failed.push(name);
                repairable.push((name, tool));
            }
            Err(e) => {
                say!("{name}: {e:#}");
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
                    auto_approve: false,
                    stage: install::Stage::Install,
                    checksums: install::Checksums::Expect,
                    targets: config.targets_for(tool),
                },
            );
            match repaired {
                Ok(()) => {
                    say!("{name}: repaired");
                    failed.retain(|f| *f != name);
                }
                Err(e) => output::print_error(&e),
            }
        }
    }
    if failed.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    say!("verification failed for {}", failed.join(", "));
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
    let mut built_of =
        |name: &str, tool: &config::Tool| built_for(project, config, name, tool, offline);
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
        say!(
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
