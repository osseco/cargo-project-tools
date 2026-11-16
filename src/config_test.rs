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
