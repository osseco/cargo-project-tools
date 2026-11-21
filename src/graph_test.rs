use super::*;

fn pkg(name: &str, version: &str) -> (String, Version) {
    (name.to_string(), Version::parse(version).unwrap())
}

#[test]
fn tree_lines_with_sources_and_markers() {
    let out = "\
mytool v1.0.0 (/tmp/src)
serde v1.0.200
serde_derive v1.0.200 (proc-macro)
gitdep v0.1.0 (https://github.com/example/gitdep?rev=abc#abcdef01)
serde v1.0.200 (*)

";
    let built = parse_tree(out, "mytool").unwrap();
    assert_eq!(
        built,
        Built::from([
            pkg("mytool", "1.0.0"),
            pkg("serde", "1.0.200"),
            pkg("serde_derive", "1.0.200"),
            pkg("gitdep", "0.1.0"),
        ])
    );
}

#[test]
fn prerelease_versions_parse() {
    let built = parse_tree("mytool v1.0.0-rc.1\n", "mytool").unwrap();
    assert!(built.contains(&pkg("mytool", "1.0.0-rc.1")));
}

#[test]
fn unexpected_output_fails() {
    assert!(parse_tree("mytool\n", "mytool").is_err());
    assert!(parse_tree("mytool 1.0.0\n", "mytool").is_err());
    assert!(parse_tree("mytool vX\n", "mytool").is_err());
}

#[test]
fn every_target_or_the_chosen_ones() {
    assert_eq!(target_args(None), ["--target", "all"]);
    let chosen = ["a".to_string(), "b".to_string()];
    assert_eq!(
        target_args(Some(&chosen)),
        ["--target", "a", "--target", "b"]
    );
}

#[test]
fn missing_root_fails() {
    assert!(parse_tree("", "mytool").is_err());
    assert!(parse_tree("serde v1.0.0\n", "mytool").is_err());
}

fn tool_at(version: &str, rev: Option<&str>) -> Tool {
    let mut text = format!("version = \"{version}\"\n");
    if let Some(r) = rev {
        text.push_str(&format!("git = \"https://example.com/x\"\nrev = \"{r}\"\n"));
    }
    toml::from_str(&text).unwrap()
}

#[test]
fn cache_key_follows_what_the_result_depends_on() {
    let none: [String; 0] = [];
    let features = ["--no-default-features".to_string()];
    let mac = ["aarch64-apple-darwin".to_string()];
    let sel = |features, targets| Selection {
        features,
        targets,
        config: &[],
    };
    let base = key("t", &tool_at("1.0.0", None), "lock", sel(&none, None));
    assert_eq!(
        base,
        key("t", &tool_at("1.0.0", None), "lock", sel(&none, None))
    );
    for other in [
        key("t", &tool_at("1.0.0", None), "lock2", sel(&none, None)),
        key("t", &tool_at("1.0.1", None), "lock", sel(&none, None)),
        key(
            "t",
            &tool_at("1.0.0", Some("abc")),
            "lock",
            sel(&none, None),
        ),
        key("t", &tool_at("1.0.0", None), "lock", sel(&features, None)),
        key("t", &tool_at("1.0.0", None), "lock", sel(&none, Some(&mac))),
        key("u", &tool_at("1.0.0", None), "lock", sel(&none, None)),
    ] {
        assert_ne!(base, other);
    }
}

#[test]
fn cached_result_is_used_only_when_it_fits() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("stamps/t.built.json");
    assert!(read_cached(&path, "k", "t").is_none());

    let built = Built::from([pkg("t", "1.0.0"), pkg("dep", "0.2.1-rc.1")]);
    write_cached(&path, "k", &built).unwrap();
    assert_eq!(read_cached(&path, "k", "t"), Some(built));
    assert!(read_cached(&path, "other", "t").is_none());
    assert!(read_cached(&path, "k", "not-the-root").is_none());

    fs::write(&path, "{\"key\":\"k\",\"packages\":[[\"t\",\"bad\"]]}").unwrap();
    assert!(read_cached(&path, "k", "t").is_none());
    fs::write(&path, "garbage").unwrap();
    assert!(read_cached(&path, "k", "t").is_none());
}
