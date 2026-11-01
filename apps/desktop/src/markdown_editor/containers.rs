//! Bounded AST container identity and lexical marker handling. Container ranges
//! keep nested list operations scoped to the selected item and its descendants.
use super::document::Block;
use markdown::mdast::Node;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Container {
    pub raw: Range<usize>,
    pub list: bool,
    pub group: usize,
    pub marker_width: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Marker {
    Quote,
    Bullet,
    Number,
    Task,
}
#[derive(Clone, Debug)]
pub(super) struct Token {
    pub raw: Range<usize>,
    pub kind: Marker,
}

pub(super) fn tokens(prefix: &str) -> Option<Vec<Token>> {
    let bytes = prefix.as_bytes();
    let mut at = 0;
    let mut result = Vec::new();
    while at < bytes.len() {
        if matches!(bytes[at], b' ' | b'\t') {
            at += 1;
            continue;
        }
        let start = at;
        if bytes[at] == b'>' {
            at += 1;
            if matches!(bytes.get(at), Some(b' ' | b'\t')) {
                at += 1;
            }
            result.push(Token {
                raw: start..at,
                kind: Marker::Quote,
            });
            continue;
        }
        let mut kind = if matches!(bytes[at], b'-' | b'+' | b'*') {
            at += 1;
            Marker::Bullet
        } else {
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            if at == start || !matches!(bytes.get(at), Some(b'.' | b')')) {
                return None;
            }
            at += 1;
            Marker::Number
        };
        if !matches!(bytes.get(at), Some(b' ' | b'\t')) {
            return None;
        }
        while matches!(bytes.get(at), Some(b' ' | b'\t')) {
            at += 1;
        }
        if bytes.get(at) == Some(&b'[')
            && matches!(bytes.get(at + 1), Some(b' ' | b'x' | b'X'))
            && bytes.get(at + 2) == Some(&b']')
            && matches!(bytes.get(at + 3), Some(b' ' | b'\t'))
        {
            at += 3;
            while matches!(bytes.get(at), Some(b' ' | b'\t')) {
                at += 1;
            }
            kind = Marker::Task;
        }
        result.push(Token {
            raw: start..at,
            kind,
        });
    }
    Some(result)
}

pub(super) fn continuation(prefix: &str, keep_last_list: bool) -> Option<String> {
    let tokens = tokens(prefix)?;
    let last = tokens.iter().rposition(|t| t.kind != Marker::Quote);
    let mut result = prefix.to_string();
    for (i, token) in tokens.iter().enumerate().rev() {
        if token.kind != Marker::Quote && !(keep_last_list && last == Some(i)) {
            let marker = &prefix[token.raw.clone()];
            let width = if token.kind == Marker::Task {
                marker.find('[').unwrap_or(marker.len())
            } else {
                marker.len()
            };
            result.replace_range(token.raw.clone(), &" ".repeat(width));
        }
    }
    Some(result)
}

pub(super) fn annotate(ast: &Node, source: &str, blocks: &mut [Block]) {
    fn walk(
        node: &Node,
        source: &str,
        blocks: &mut [Block],
        stack: &mut Vec<Container>,
        group: usize,
        depth: usize,
        budget: &mut usize,
    ) {
        if depth > 48 || *budget == 0 {
            return;
        }
        *budget -= 1;
        let group = if let Node::List(list) = node {
            list.position.as_ref().map_or(group, |p| p.start.offset)
        } else {
            group
        };
        let container = matches!(node, Node::ListItem(_) | Node::Blockquote(_));
        if container && let Some(p) = node.position() {
            let raw = p.start.offset..p.end.offset;
            let first = source
                .get(raw.clone())
                .unwrap_or("")
                .split('\n')
                .next()
                .unwrap_or("");
            let width = if matches!(node, Node::Blockquote(_)) {
                2
            } else {
                let start = if first.starts_with(['-', '+', '*']) {
                    1
                } else {
                    first.bytes().take_while(u8::is_ascii_digit).count() + 1
                };
                start
                    + first
                        .get(start..)
                        .unwrap_or("")
                        .bytes()
                        .take_while(|b| matches!(b, b' ' | b'\t'))
                        .count()
            };
            stack.push(Container {
                raw: raw.clone(),
                list: matches!(node, Node::ListItem(_)),
                group,
                marker_width: width.max(2),
            });
            for block in blocks.iter_mut().filter(|b| {
                b.raw.start >= raw.start
                    && (b.raw.end <= raw.end
                        || b.raw.is_empty()
                            && raw.end <= b.raw.start
                            && source
                                .get(raw.end..b.raw.start)
                                .is_some_and(|s| s.chars().all(|c| matches!(c, ' ' | '\t'))))
            }) {
                block.containers = stack.clone();
            }
        }
        if let Some(children) = node.children() {
            for child in children {
                walk(child, source, blocks, stack, group, depth + 1, budget);
            }
        }
        if container && node.position().is_some() {
            stack.pop();
        }
    }
    walk(ast, source, blocks, &mut Vec::new(), 0, 0, &mut 10_000);
}

pub(super) fn inherit(source: &str, block: &mut Block) {
    let mut options = markdown::ParseOptions::gfm();
    options.constructs.frontmatter = true;
    if let Ok(ast) = markdown::to_mdast(source, &options) {
        annotate(&ast, source, std::slice::from_mut(block));
    }
}

pub(super) fn indent(
    doc: &super::document::Document,
    index: usize,
    outdent: bool,
) -> Result<Option<super::structure::Edit>, String> {
    let block = doc.blocks.get(index).ok_or("The selected block changed")?;
    let Some(container) = block.containers.last() else {
        return Ok(None);
    };
    let source = &doc.source;
    let start = source[..container.raw.start]
        .rfind('\n')
        .map_or(0, |i| i + 1);
    let end = container.raw.end;
    let first_prefix = &source[start..block.raw.start];
    let all = tokens(first_prefix).ok_or("Unsupported container prefix")?;
    let own = all.last().ok_or("Missing container marker")?;
    let column = container.raw.start - start;
    let mut replacement = String::new();
    if container.list {
        let parent = block.containers.iter().rev().filter(|c| c.list).nth(1);
        let width = if outdent {
            parent.map_or(container.marker_width, |c| c.marker_width)
        } else {
            let previous = doc.blocks[..index]
                .iter()
                .rev()
                .flat_map(|b| b.containers.iter().rev())
                .find(|c| c.list && c.group == container.group && c.raw.end <= container.raw.start)
                .ok_or("A list item needs a preceding sibling before it can be indented")?;
            previous.marker_width
        };
        for (i, line) in source[start..end].split_inclusive('\n').enumerate() {
            if outdent && parent.is_none() {
                if i == 0 {
                    replacement.push_str(&line[..own.raw.start]);
                    replacement.push_str(&line[own.raw.end..]);
                } else {
                    let remove = line
                        .get(column..)
                        .unwrap_or("")
                        .bytes()
                        .take_while(|b| *b == b' ')
                        .count()
                        .min(width);
                    replacement.push_str(&line[..column.min(line.len())]);
                    replacement.push_str(&line[column.min(line.len()) + remove..]);
                }
            } else if outdent {
                let at = column
                    .checked_sub(width)
                    .ok_or("Invalid nested indentation")?;
                if line.trim().is_empty() {
                    replacement.push_str(line);
                    continue;
                }
                if line.get(at..column) != Some(&" ".repeat(width)) {
                    return Err(
                        "This subtree uses unsupported indentation. The source is unchanged."
                            .into(),
                    );
                }
                replacement.push_str(&line[..at]);
                replacement.push_str(&line[column..]);
            } else {
                let at = column.min(line.trim_end_matches(['\r', '\n']).len());
                replacement.push_str(&line[..at]);
                replacement.push_str(&" ".repeat(width));
                replacement.push_str(&line[at..]);
            }
        }
    } else {
        for line in source[start..end].split_inclusive('\n') {
            if line.trim().is_empty() {
                replacement.push_str(line);
                continue;
            }
            let at = column.min(line.len());
            if outdent {
                let tail = line.get(at..).ok_or("Invalid quote prefix")?;
                if !tail.starts_with('>') {
                    return Err("Lazy quote continuation cannot be lifted safely".into());
                }
                let count = if tail.starts_with("> ") { 2 } else { 1 };
                replacement.push_str(&line[..at]);
                replacement.push_str(&line[at + count..]);
            } else {
                replacement.push_str(&line[..at]);
                replacement.push_str("> ");
                replacement.push_str(&line[at..]);
            }
        }
    }
    let mut next = source.clone();
    next.replace_range(start..end, &replacement);
    let document = super::document::Document::parse(&next)?;
    if document.blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
        != doc.blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
    {
        return Err("Indentation would change document content. The source is unchanged.".into());
    }
    let focus = document.blocks[index].clone();
    Ok(Some(super::structure::Edit {
        range: start..end,
        replacement,
        document,
        focus,
        caret: 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::{continuation, indent};
    use crate::markdown_editor::{
        document::Document,
        structure::{Operation, edit},
    };
    #[test]
    fn mixed_prefixes_continue_only_the_active_item() {
        assert_eq!(continuation("> - [x] > 9. ", true).unwrap(), ">   > 9. ");
        let doc = Document::parse("> - [x] one two\n>   - child\n\nend").unwrap();
        let changed = edit(&doc, 0, 3..4, Operation::Split).unwrap().unwrap();
        assert!(
            changed
                .document
                .source
                .starts_with("> - [x] one\n> - [ ] two\n>   - child")
        );
        let doc = Document::parse("- parent\n\n  9. one two").unwrap();
        assert_eq!(
            edit(&doc, 1, 3..4, Operation::Split)
                .unwrap()
                .unwrap()
                .document
                .source,
            "- parent\n\n  9. one\n  10. two"
        );
    }
    #[test]
    fn list_subtrees_indent_and_outdent_atomically() {
        let source = "- first\n- second\n  - child\n- third\n\nend";
        let doc = Document::parse(source).unwrap();
        let changed = indent(&doc, 1, false).unwrap().unwrap();
        assert_eq!(
            changed.document.source,
            "- first\n  - second\n    - child\n- third\n\nend"
        );
        assert_eq!(
            indent(&changed.document, 1, true)
                .unwrap()
                .unwrap()
                .document
                .source,
            source
        );
        assert!(indent(&doc, 0, false).is_err());
    }
    #[test]
    fn empty_nested_item_keeps_its_container_and_lifts_one_level() {
        let doc = Document::parse("- parent\n  - child").unwrap();
        let split = edit(&doc, 1, 5..5, Operation::Split).unwrap().unwrap();
        assert_eq!(split.focus.containers.len(), 2);
        let index = split
            .document
            .blocks
            .iter()
            .position(|b| b.raw == split.focus.raw)
            .unwrap();
        let lifted = edit(&split.document, index, 0..0, Operation::Split)
            .unwrap()
            .unwrap();
        assert_eq!(lifted.document.source, "- parent\n  - child\n- ");
    }
    #[test]
    fn quoted_subtrees_and_top_level_lifting_preserve_text() {
        let doc = Document::parse("> one\n>\n> two\n\nend").unwrap();
        let nested = indent(&doc, 0, false).unwrap().unwrap();
        assert_eq!(
            indent(&nested.document, 0, true)
                .unwrap()
                .unwrap()
                .document
                .source,
            "> one\n> \n> two\n\nend"
        );
        let doc = Document::parse("- one\n  continuation\n\nend").unwrap();
        assert_eq!(
            indent(&doc, 0, true).unwrap().unwrap().document.source,
            "one\ncontinuation\n\nend"
        );
    }
}
