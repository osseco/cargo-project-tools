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
fn missing_root_fails() {
    assert!(parse_tree("", "mytool").is_err());
    assert!(parse_tree("serde v1.0.0\n", "mytool").is_err());
}
