//! Syntax colours for the review screen's editor: a small lexer for Rust and TOML that
//! knows comments, strings, keywords, numbers, attributes and macros. Good enough to read
//! by; it isn't a parser.

use std::path::Path;

use edtui::{Highlight, Index2};
use ratatui::style::{Color, Modifier, Style};

const COMMENT: Style = Style::new().fg(Color::DarkGray);
const STRING: Style = Style::new().fg(Color::Green);
const KEYWORD: Style = Style::new().fg(Color::Magenta);
const NUMBER: Style = Style::new().fg(Color::Yellow);
const ATTRIBUTE: Style = Style::new().fg(Color::Cyan);
const MACRO: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const TYPE: Style = Style::new().fg(Color::LightBlue);
const KEY: Style = Style::new().fg(Color::Blue);
const HEADER: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

/// What carries over from one line to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Code,
    /// Inside a (nested) block comment.
    Block(u32),
    /// Inside a Rust string: `None` for a plain one, the number of `#`s for a raw one.
    Str(Option<usize>),
    /// Inside a TOML multi-line string with this quote.
    Multi(char),
}

/// Highlights per row of `text`, by the file's kind. Empty for kinds we don't know.
pub fn rows(path: &Path, text: &str) -> Vec<Vec<Highlight>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let line_fn: fn(&[char], &mut State, &mut Row) = match ext {
        "rs" => rust_line,
        "toml" => toml_line,
        // Cargo's old config name.
        _ if name == "config" => toml_line,
        _ => return Vec::new(),
    };
    let mut state = State::Code;
    text.split('\n')
        .enumerate()
        .map(|(row, line)| {
            let chars: Vec<char> = line.chars().collect();
            let mut out = Row {
                row,
                spans: Vec::new(),
            };
            line_fn(&chars, &mut state, &mut out);
            out.spans
        })
        .collect()
}

struct Row {
    row: usize,
    spans: Vec<Highlight>,
}

impl Row {
    /// Style columns `start..end`.
    fn push(&mut self, start: usize, end: usize, style: Style) {
        if end > start {
            self.spans.push(Highlight::new(
                Index2::new(self.row, start),
                Index2::new(self.row, end - 1),
                style,
            ));
        }
    }
}

fn at(line: &[char], i: usize, s: &str) -> bool {
    s.chars()
        .enumerate()
        .all(|(k, c)| line.get(i + k) == Some(&c))
}

fn word_end(line: &[char], mut i: usize) -> usize {
    while i < line.len() && (line[i].is_alphanumeric() || line[i] == '_') {
        i += 1;
    }
    i
}

/// Where a Rust string that's open at `i` ends (just past its closing quote), if on this line.
fn string_end(line: &[char], mut i: usize, hashes: Option<usize>) -> Option<usize> {
    while i < line.len() {
        match (line[i], hashes) {
            ('\\', None) => i += 2,
            ('"', None) => return Some(i + 1),
            ('"', Some(h)) if (1..=h).all(|k| line.get(i + k) == Some(&'#')) => {
                return Some(i + 1 + h);
            }
            _ => i += 1,
        }
    }
    None
}

fn rust_line(line: &[char], state: &mut State, out: &mut Row) {
    let n = line.len();
    let mut i = 0;
    while i < n {
        match *state {
            State::Block(mut depth) => {
                let start = i;
                while i < n && depth > 0 {
                    if at(line, i, "*/") {
                        depth -= 1;
                        i += 2;
                    } else if at(line, i, "/*") {
                        depth += 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                out.push(start, i.min(n), COMMENT);
                *state = if depth == 0 {
                    State::Code
                } else {
                    State::Block(depth)
                };
                continue;
            }
            State::Str(hashes) => {
                let end = string_end(line, i, hashes);
                out.push(i, end.unwrap_or(n).min(n), STRING);
                if let Some(e) = end {
                    *state = State::Code;
                    i = e;
                } else {
                    i = n;
                }
                continue;
            }
            State::Code | State::Multi(_) => {}
        }
        let c = line[i];
        if at(line, i, "//") {
            out.push(i, n, COMMENT);
            return;
        }
        if at(line, i, "/*") {
            out.push(i, i + 2, COMMENT);
            *state = State::Block(1);
            i += 2;
        } else if c == '"' {
            out.push(i, i + 1, STRING);
            *state = State::Str(None);
            i += 1;
        } else if c == '\'' {
            // A char literal, or else a lifetime.
            let close = if line.get(i + 1) == Some(&'\\') {
                (i + 3..n.min(i + 12)).find(|&k| line[k] == '\'')
            } else {
                Some(i + 2).filter(|&k| line.get(k) == Some(&'\''))
            };
            match close {
                Some(k) => {
                    out.push(i, k + 1, STRING);
                    i = k + 1;
                }
                None => {
                    let end = word_end(line, i + 1);
                    out.push(i, end, KEYWORD);
                    i = end.max(i + 1);
                }
            }
        } else if c == '#' && matches!(line.get(i + 1), Some('[') | Some('!')) {
            let mut depth = 0;
            let mut end = n;
            for (k, ch) in line.iter().enumerate().skip(i) {
                match ch {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            end = k + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            out.push(i, end, ATTRIBUTE);
            i = end;
        } else if c.is_ascii_digit() {
            let mut end = i;
            while end < n
                && (line[end].is_alphanumeric()
                    || line[end] == '_'
                    || (line[end] == '.' && line.get(end + 1) != Some(&'.')))
            {
                end += 1;
            }
            out.push(i, end, NUMBER);
            i = end;
        } else if c.is_alphabetic() || c == '_' {
            let end = word_end(line, i);
            let word: String = line[i..end].iter().collect();
            let hashes = line[end..].iter().take_while(|&&ch| ch == '#').count();
            if matches!(word.as_str(), "r" | "br") && line.get(end + hashes) == Some(&'"') {
                let quote = end + hashes;
                out.push(i, quote + 1, STRING);
                *state = State::Str(Some(hashes));
                i = quote + 1;
            } else if word == "b" && matches!(line.get(end), Some('"') | Some('\'')) {
                out.push(i, end, STRING);
                i = end;
            } else {
                if line.get(end) == Some(&'!') && line.get(end + 1) != Some(&'=') {
                    out.push(i, end + 1, MACRO);
                } else if RUST_KEYWORDS.contains(&word.as_str()) {
                    out.push(i, end, KEYWORD);
                } else if c.is_uppercase() {
                    out.push(i, end, TYPE);
                }
                i = end;
            }
        } else {
            i += 1;
        }
    }
}

fn toml_line(line: &[char], state: &mut State, out: &mut Row) {
    let n = line.len();
    let mut i = 0;
    if let State::Multi(q) = *state {
        let triple: String = [q; 3].iter().collect();
        match (0..n).find(|&k| at(line, k, &triple)) {
            Some(k) => {
                out.push(0, k + 3, STRING);
                *state = State::Code;
                i = k + 3;
            }
            None => {
                out.push(0, n, STRING);
                return;
            }
        }
    } else {
        let start = line.iter().take_while(|c| c.is_whitespace()).count();
        if line.get(start) == Some(&'[') {
            let end = line.iter().rposition(|&c| c == ']').map_or(n, |k| k + 1);
            out.push(start, end, HEADER);
            i = end;
        } else {
            let mut end = start;
            while end < n
                && (line[end].is_alphanumeric()
                    || matches!(line[end], '_' | '-' | '.' | '"' | '\''))
            {
                end += 1;
            }
            let rest = line[end..].iter().position(|c| !c.is_whitespace());
            if end > start && rest.is_some_and(|k| line[end + k] == '=') {
                out.push(start, end, KEY);
                i = end;
            }
        }
    }
    while i < n {
        let c = line[i];
        if c == '#' {
            out.push(i, n, COMMENT);
            return;
        }
        if c == '"' || c == '\'' {
            let triple: String = [c; 3].iter().collect();
            if at(line, i, &triple) {
                match (i + 3..n).find(|&k| at(line, k, &triple)) {
                    Some(k) => {
                        out.push(i, k + 3, STRING);
                        i = k + 3;
                    }
                    None => {
                        out.push(i, n, STRING);
                        *state = State::Multi(c);
                        return;
                    }
                }
                continue;
            }
            let mut k = i + 1;
            while k < n && line[k] != c {
                k += if c == '"' && line[k] == '\\' { 2 } else { 1 };
            }
            let end = (k + 1).min(n);
            out.push(i, end, STRING);
            i = end;
        } else if c.is_ascii_digit()
            || (matches!(c, '-' | '+') && line.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let mut end = i + 1;
            while end < n
                && (line[end].is_alphanumeric() || matches!(line[end], '_' | '.' | ':' | '-' | '+'))
            {
                end += 1;
            }
            out.push(i, end, NUMBER);
            i = end;
        } else if c.is_alphabetic() {
            let end = word_end(line, i);
            let word: String = line[i..end].iter().collect();
            if word == "true" || word == "false" {
                out.push(i, end, KEYWORD);
            }
            i = end;
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
#[path = "highlight_test.rs"]
mod tests;
