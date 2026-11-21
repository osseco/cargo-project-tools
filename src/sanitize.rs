//! Making untrusted text (file contents, names, cargo output) safe to draw on a terminal.

/// Whether `c` is drawn as an escape instead of itself. Code on the review screen is there
/// to be read, so anything that can hide, reorder or disguise text is shown by its code
/// point: format characters (bidi controls, zero-width characters, tag characters),
/// variation selectors and other characters that draw nothing, spaces other than U+0020,
/// line and paragraph separators, private-use characters and noncharacters. Control
/// characters are handled before this.
fn hidden(c: char) -> bool {
    let code = c as u32;
    matches!(code,
        // Format characters (general category Cf).
        0x00ad
        | 0x0600..=0x0605
        | 0x061c
        | 0x06dd
        | 0x070f
        | 0x0890..=0x0891
        | 0x08e2
        | 0x180e
        | 0x200b..=0x200f
        | 0x202a..=0x202e
        | 0x2060..=0x2064
        | 0x2066..=0x206f
        | 0xfeff
        | 0xfff9..=0xfffb
        | 0x110bd
        | 0x110cd
        | 0x13430..=0x1343f
        | 0x1bca0..=0x1bca3
        | 0x1d173..=0x1d17a
        | 0xe0001
        | 0xe0020..=0xe007f
        // Line and paragraph separators.
        | 0x2028..=0x2029
        // Spaces that look like U+0020.
        | 0x00a0
        | 0x1680
        | 0x2000..=0x200a
        | 0x202f
        | 0x205f
        | 0x3000
        // Characters that draw nothing or next to nothing.
        | 0x034f
        | 0x115f..=0x1160
        | 0x17b4..=0x17b5
        | 0x180b..=0x180d
        | 0x180f
        | 0x2800
        | 0x3164
        | 0xffa0
        // Variation selectors.
        | 0xfe00..=0xfe0f
        | 0xe0100..=0xe01ef
        // Private use.
        | 0xe000..=0xf8ff
        | 0xf0000..=0x10ffff
        // Noncharacters (those in planes 15 and 16 are private use above).
        | 0xfdd0..=0xfdef
    ) || code & 0xfffe == 0xfffe
}

/// Make untrusted text safe to draw: control characters become `\xNN`, and characters
/// that could hide or disguise text (see `hidden`) become `<U+XXXX>`. A CRLF line ending
/// becomes `\n`; a `\r` on its own is escaped, since it moves the cursor back over the line.
/// Returns the text and how many characters were replaced.
pub fn sanitize(text: &str) -> (String, usize) {
    let mut out = String::with_capacity(text.len());
    let mut replaced = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let code = c as u32;
        if c == '\r' && chars.peek() == Some(&'\n') {
            // The `\n` comes next.
        } else if c == '\n' || c == '\t' {
            out.push(c);
        } else if code < 0x20 || (0x7f..0xa0).contains(&code) {
            out.push_str(&format!("\\x{code:02x}"));
            replaced += 1;
        } else if hidden(c) {
            out.push_str(&format!("<U+{code:04X}>"));
            replaced += 1;
        } else {
            out.push(c);
        }
    }
    (out, replaced)
}

/// `text` made safe to draw.
pub fn shown(text: &str) -> String {
    sanitize(text).0
}

#[cfg(test)]
#[path = "sanitize_test.rs"]
mod tests;
