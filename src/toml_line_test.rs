use super::*;

const IGNORES: &str = r#"# Advisories to ignore

[[ignore]]
id = "RUSTSEC-2020-0001"
reason = "a"

# the second one
[[ignore]]
id = "RUSTSEC-2020-0002"
reason = "b"
"#;

#[test]
fn tables_have_their_header_lines() {
    let found = tables(IGNORES, "ignore");
    let lines: Vec<usize> = found.iter().map(|(l, _)| *l).collect();
    assert_eq!(lines, [3, 8]);
    assert_eq!(found[1].1["id"].as_str(), Some("RUSTSEC-2020-0002"));
}

#[test]
fn crlf_counts_lines_the_same() {
    let crlf = IGNORES.replace('\n', "\r\n");
    let lines: Vec<usize> = tables(&crlf, "ignore").iter().map(|(l, _)| *l).collect();
    assert_eq!(lines, [3, 8]);
}

#[test]
fn other_arrays_and_bad_files_have_none() {
    assert!(tables(IGNORES, "package").is_empty());
    assert!(tables("[[ignore]\nid = ", "ignore").is_empty());
    assert_eq!(key_line("[settings\n", &["settings"], "targets"), None);
}

#[test]
fn key_line_finds_keys_in_nested_tables() {
    let config = r#"[settings]
targets = ["aarch64-apple-darwin"]

[tool.cargo-deny]
version = "0.18.0"
targets = [
  "x86_64-unknown-linux-gnu",
]
"#;
    assert_eq!(key_line(config, &["settings"], "targets"), Some(2));
    assert_eq!(
        key_line(config, &["tool", "cargo-deny"], "targets"),
        Some(6)
    );
    assert_eq!(key_line(config, &["tool", "other"], "targets"), None);
}

#[test]
fn key_line_finds_inline_tables() {
    let config = "[tool]\ncargo-deny = { version = \"1\", targets = [\"x\"] }\n";
    assert_eq!(
        key_line(config, &["tool", "cargo-deny"], "targets"),
        Some(2)
    );
}
