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
    audit_before_install(&project, &mut auditor, &tools, deny_warnings, "2026-01-01")
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
