use super::*;

/// Each highlighted piece of `text` as (row, text, style).
fn pieces(name: &str, text: &str) -> Vec<(usize, String, Style)> {
    let lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
    rows(Path::new(name), text)
        .into_iter()
        .flatten()
        .map(|h| {
            let (s, e) = (h.start(), h.end());
            let piece = lines[s.row][s.col..=e.col].iter().collect();
            (s.row, piece, h.style)
        })
        .collect()
}

fn has(pieces: &[(usize, String, Style)], text: &str, style: Style) -> bool {
    pieces.iter().any(|(_, p, s)| p == text && *s == style)
}

#[test]
fn rust_tokens() {
    let p = pieces(
        "build.rs",
        "#[cfg(x)]\nfn main() { let s: String = \"a\\\"b\"; println!(\"{}\", 0x1f); } // done\n",
    );
    assert!(has(&p, "#[cfg(x)]", ATTRIBUTE), "{p:?}");
    assert!(has(&p, "fn", KEYWORD));
    assert!(has(&p, "let", KEYWORD));
    assert!(has(&p, "String", TYPE));
    assert!(has(&p, "println!", MACRO));
    assert!(has(&p, "0x1f", NUMBER));
    assert!(has(&p, "// done", COMMENT));
    // The escaped quote doesn't end the string.
    assert!(has(&p, "a\\\"b\"", STRING), "{p:?}");
}

#[test]
fn rust_state_carries_across_lines() {
    let p = pieces(
        "build.rs",
        "/* outer /* inner */\nstill */ fn\nlet s = r#\"raw \"quote\"\nend\"#; 'a' 'static\n",
    );
    assert!(has(&p, "still */", COMMENT), "{p:?}");
    assert!(has(&p, "fn", KEYWORD));
    assert!(has(&p, "end\"#", STRING), "{p:?}");
    assert!(has(&p, "'a'", STRING));
    assert!(has(&p, "'static", KEYWORD));
}

#[test]
fn toml_tokens() {
    let p = pieces(
        "config.toml",
        "[target.x86_64]\nrustflags = [\"-C\", 'lit'] # why\njobs = 4\noffline = true\ns = \"\"\"\nmore\n\"\"\"\n",
    );
    assert!(has(&p, "[target.x86_64]", HEADER), "{p:?}");
    assert!(has(&p, "rustflags", KEY));
    assert!(has(&p, "\"-C\"", STRING));
    assert!(has(&p, "'lit'", STRING));
    assert!(has(&p, "# why", COMMENT));
    assert!(has(&p, "4", NUMBER));
    assert!(has(&p, "true", KEYWORD));
    assert!(has(&p, "more", STRING), "{p:?}");
}

#[test]
fn old_config_name_is_toml_and_others_are_plain() {
    assert!(
        !rows(Path::new(".cargo/config"), "[build]\n")
            .concat()
            .is_empty()
    );
    assert!(rows(Path::new("data.bin"), "fn x\n").is_empty());
}
