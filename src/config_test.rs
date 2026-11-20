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

#[test]
fn advisory_db_max_age() {
    assert_eq!(
        parse_max_age("12h").unwrap(),
        Duration::from_secs(12 * 3600)
    );
    assert_eq!(
        parse_max_age("7d").unwrap(),
        Duration::from_secs(7 * 86_400)
    );
    for bad in ["", "d", "1w", "1.5d", "-1d", "1é"] {
        assert!(parse_max_age(bad).is_err(), "{bad}");
    }
    let config: Config = toml::from_str("[settings]\nadvisory-db-max-age = \"2d\"\n").unwrap();
    assert_eq!(
        config.settings.advisory_db_max_age().unwrap(),
        Duration::from_secs(2 * 86_400)
    );
    assert_eq!(
        Settings::default().advisory_db_max_age().unwrap(),
        DEFAULT_ADVISORY_DB_MAX_AGE
    );
}

#[test]
fn self_ignores_need_an_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SELF_AUDIT_IGNORE);
    assert!(AuditIgnore::load_self(&path).unwrap().ignore.is_empty());
    fs::write(
        &path,
        "[[ignore]]\nid = \"RUSTSEC-2020-0001\"\nreason = \"x\"\n",
    )
    .unwrap();
    let err = AuditIgnore::load_self(&path).unwrap_err().to_string();
    assert!(err.contains("needs an `expires` date"), "{err}");
    fs::write(
        &path,
        "[[ignore]]\nid = \"RUSTSEC-2020-0001\"\nreason = \"x\"\nexpires = \"2027-01-01\"\n",
    )
    .unwrap();
    assert_eq!(AuditIgnore::load_self(&path).unwrap().ignore.len(), 1);
}

#[test]
fn tool_targets_replace_the_project_list() {
    let config: Config = toml::from_str(
        "[settings]\ntargets = [\"a\", \"b\"]\n[tool.x]\nversion = \"1.0.0\"\n[tool.y]\nversion = \"1.0.0\"\ntargets = [\"c\"]\n",
    )
    .unwrap();
    assert_eq!(config.targets_for(&config.tool["x"]).unwrap(), ["a", "b"]);
    assert_eq!(config.targets_for(&config.tool["y"]).unwrap(), ["c"]);
    let unset: Config = toml::from_str("[tool.x]\nversion = \"1.0.0\"\n").unwrap();
    assert!(unset.targets_for(&unset.tool["x"]).is_none());
}

#[test]
fn bad_target_lists_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project-tools.toml");
    for (text, needle) in [
        ("[settings]\ntargets = []\n", "is empty"),
        ("[tool.x]\nversion = \"1.0.0\"\ntargets = []\n", "[tool.x]"),
        ("[settings]\ntargets = [\"--x\"]\n", "not a target triple"),
    ] {
        fs::write(&path, text).unwrap();
        let err = format!("{:#}", Config::load(&path).unwrap_err());
        assert!(err.contains(needle), "{text}: {err}");
    }
}

#[test]
fn editor_adds_targets_where_they_apply() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project-tools.toml");
    fs::write(
        &path,
        "[settings]\n# ours\ntargets = [\"a\"]\n\n[tool.y]\nversion = \"1.0.0\"\ntargets = [\"c\"]\n",
    )
    .unwrap();
    let mut ed = ConfigEditor::open(&path).unwrap();
    ed.add_targets(None, &["b".into(), "a".into()]);
    ed.add_targets(Some("y"), &["d".into()]);
    ed.save().unwrap();
    assert!(fs::read_to_string(&path).unwrap().contains("# ours"));
    let config = Config::load(&path).unwrap();
    assert_eq!(config.settings.targets.unwrap(), ["a", "b"]);
    assert_eq!(config.tool["y"].targets.as_deref().unwrap(), ["c", "d"]);
}

#[test]
fn template_with_targets() {
    assert_eq!(template_with(None), TEMPLATE);
    let text = template_with(Some(&["a-b-c".to_string(), "d-e-f".to_string()]));
    assert!(text.contains("# Target triples tools are reviewed"));
    assert!(text.starts_with("# Tools managed by"));
    let config: Config = toml::from_str(&text).unwrap();
    assert_eq!(config.settings.targets.unwrap(), ["a-b-c", "d-e-f"]);
}

#[test]
fn tool_names_are_crate_names() {
    for good in ["cargo-nextest", "x", "_x", "a_b-1"] {
        validate_tool_name(good).unwrap();
    }
    for bad in ["", "-x", "../x", "a/b", "a\\b", ".", "..", "a b", "é"] {
        assert!(validate_tool_name(bad).is_err(), "{bad}");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project-tools.toml");
    fs::write(&path, "[tool.\"../x\"]\nversion = \"1.0.0\"\n").unwrap();
    let err = format!("{:#}", Config::load(&path).unwrap_err());
    assert!(err.contains("not a valid crate name"), "{err}");
}
