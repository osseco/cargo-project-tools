use super::*;

const SNIPPET: &str = r#"[source.crates-io]
replace-with = "vendored-sources"

[source."git+https://github.com/example/dep?rev=abc"]
git = "https://github.com/example/dep"
rev = "abc"
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "/p/.project-tools/targets/tool/vendor"
"#;

#[test]
fn source_replacement_is_kept_as_is() {
    assert_eq!(
        source_config(SNIPPET, true).unwrap().as_deref(),
        Some(SNIPPET)
    );
}

#[test]
fn nothing_to_vendor() {
    assert!(source_config("\n", false).unwrap().is_none());
}

#[test]
fn missing_config_with_dependencies_fails() {
    assert!(source_config("", true).is_err());
}

#[test]
fn anything_but_source_replacement_is_refused() {
    let extra = format!("{SNIPPET}\n[build]\nrustc-wrapper = \"x\"\n");
    assert!(source_config(&extra, true).is_err());
    assert!(source_config("[source.crates-io]\nreplace-with = \"x\"\n", true).is_err());
    assert!(source_config("not toml [", true).is_err());
}
