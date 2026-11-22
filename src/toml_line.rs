//! Where things are in a TOML file, as 1-based lines for records. A file that doesn't parse
//! has no lines: records leave `line` out.

use toml_edit::{Document, Table, TableLike};

/// The 1-based line `offset` is on.
fn line_at(text: &str, offset: usize) -> usize {
    1 + text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}

/// Every `[[array]]` table in `text`, in order, with the line of its header.
pub fn tables(text: &str, array: &str) -> Vec<(usize, Table)> {
    let Ok(doc) = Document::parse(text) else {
        return Vec::new();
    };
    let Some(list) = doc
        .as_table()
        .get(array)
        .and_then(|i| i.as_array_of_tables())
    else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|t| Some((line_at(text, t.span()?.start), t.clone())))
        .collect()
}

/// The line of `key` in the table (or inline table) at `path`.
pub fn key_line(text: &str, path: &[&str], key: &str) -> Option<usize> {
    let doc = Document::parse(text).ok()?;
    let mut table: &dyn TableLike = doc.as_table();
    for name in path {
        table = table.get(name)?.as_table_like()?;
    }
    Some(line_at(text, table.key(key)?.span()?.start))
}

#[cfg(test)]
#[path = "toml_line_test.rs"]
mod tests;
