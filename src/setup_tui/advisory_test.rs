use super::*;

fn plain(lines: &[Line]) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

#[test]
fn markdown_is_read_lightly() {
    let lines = markdown(
        "## Impact\n\nSee [the issue](https://example.com/1) and `code()`.\n\n```rust\nlet x = [a](b);\n```\n",
    );
    assert_eq!(
        plain(&lines),
        [
            "Impact",
            "",
            "See the issue <https://example.com/1> and code().",
            "",
            "let x = [a](b);",
        ]
    );
    assert!(
        lines[0]
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD)
    );
}

#[test]
fn advisory_text_is_made_safe() {
    // An escape that would clear the screen, and a bidi override that reorders text.
    let lines = markdown("bad \x1b[2J here\n# \u{202e}title\n");
    assert_eq!(plain(&lines), ["bad \\x1b[2J here", "<U+202E>title"]);
}

#[test]
fn an_advisory_lists_its_fields() {
    let advisory: rustsec::Advisory = "```toml
[advisory]
id = \"RUSTSEC-2024-0001\"
package = \"dep\"
date = \"2024-01-01\"
aliases = [\"CVE-2024-1\"]
url = \"https://example.com/a\"

[versions]
patched = [\">= 1.1.0\"]
```

# Dep is broken

Description.
"
    .parse()
    .unwrap();
    let text = plain(&lines(&advisory)).join("\n");
    for want in [
        "Dep is broken",
        "RUSTSEC-2024-0001",
        "CVE-2024-1",
        ">=1.1.0",
        "https://example.com/a",
        "Description.",
    ] {
        assert!(text.contains(want), "{want} missing in:\n{text}");
    }
}
