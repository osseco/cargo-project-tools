//! An advisory as text for its dialog. Advisory texts come from a third party: everything
//! goes through `shown`, and the markdown is only lightly read (headings, code, links).

use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use rustsec::Advisory;

use crate::sanitize::shown;

pub(super) fn lines(a: &Advisory) -> Vec<Line<'static>> {
    let m = &a.metadata;
    let dim = Style::default().dim();
    let mut out = vec![Line::raw(shown(&m.title)).bold(), Line::raw("")];
    let mut field = |name: &str, value: String| {
        if !value.is_empty() {
            out.push(Line::from(vec![
                Span::styled(format!("{name:<12}"), dim),
                Span::raw(shown(&value)),
            ]));
        }
    };
    let list = |items: Vec<String>| items.join(", ");
    field("Id", m.id.to_string());
    field("Package", m.package.to_string());
    field("Date", m.date.to_string());
    if let Some(w) = &m.withdrawn {
        field("Withdrawn", w.to_string());
    }
    if let Some(i) = &m.informational {
        field("Kind", i.to_string());
    }
    field(
        "Aliases",
        list(m.aliases.iter().map(ToString::to_string).collect()),
    );
    field(
        "Related",
        list(m.related.iter().map(ToString::to_string).collect()),
    );
    field(
        "Categories",
        list(m.categories.iter().map(ToString::to_string).collect()),
    );
    field(
        "Keywords",
        list(m.keywords.iter().map(|k| k.as_str().to_string()).collect()),
    );
    if let Some(c) = &m.cvss {
        field("CVSS", c.to_string());
    }
    field(
        "Patched",
        list(
            a.versions
                .patched()
                .iter()
                .map(ToString::to_string)
                .collect(),
        ),
    );
    field(
        "Unaffected",
        list(
            a.versions
                .unaffected()
                .iter()
                .map(ToString::to_string)
                .collect(),
        ),
    );
    if let Some(u) = &m.url {
        field("Url", u.to_string());
    }
    for (i, r) in m.references.iter().enumerate() {
        field(if i == 0 { "References" } else { "" }, r.to_string());
    }
    out.push(Line::raw(""));
    out.extend(markdown(&m.description));
    out
}

/// Headings bold, code dim, `[text](url)` as `text <url>`; the rest as it is.
pub(super) fn markdown(text: &str) -> Vec<Line<'static>> {
    let dim = Style::default().dim();
    let mut out = Vec::new();
    let mut fenced = false;
    for line in shown(text).replace('\t', "    ").lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            out.push(Line::styled(line.to_string(), dim));
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let heading = trimmed.trim_start_matches('#').trim();
            out.push(Line::raw(links(heading)).bold());
            continue;
        }
        // Backticks alternate between text and code.
        let spans: Vec<Span> = links(line)
            .split('`')
            .enumerate()
            .filter(|(_, part)| !part.is_empty())
            .map(|(i, part)| {
                if i % 2 == 1 {
                    Span::styled(part.to_string(), dim)
                } else {
                    Span::raw(part.to_string())
                }
            })
            .collect();
        out.push(Line::from(spans));
    }
    out
}

/// `[text](url)` becomes `text <url>`.
fn links(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find("](") else {
            break;
        };
        let Some(end) = after[close + 2..].find(')') else {
            break;
        };
        let text = &after[..close];
        let url = &after[close + 2..close + 2 + end];
        out.push_str(&rest[..open]);
        out.push_str(&format!("{text} <{url}>"));
        rest = &after[close + 2 + end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
#[path = "advisory_test.rs"]
mod tests;
