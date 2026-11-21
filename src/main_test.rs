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
    ] {
        let mut args = vec!["update", "t", "-i"];
        args.extend_from_slice(clash);
        assert!(parse(&args).is_err(), "{args:?}");
    }
    assert!(parse(&["add", "cargo-nextest", "-i", "--no-default-features"]).is_ok());
    // `upgrade` still parses, but isn't listed.
    assert!(parse(&["upgrade", "t"]).is_ok());
    let help = <Cli as clap::CommandFactory>::command()
        .render_help()
        .to_string();
    assert!(help.contains("update"), "{help}");
    assert!(!help.contains("upgrade"), "{help}");
}
