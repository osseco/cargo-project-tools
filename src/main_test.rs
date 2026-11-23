use std::path::Path;

use super::*;

const REGISTRY: &str = "registry+https://github.com/rust-lang/crates.io-index";

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// `badtool` has an advisory against itself; `goodtool` only depends on an unmaintained
/// crate, which is a warning.
fn fixture() -> (tempfile::TempDir, Project, audit::Auditor, Config) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project {
        root: dir.path().to_path_buf(),
    };
    let db_dir = dir.path().join("db");
    for (id, package, extra) in [
        ("RUSTSEC-2020-0001", "badtool", ""),
        (
            "RUSTSEC-2020-0002",
            "unm",
            "informational = \"unmaintained\"",
        ),
    ] {
        write(
            &db_dir.join("crates").join(package).join(format!("{id}.md")),
            &format!(
                "```toml\n[advisory]\nid = \"{id}\"\npackage = \"{package}\"\ndate = \"2020-01-01\"\n{extra}\n\n[versions]\npatched = [\">= 2.0.0\"]\n```\n\n# Title\n\nDescription.\n"
            ),
        );
    }
    write(
        &project.lock_path("badtool"),
        "version = 3\n\n[[package]]\nname = \"badtool\"\nversion = \"1.0.0\"\n",
    );
    write(
        &project.lock_path("goodtool"),
        &format!(
            "version = 3\n\n[[package]]\nname = \"goodtool\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"unm\"\nversion = \"1.0.0\"\nsource = \"{REGISTRY}\"\n"
        ),
    );
    let mut auditor = audit::Auditor::new(rustsec::Database::open(&db_dir).unwrap());
    for name in ["badtool", "goodtool", "unm"] {
        auditor.index.insert(name, Vec::new());
    }
    let config: Config = toml::from_str(
        "[tool.badtool]\nversion = \"1.0.0\"\n[tool.goodtool]\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    (dir, project, auditor, config)
}

fn gate(names: &[&str], deny_warnings: bool, extra_ignore: Option<(&str, &str)>) -> Result<()> {
    let (_dir, project, mut auditor, config) = fixture();
    if let Some((tool, id)) = extra_ignore {
        write(
            &project.audit_ignore_path(tool),
            &format!("[[ignore]]\nid = \"{id}\"\nreason = \"test\"\n"),
        );
    }
    let names: Vec<String> = names.iter().map(ToString::to_string).collect();
    let tools = config.select(&names).unwrap();
    // Every locked package is built.
    let mut built_of = |name: &str, _: &config::Tool| {
        let lock = lock::load(&project.lock_path(name))?;
        Ok(lock
            .packages
            .iter()
            .map(|p| (p.name.as_str().to_string(), p.version.clone()))
            .collect())
    };
    audit_before_install(
        &project,
        &mut auditor,
        &tools,
        deny_warnings,
        "2026-01-01",
        &mut built_of,
    )
}

#[test]
fn vulnerable_tool_blocks_the_whole_install() {
    let err = gate(&[], false, None).unwrap_err().to_string();
    assert!(err.contains("audit failed for badtool;"), "{err}");
    assert!(err.contains("nothing was installed"), "{err}");
}

#[test]
fn warnings_pass_unless_denied() {
    assert!(gate(&["goodtool"], false, None).is_ok());
    let err = gate(&["goodtool"], true, None).unwrap_err().to_string();
    assert!(err.contains("audit failed for goodtool;"), "{err}");
}

#[test]
fn every_failing_tool_is_named() {
    let err = gate(&[], true, None).unwrap_err().to_string();
    assert!(err.contains("badtool, goodtool"), "{err}");
}

#[test]
fn ignored_advisory_lets_it_through() {
    assert!(gate(&["badtool"], false, Some(("badtool", "RUSTSEC-2020-0001"))).is_ok());
    assert!(gate(&["goodtool"], true, Some(("goodtool", "RUSTSEC-2020-0002"))).is_ok());
}

#[test]
fn install_flags() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(
            ["cargo-project-tools"]
                .iter()
                .chain(args)
                .copied()
                .collect::<Vec<_>>(),
        )
    };
    assert!(parse(&["install", "--no-audit", "--deny-warnings"]).is_err());
    let Command::Install(args) = parse(&["install", "--no-audit", "x"]).unwrap().command else {
        panic!("not install");
    };
    assert!(args.no_audit && !args.deny_warnings);
    assert_eq!(args.names, vec!["x".to_string()]);
    // Moved to `check`; renamed to `--auto-approve`.
    assert!(parse(&["install", "--check"]).is_err());
    assert!(parse(&["install", "--approve-build"]).is_err());
    assert!(parse(&["install", "--auto-approve"]).is_ok());
}

#[test]
fn check_flags() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(
            ["cargo-project-tools"]
                .iter()
                .chain(args)
                .copied()
                .collect::<Vec<_>>(),
        )
    };
    assert!(parse(&["check", "--no-audit", "--deny-warnings"]).is_err());
    // It approves and builds nothing.
    for flag in ["--auto-approve", "--approve-target"] {
        assert!(parse(&["check", flag]).is_err(), "{flag}");
    }
    // `--force` checks and prepares up-to-date tools too.
    let Command::Check(args) = parse(&["check", "--force"]).unwrap().command else {
        panic!("not check");
    };
    assert!(args.force);
    let cmd = parse(&["check", "--target", "x86_64-unknown-linux-gnu", "x"])
        .unwrap()
        .command;
    assert!(cmd.downloads_packages() && cmd.has_summary());
    let Command::Check(args) = cmd else {
        panic!("not check");
    };
    assert_eq!(
        args.build.target.as_deref(),
        Some("x86_64-unknown-linux-gnu")
    );
    assert_eq!(args.names, vec!["x".to_string()]);
}

#[test]
fn review_flags() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(
            ["cargo-project-tools"]
                .iter()
                .chain(args)
                .copied()
                .collect::<Vec<_>>(),
        )
    };
    // It reviews; it doesn't build, audit or approve targets.
    for flag in ["--force", "--approve-target", "--no-audit", "--profile=x"] {
        assert!(parse(&["review", flag]).is_err(), "{flag}");
    }
    let cmd = parse(&["review", "--auto-approve", "--offline", "x"])
        .unwrap()
        .command;
    assert!(cmd.downloads_packages() && cmd.has_summary());
    let Command::Review(args) = cmd else {
        panic!("not review");
    };
    assert!(args.auto_approve && args.offline);
    assert_eq!(args.names, vec!["x".to_string()]);

    let cmd = parse(&[
        "review",
        "x",
        "--approve",
        "a@1",
        "--approve",
        "b@2=ff",
        "--strip",
        "c@3",
        "--override",
        "d@4",
    ])
    .unwrap()
    .command;
    let Command::Review(args) = cmd else {
        panic!("not review");
    };
    assert_eq!(args.approve, ["a@1", "b@2=ff"]);
    assert_eq!(args.strip, ["c@3"]);
    assert_eq!(args.override_, ["d@4"]);
    // Picking pieces and approving them all don't mix.
    assert!(parse(&["review", "x", "--auto-approve", "--approve", "a@1"]).is_err());
    let cmd = parse(&["review", "x", "--from", "a.toml", "--from", "b.toml"])
        .unwrap()
        .command;
    let Command::Review(args) = cmd else {
        panic!("not review");
    };
    assert_eq!(args.from.len(), 2);
    assert!(parse(&["review", "x", "--from", "a.toml", "--auto-approve"]).is_err());
    // `--diff` only shows.
    assert!(parse(&["review", "x", "--diff"]).is_ok());
    for flag in [
        "--auto-approve",
        "--approve=a@1",
        "--strip=a@1",
        "--override=a@1",
    ] {
        assert!(parse(&["review", "x", "--diff", flag]).is_err(), "{flag}");
    }
}

#[test]
fn verify_flags() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(
            ["cargo-project-tools"]
                .iter()
                .chain(args)
                .copied()
                .collect::<Vec<_>>(),
        )
    };
    // Audit and build flags only make sense when repairing.
    assert!(parse(&["verify", "--no-audit"]).is_err());
    assert!(parse(&["verify", "--deny-warnings"]).is_err());
    let plain = parse(&["verify"]).unwrap().command;
    assert!(!plain.downloads_packages());
    // An offline repair or install only uses what `check` prepared.
    let offline = parse(&["verify", "--repair", "--offline"]).unwrap().command;
    assert!(!offline.downloads_packages());
    assert!(
        !parse(&["install", "--offline"])
            .unwrap()
            .command
            .downloads_packages()
    );
    assert!(parse(&["install"]).unwrap().command.downloads_packages());
    // `audit` goes through sfw (and reports a missing one) even when it downloads nothing.
    assert!(parse(&["audit"]).unwrap().command.downloads_packages());
    assert!(
        !parse(&["audit-db", "update"])
            .unwrap()
            .command
            .downloads_packages()
    );
    let repair = parse(&["verify", "--repair", "--profile", "dist", "x"])
        .unwrap()
        .command;
    assert!(repair.downloads_packages());
    let Command::Verify(args) = repair else {
        panic!("not verify");
    };
    assert_eq!(args.build.profile.as_deref(), Some("dist"));
    assert_eq!(args.names, vec!["x".to_string()]);
}

fn tools() -> Config {
    toml::from_str(
        "[tool.reg]\nversion = \"1.0.0\"\n\n[tool.gitty]\nversion = \"0.1.0\"\ngit = \"https://example.com/g.git\"\nbranch = \"main\"\nrev = \"abc\"\n",
    )
    .unwrap()
}

#[test]
fn update_moves_only_when_asked() {
    let config = tools();
    let reg = config.tool("reg").unwrap();
    assert!(moved_origin(reg, None, false).unwrap().is_none());
    match moved_origin(reg, Some("^1.2"), false).unwrap() {
        Some(Origin::Registry { version_req }) => assert_eq!(version_req.as_deref(), Some("^1.2")),
        other => panic!("{other:?}"),
    }
    match moved_origin(reg, None, true).unwrap() {
        Some(Origin::Registry { version_req }) => assert_eq!(version_req, None),
        other => panic!("{other:?}"),
    }
}

fn update_args(args: &[&str]) -> UpdateArgs {
    let cli = Cli::try_parse_from(
        ["cargo-project-tools", "update", "t"]
            .iter()
            .chain(args)
            .copied()
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let Command::Update(args) = cli.command else {
        panic!("not update");
    };
    args
}

#[test]
fn update_features_replace_or_keep_the_tools() {
    let config: Config = toml::from_str(
        "[tool.t]\nversion = \"1.0.0\"\nfeatures = [\"a\", \"b\"]\ndefault-features = false\n",
    )
    .unwrap();
    let tool = config.tool("t").unwrap();
    let features = |args: &[&str]| updated_features(tool, &update_args(args));

    assert_eq!(features(&[]), (vec!["a".into(), "b".into()], false, false));
    // The same list in another order is no change.
    assert_eq!(
        features(&["-F", "b,a"]),
        (vec!["b".into(), "a".into()], false, false)
    );
    assert_eq!(features(&["-F", "c"]), (vec!["c".into()], false, true));
    assert_eq!(features(&["-F", ""]), (vec![], false, true));
    assert_eq!(
        features(&["--default-features"]),
        (vec!["a".into(), "b".into()], true, true)
    );
    assert!(!features(&["--no-default-features"]).2);
}

#[test]
fn a_git_tool_follows_its_branch_again_and_takes_no_version() {
    let config = tools();
    let git = config.tool("gitty").unwrap();
    match moved_origin(git, None, true).unwrap() {
        Some(Origin::Git { reference, .. }) => {
            assert_eq!(reference.rev, None);
            assert_eq!(reference.branch.as_deref(), Some("main"));
        }
        other => panic!("{other:?}"),
    }
    let err = moved_origin(git, Some("1"), false).unwrap_err().to_string();
    assert!(err.contains("--latest"), "{err}");
}

#[test]
fn update_flags() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(
            ["cargo-project-tools"]
                .iter()
                .chain(args)
                .copied()
                .collect::<Vec<_>>(),
        )
    };
    assert!(parse(&["update", "t", "--version", "1", "--latest"]).is_err());
    assert!(parse(&["update", "t", "--latest", "serde"]).is_ok());
    assert!(parse(&["update", "t", "-i"]).is_ok());
    for clash in [
        &["--latest"][..],
        &["--version", "1"],
        &["serde"],
        &["--recursive"],
        &["-F", "x"],
        &["--no-default-features"],
        &["--default-features"],
    ] {
        let mut args = vec!["update", "t", "-i"];
        args.extend_from_slice(clash);
        assert!(parse(&args).is_err(), "{args:?}");
    }
    assert!(parse(&["add", "cargo-nextest", "-i", "--no-default-features"]).is_ok());
    assert!(parse(&["add", "cargo-nextest", "--fix-advisories"]).is_ok());
    assert!(parse(&["add", "cargo-nextest", "-i", "--fix-advisories"]).is_err());
    assert!(parse(&["update", "cargo-nextest", "--fix-advisories"]).is_ok());
    assert!(parse(&["update", "cargo-nextest", "-i", "--fix-advisories"]).is_err());
    assert!(parse(&["update", "t", "--default-features", "--no-default-features"]).is_err());
    assert!(parse(&["update", "t", "-F", "a,b", "--latest"]).is_ok());
    // `upgrade` still parses, but isn't listed.
    assert!(parse(&["upgrade", "t"]).is_ok());
    let help = <Cli as clap::CommandFactory>::command()
        .render_help()
        .to_string();
    assert!(help.contains("update"), "{help}");
    assert!(!help.contains("upgrade"), "{help}");
}
