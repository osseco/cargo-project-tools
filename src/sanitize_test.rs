use super::*;

#[test]
fn control_characters_become_hex_escapes() {
    let (text, n) = sanitize("a\x1b]52;c;aGk=\x07b");
    assert_eq!(text, "a\\x1b]52;c;aGk=\\x07b");
    assert_eq!(n, 2);
    let (text, n) = sanitize("cr\r c1\u{85} del\x7f");
    assert_eq!(text, "cr\\x0d c1\\x85 del\\x7f");
    assert_eq!(n, 3);
}

#[test]
fn crlf_is_a_line_ending_and_a_lone_cr_is_not() {
    // As in generic-array 0.14.7's build.rs.
    let crlf = "fn main() {\r\n    println!(\"x\");\r\n}\r\n";
    assert_eq!(
        sanitize(crlf),
        ("fn main() {\n    println!(\"x\");\n}\n".to_string(), 0)
    );
    // A `\r` that would draw over its line, and one at the very end.
    let (text, n) = sanitize("safe();\rEVIL\r\n\r");
    assert_eq!(text, "safe();\\x0dEVIL\n\\x0d");
    assert_eq!(n, 2);
}

#[test]
fn characters_that_hide_or_disguise_text_become_code_points() {
    for (c, name) in [
        ('\u{202e}', "bidi override"),
        ('\u{2066}', "bidi isolate"),
        ('\u{200b}', "zero-width space"),
        ('\u{feff}', "byte order mark"),
        ('\u{00ad}', "soft hyphen"),
        ('\u{e0041}', "tag character"),
        ('\u{2028}', "line separator"),
        ('\u{00a0}', "no-break space"),
        ('\u{3000}', "ideographic space"),
        ('\u{3164}', "hangul filler"),
        ('\u{2800}', "braille blank"),
        ('\u{034f}', "combining grapheme joiner"),
        ('\u{fe0f}', "variation selector"),
        ('\u{e0100}', "variation selector supplement"),
        ('\u{e000}', "private use"),
        ('\u{10fffd}', "private use, plane 16"),
        ('\u{fdd0}', "noncharacter"),
        ('\u{1fffe}', "noncharacter"),
    ] {
        let (text, n) = sanitize(&format!("a{c}b"));
        assert_eq!(text, format!("a<U+{:04X}>b", c as u32), "{name}");
        assert_eq!(n, 1, "{name}");
    }
}

#[test]
fn ordinary_text_is_left_alone() {
    let plain = "fn main() {\n\tprintln!(\"åäö 日本 Ωμέγα é 🦀\");\n}\n";
    assert_eq!(sanitize(plain), (plain.to_string(), 0));
}
