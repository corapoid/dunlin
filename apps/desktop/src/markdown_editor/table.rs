//! Bounded, source-local table edits. Untouched cell spelling, escapes, container
//! prefixes and line endings survive; no whole-document AST serialization.
use markdown::{
    ParseOptions,
    mdast::{AlignKind, Node},
};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Operation {
    RowBefore,
    RowAfter,
    DeleteRow,
    ColumnBefore,
    ColumnAfter,
    DeleteColumn,
    Align(AlignKind),
}

pub(super) struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    pub row: usize,
    pub column: usize,
}

struct Line {
    prefix: String,
    body: String,
    ending: String,
}

fn position(node: &Node) -> Result<Range<usize>, &'static str> {
    let p = node.position().ok_or("Missing table source position")?;
    Ok(p.start.offset..p.end.offset)
}

fn find_table<'a>(
    node: &'a Node,
    at: usize,
    depth: usize,
    budget: &mut usize,
) -> Option<&'a markdown::mdast::Table> {
    if depth > 48 || *budget == 0 {
        return None;
    }
    *budget -= 1;
    if let Node::Table(table) = node
        && table.position.as_ref()?.start.offset == at
    {
        return Some(table);
    }
    node.children()?
        .iter()
        .find_map(|child| find_table(child, at, depth + 1, budget))
}

/// Pipe separators obey GFM escaping, including backslash parity. Pipes inside
/// code spans still need escaping in GFM tables; entities are not separators.
fn cells(body: &str) -> Vec<Range<usize>> {
    let mut pipes = Vec::new();
    let mut slashes = 0;
    for (i, b) in body.bytes().enumerate() {
        if b == b'|' && slashes % 2 == 0 {
            pipes.push(i);
        }
        slashes = if b == b'\\' { slashes + 1 } else { 0 };
    }
    let mut parts = Vec::new();
    let mut start = 0;
    for pipe in pipes {
        parts.push(start..pipe);
        start = pipe + 1;
    }
    parts.push(start..body.len());
    if parts.len() > 1 && body[parts[0].clone()].trim().is_empty() {
        parts.remove(0);
    }
    if parts.len() > 1 && body[parts.last().unwrap().clone()].trim().is_empty() {
        parts.pop();
    }
    parts
}

fn insert_column(body: &mut String, column: usize, value: &str) -> Result<(), &'static str> {
    let spans = cells(body);
    if column > spans.len() {
        return Err("This sparse table row needs a cell before this column.");
    }
    if column < spans.len() {
        let opening = if column == 0 && spans[0].start == 0 {
            "|"
        } else {
            ""
        };
        body.insert_str(spans[column].start, &format!("{opening} {value} |"));
    } else {
        let at = spans.last().ok_or("Empty table row")?.end;
        let closing = if at == body.len() { "|" } else { "" };
        body.insert_str(at, &format!("| {value} {closing}"));
    }
    Ok(())
}

fn delete_column(body: &mut String, column: usize) -> Result<(), &'static str> {
    let spans = cells(body);
    if spans.len() <= 1 || column >= spans.len() {
        return Err("The table needs at least one column.");
    }
    let range = if column + 1 < spans.len() {
        spans[column].start..spans[column + 1].start
    } else {
        spans[column - 1].end..spans[column].end
    };
    body.replace_range(range, "");
    Ok(())
}

pub(super) fn edit(
    source: &str,
    table_at: usize,
    row: usize,
    column: usize,
    operation: Operation,
) -> Result<Edit, &'static str> {
    if !super::preview_allowed(source) {
        return Err("This document exceeds visual editing limits.");
    }
    let mut options = ParseOptions::gfm();
    options.constructs.frontmatter = true;
    let ast = markdown::to_mdast(source, &options).map_err(|_| "Could not parse the table")?;
    let table = find_table(&ast, table_at, 0, &mut 10_000)
        .ok_or("The table changed. Select a cell again.")?;
    let columns = table.align.len();
    if row >= table.children.len() || column >= columns {
        return Err("The selected table cell changed.");
    }
    let table_range = table
        .position
        .as_ref()
        .ok_or("Missing table source position")?;
    let start = source[..table_range.start.offset]
        .rfind('\n')
        .map_or(0, |n| n + 1);
    let end = source[table_range.end.offset..]
        .find('\n')
        .map_or(source.len(), |n| table_range.end.offset + n + 1);
    let mut lines = Vec::new();
    let mut at = start;
    for (index, raw) in source[start..end].split_inclusive('\n').enumerate() {
        let text = raw.trim_end_matches(['\r', '\n']);
        let prefix_len = if index == 1 {
            text.bytes()
                .take_while(|b| matches!(b, b' ' | b'\t' | b'>'))
                .count()
        } else {
            let node = table
                .children
                .get(if index == 0 { 0 } else { index - 1 })
                .ok_or("Unexpected multiline table")?;
            position(node)?
                .start
                .checked_sub(at)
                .ok_or("Invalid table source position")?
        };
        if !text.is_char_boundary(prefix_len) || prefix_len > text.len() {
            return Err("Invalid table prefix");
        }
        lines.push(Line {
            prefix: text[..prefix_len].into(),
            body: text[prefix_len..].into(),
            ending: raw[text.len()..].into(),
        });
        at += raw.len();
    }
    if lines.len() != table.children.len() + 1 || lines.len() < 2 {
        return Err("Unexpected table layout");
    }
    // Do not silently expose or discard extra cells that GFM currently ignores.
    if lines.iter().any(|line| cells(&line.body).len() > columns) {
        return Err(
            "This table has extra hidden cells. Make its rows rectangular before structural edits.",
        );
    }
    let newline = lines
        .iter()
        .find(|l| !l.ending.is_empty())
        .map(|l| l.ending.clone())
        .unwrap_or_else(|| "\n".into());
    let final_ending = lines.last().unwrap().ending.clone();
    let mut focus_row = row;
    let mut focus_column = column;
    match operation {
        Operation::RowBefore | Operation::RowAfter => {
            let insert = if row == 0 {
                2
            } else {
                row + 1 + usize::from(operation == Operation::RowAfter)
            };
            let new_row = Line {
                prefix: lines[1].prefix.clone(),
                body: format!("|{}", "  |".repeat(columns)),
                ending: newline.clone(),
            };
            if insert == lines.len() {
                lines.last_mut().unwrap().ending = newline.clone();
            }
            lines.insert(insert, new_row);
            focus_row = insert - 1;
        }
        Operation::DeleteRow => {
            if row == 0 {
                return Err("The header row cannot be deleted.");
            }
            lines.remove(row + 1);
            focus_row = row.min(lines.len() - 2);
        }
        Operation::ColumnBefore | Operation::ColumnAfter => {
            let insert = column + usize::from(operation == Operation::ColumnAfter);
            for (i, line) in lines.iter_mut().enumerate() {
                while cells(&line.body).len() < columns {
                    let count = cells(&line.body).len();
                    insert_column(&mut line.body, count, "")?;
                }
                insert_column(&mut line.body, insert, if i == 1 { "---" } else { "" })?;
            }
            focus_column = insert;
        }
        Operation::DeleteColumn => {
            if columns <= 1 {
                return Err("The table needs at least one column.");
            }
            for line in &mut lines {
                while cells(&line.body).len() < columns {
                    let count = cells(&line.body).len();
                    insert_column(&mut line.body, count, "")?;
                }
                delete_column(&mut line.body, column)?;
            }
            focus_column = column.min(columns - 2);
        }
        Operation::Align(align) => {
            let span = cells(&lines[1].body)
                .get(column)
                .cloned()
                .ok_or("Missing delimiter cell")?;
            let original = &lines[1].body[span.clone()];
            let left = original.len() - original.trim_start().len();
            let right = original.trim_end().len();
            let dashes = original.trim().trim_matches(':');
            if dashes.is_empty() || !dashes.bytes().all(|b| b == b'-') {
                return Err("Invalid table alignment marker");
            }
            let replacement = format!(
                "{}{}{}",
                if matches!(align, AlignKind::Left | AlignKind::Center) {
                    ":"
                } else {
                    ""
                },
                dashes,
                if matches!(align, AlignKind::Right | AlignKind::Center) {
                    ":"
                } else {
                    ""
                }
            );
            lines[1]
                .body
                .replace_range(span.start + left..span.start + right, &replacement);
        }
    }
    lines.last_mut().unwrap().ending = final_ending;
    let replacement = lines
        .iter()
        .map(|l| format!("{}{}{}", l.prefix, l.body, l.ending))
        .collect::<String>();
    Ok(Edit {
        range: start..end,
        replacement,
        row: focus_row,
        column: focus_column,
    })
}

#[cfg(test)]
mod tests {
    use super::{Operation, cells, edit};
    use crate::markdown_editor::document::{Document, Kind};
    use markdown::mdast::AlignKind;

    fn apply(source: &str, row: usize, column: usize, operation: Operation) -> String {
        let table = Document::parse(source)
            .unwrap()
            .blocks
            .iter()
            .find_map(|b| {
                if let Kind::Cell { table, .. } = b.kind {
                    Some(table)
                } else {
                    None
                }
            })
            .unwrap();
        let edit = edit(source, table, row, column, operation).unwrap();
        let mut result = source.to_string();
        result.replace_range(edit.range, &edit.replacement);
        let doc = Document::parse(&result).unwrap();
        assert!(doc.blocks.iter().any(|b| matches!(b.kind, Kind::Cell { table: id, row, column, .. } if id == table && row == edit.row && column == edit.column)));
        result
    }

    #[test]
    fn rows_preserve_surroundings_escapes_and_eof_policy() {
        let source = "before\n\n| A | B |\n| :- | -: |\n| x\\|y | **z** |\n\nafter";
        let added = apply(source, 1, 0, Operation::RowAfter);
        assert_eq!(
            added,
            "before\n\n| A | B |\n| :- | -: |\n| x\\|y | **z** |\n|  |  |\n\nafter"
        );
        assert_eq!(apply(&added, 2, 0, Operation::DeleteRow), source);
        assert_eq!(
            apply("A | B\n--- | ---", 0, 0, Operation::RowBefore),
            "A | B\n--- | ---\n|  |  |"
        );
    }

    #[test]
    fn columns_preserve_untouched_cells_and_are_reversible() {
        let source = "| A | B |\n| :--- | ---: |\n| `a\\|b` | &amp; żółw |\n";
        let added = apply(source, 1, 0, Operation::ColumnAfter);
        assert_eq!(
            added,
            "| A |  | B |\n| :--- | --- | ---: |\n| `a\\|b` |  | &amp; żółw |\n"
        );
        assert_eq!(apply(&added, 1, 1, Operation::DeleteColumn), source);
        assert_eq!(
            apply("A | B\n--- | ---\nx | y", 0, 0, Operation::ColumnBefore),
            "|  |A | B\n| --- |--- | ---\n|  |x | y"
        );
    }

    #[test]
    fn alignment_changes_only_the_selected_delimiter() {
        let source = "| A | B |\n| :----- | ---: |\n| x | y |\n";
        assert_eq!(
            apply(source, 1, 0, Operation::Align(AlignKind::Center)),
            "| A | B |\n| :-----: | ---: |\n| x | y |\n"
        );
        assert_eq!(
            apply(source, 1, 1, Operation::Align(AlignKind::None)),
            "| A | B |\n| :----- | --- |\n| x | y |\n"
        );
    }

    #[test]
    fn quoted_and_list_tables_keep_continuation_prefixes_and_crlf() {
        let source = "> | A | B |\r\n> | - | - |\r\n> | x | y |\r\n";
        assert_eq!(
            apply(source, 0, 0, Operation::RowAfter),
            "> | A | B |\r\n> | - | - |\r\n> |  |  |\r\n> | x | y |\r\n"
        );
        let list = "- | A | B |\n  | - | - |\n  | x | y |\n";
        assert_eq!(
            apply(list, 1, 0, Operation::RowBefore),
            "- | A | B |\n  | - | - |\n  |  |  |\n  | x | y |\n"
        );
    }

    #[test]
    fn stale_cells_hidden_extras_and_deleting_required_structure_fail_closed() {
        let source = "| A |\n| - |\n| x |\n";
        for (table, row, column, op) in [
            (1, 1, 0, Operation::RowAfter),
            (0, 4, 0, Operation::RowAfter),
            (0, 1, 2, Operation::RowAfter),
            (0, 0, 0, Operation::DeleteRow),
            (0, 1, 0, Operation::DeleteColumn),
        ] {
            assert!(edit(source, table, row, column, op).is_err());
        }
        assert!(
            edit(
                "| A |\n| - |\n| x | hidden |\n",
                0,
                1,
                0,
                Operation::ColumnAfter
            )
            .is_err()
        );
        assert_eq!(cells("| a\\|b | c\\\\| d | ").len(), 3);
    }

    #[test]
    fn sparse_rows_and_missing_outer_pipes_gain_real_empty_cells() {
        let sparse = "| A | B |\n| --- | --- |\n| x |\n";
        let expanded = apply(sparse, 0, 1, Operation::ColumnAfter);
        let doc = Document::parse(&expanded).unwrap();
        assert_eq!(
            doc.blocks
                .iter()
                .filter(|b| matches!(b.kind, Kind::Cell { .. }))
                .count(),
            6
        );
        assert!(
            doc.blocks
                .iter()
                .all(|b| matches!(b.kind, Kind::Cell { columns: 3, .. }))
        );
        assert_eq!(
            apply("A | B\n--- | ---\nx | y", 1, 1, Operation::ColumnAfter),
            "A | B|  |\n--- | ---| --- |\nx | y|  |"
        );
    }

    #[test]
    fn deleting_the_last_body_row_keeps_the_header_and_original_eof_policy() {
        assert_eq!(
            apply(
                "| A | B |\n| --- | --- |\n| x | y |",
                1,
                1,
                Operation::DeleteRow
            ),
            "| A | B |\n| --- | --- |"
        );
        assert_eq!(
            apply(
                "| A | B |\r\n| --- | --- |\r\n| x | y |\r\n",
                1,
                1,
                Operation::DeleteRow
            ),
            "| A | B |\r\n| --- | --- |\r\n"
        );
    }
}
