//! Native prose split/merge plans, validated against a new source projection
//! before the authoritative buffer is touched. No unrelated AST serialization.
use super::{
    containers::{self, Marker},
    document::{Block, Document, Kind, protect_spaces},
};
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
pub(super) enum Operation {
    Split,
    Backward,
    Forward,
}

pub(super) struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    pub document: Document,
    pub focus: Block,
    pub caret: usize,
}

struct Prefix {
    start: usize,
    first: String,
    next: String,
    exit: String,
    family: String,
    quote: bool,
}

fn prefix(source: &str, block: &Block) -> Option<Prefix> {
    if block.is_literal() {
        return None;
    }
    let start = source
        .get(..block.raw.start)?
        .rfind('\n')
        .map_or(0, |n| n + 1);
    let outside = source.get(start..block.raw.start)?;
    let indent = outside.len() - outside.trim_start_matches([' ', '\t']).len();
    let whitespace = &outside[..indent];
    let marker = &outside[indent..];
    let tokens = containers::tokens(outside)?;
    let mut value = Prefix {
        start,
        first: outside.into(),
        next: containers::continuation(outside, true)?,
        exit: whitespace.into(),
        family: String::new(),
        quote: false,
    };
    if let Some(token) = tokens.last() {
        let is_list = matches!(
            block.kind,
            Kind::Bullet | Kind::Numbered(_) | Kind::Task(_, _)
        );
        if is_list && token.kind != Marker::Quote {
            let active = &outside[token.raw.clone()];
            let base = containers::continuation(&outside[..token.raw.start], false)?;
            value.next = format!("{base}{active}");
            if token.kind == Marker::Task {
                let check = value.next.rfind('[')? + 1;
                value.next.replace_range(check..check + 1, " ");
            } else if token.kind == Marker::Number {
                let digits = active.bytes().take_while(u8::is_ascii_digit).count();
                let number = active[..digits].parse::<u32>().ok()?.checked_add(1)?;
                value.next = format!("{base}{number}{}", &active[digits..]);
            }
            value.exit = outside[..token.raw.start].into();
            // An indented nested item lifts one level, rather than becoming an
            // indented code block. Ancestor quote markers remain untouched.
            if let Some(parent) = block.containers.iter().rev().filter(|c| c.list).nth(1) {
                let width = parent.marker_width;
                let end = value.exit.len();
                let spaces = value.exit.bytes().rev().take_while(|b| *b == b' ').count();
                value.exit.truncate(end - spaces.min(width));
                value.exit.push_str(active);
            }
            value.family = format!(
                "list:{:?}:{}",
                token.kind,
                block.containers.last().map_or(token.raw.start, |c| c.group)
            );
            value.quote = tokens.iter().any(|t| t.kind == Marker::Quote);
            return Some(value);
        }
        if matches!(block.kind, Kind::Quote) && token.kind == Marker::Quote {
            value.family = format!("quote:{outside}");
            value.quote = true;
            value.exit = outside[..token.raw.start].into();
            return Some(value);
        }
    }
    match block.kind {
        Kind::Paragraph | Kind::Heading(_) if marker.is_empty() => {
            value.family = format!("root:{whitespace}");
            value.next = whitespace.into();
            if let Kind::Heading(depth) = block.kind {
                value.first = format!("{whitespace}{} ", "#".repeat(depth as usize));
            }
        }
        Kind::Quote
            if !marker.is_empty() && outside.chars().all(|c| matches!(c, '>' | ' ' | '\t')) =>
        {
            value.family = format!("quote:{outside}");
            value.quote = true;
            value.exit = outside[..outside.rfind('>')?].into();
        }
        Kind::Bullet | Kind::Task(_, _)
            if matches!(marker.as_bytes().first(), Some(b'-' | b'+' | b'*')) =>
        {
            let after = marker.get(1..)?;
            if !after.starts_with([' ', '\t']) {
                return None;
            }
            if matches!(block.kind, Kind::Task(_, _)) {
                let check = outside.find('[')? + 1;
                if !matches!(outside.as_bytes().get(check), Some(b'x' | b'X' | b' '))
                    || outside.as_bytes().get(check + 1) != Some(&b']')
                    || !outside[check + 2..].chars().all(char::is_whitespace)
                {
                    return None;
                }
                value.next.replace_range(check..check + 1, " ");
                value.family = format!("task:{whitespace}:{}", &marker[..1]);
            } else {
                if !after.chars().all(char::is_whitespace) {
                    return None;
                }
                value.family = format!("bullet:{whitespace}:{}", &marker[..1]);
            }
        }
        Kind::Numbered(_) => {
            let digits = marker.bytes().take_while(u8::is_ascii_digit).count();
            let punctuation = *marker.as_bytes().get(digits)?;
            if !matches!(punctuation, b'.' | b')')
                || marker.get(digits + 1..)?.is_empty()
                || !marker[digits + 1..].chars().all(char::is_whitespace)
            {
                return None;
            }
            let number = marker[..digits].parse::<u32>().ok()?.checked_add(1)?;
            value.next = format!(
                "{whitespace}{number}{}{}",
                punctuation as char,
                &marker[digits + 1..]
            );
            value.family = format!("number:{whitespace}:{}", punctuation as char);
        }
        _ => return None,
    }
    Some(value)
}

fn validate(
    source: &str,
    range: Range<usize>,
    replacement: String,
    anchor: usize,
    expected: &str,
    kind: Kind,
    caret: usize,
) -> Result<Edit, String> {
    let mut next = source.to_string();
    next.replace_range(range.clone(), &replacement);
    let mut document = Document::parse(&next)?;
    let end = range.start + replacement.len();
    let focus = document
        .blocks
        .iter()
        .find(|b| b.raw.start >= anchor && b.raw.end <= end && b.text == expected)
        .cloned();
    let focus = match focus {
        Some(block) => block,
        None if expected.is_empty() => {
            let mut block = Block::empty(anchor); block.kind = kind;
            containers::inherit(&next,&mut block);
            document.blocks.push(block.clone()); document.blocks.sort_by_key(|b| b.raw.start); block
        }
        _ => return Err("This split/merge would change the surrounding Markdown structure. The source is unchanged.".into()),
    };
    Ok(Edit {
        range,
        replacement,
        document,
        focus,
        caret,
    })
}

fn combine(left: &Block, right: &Block, a: String, b: String) -> String {
    // Adjacent identical wrappers need to share a delimiter, otherwise e.g.
    // **one****two** can become literal asterisks instead of two bold runs.
    let same = left.leaves.last().zip(right.leaves.first());
    for (token, compatible) in [
        (
            "**",
            same.is_some_and(|(a, b)| a.marks.bold && b.marks.bold),
        ),
        (
            "__",
            same.is_some_and(|(a, b)| a.marks.bold && b.marks.bold),
        ),
        (
            "~~",
            same.is_some_and(|(a, b)| a.marks.strike && b.marks.strike),
        ),
        (
            "*",
            same.is_some_and(|(a, b)| a.marks.italic && b.marks.italic),
        ),
        (
            "_",
            same.is_some_and(|(a, b)| a.marks.italic && b.marks.italic),
        ),
        ("`", same.is_some_and(|(a, b)| a.marks.code && b.marks.code)),
    ] {
        if compatible && a.ends_with(token) && b.starts_with(token) {
            return format!("{}{}", &a[..a.len() - token.len()], &b[token.len()..]);
        }
    }
    format!("{a}{b}")
}

pub(super) fn replace_across(
    doc: &Document,
    a: usize,
    start: usize,
    b: usize,
    end: usize,
    inserted: &str,
) -> Result<Option<super::selection::Edit>, String> {
    let left = &doc.blocks[a];
    let right = &doc.blocks[b];
    let (Some(first), Some(last)) = (prefix(&doc.source, left), prefix(&doc.source, right)) else {
        return Ok(None);
    };
    if first.family != last.family {
        return Ok(None);
    }
    let x = left
        .inline_slice(&doc.source, 0..start)
        .map_err(str::to_string)?;
    let y = right
        .inline_slice(&doc.source, end..right.text.len())
        .map_err(str::to_string)?;
    let inline = if inserted.is_empty() {
        combine(left, right, x, y)
    } else {
        format!("{x}{}{y}", super::document::escaped(inserted))
    };
    let range = first.start..right.raw.end;
    let replacement = format!("{}{}", first.first, protect_spaces(&inline));
    let mut source = doc.source.clone();
    source.replace_range(range.clone(), &replacement);
    let mut document = Document::parse(&source)?;
    if expected_empty(left, start, right, end, inserted)
        && document
            .blocks
            .iter()
            .all(|p| p.raw.start < range.start || p.raw.start > range.start + replacement.len())
    {
        let mut focus = Block::empty(range.start + first.first.len());
        focus.kind = left.kind;
        document.blocks.push(focus.clone());
        document.blocks.sort_by_key(|b| b.raw.start);
        return Ok(Some(super::selection::Edit {
            range,
            replacement,
            document,
            focus,
            caret: 0,
        }));
    }
    let parts = document
        .blocks
        .iter()
        .filter(|p| p.raw.start >= range.start && p.raw.end <= range.start + replacement.len())
        .collect::<Vec<_>>();
    let separator = if matches!(
        left.kind,
        Kind::Bullet | Kind::Numbered(_) | Kind::Task(_, _)
    ) {
        "\n"
    } else {
        "\n\n"
    };
    let expected = format!("{}{}{}", &left.text[..start], inserted, &right.text[end..]);
    if parts
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join(separator)
        != expected
    {
        return Err(
            "The selection replacement would change Markdown structure. The source is unchanged."
                .into(),
        );
    }
    let mut caret = start + inserted.len();
    let mut focus = None;
    for part in &parts {
        if caret <= part.text.len() {
            focus = Some((*part).clone());
            break;
        }
        caret = caret.saturating_sub(part.text.len() + separator.len());
    }
    let focus = focus
        .or_else(|| parts.last().map(|p| (*p).clone()))
        .ok_or("The selection has no editable result")?;
    caret = caret.min(focus.text.len());
    Ok(Some(super::selection::Edit {
        range,
        replacement,
        document,
        focus,
        caret,
    }))
}

fn expected_empty(left: &Block, start: usize, right: &Block, end: usize, inserted: &str) -> bool {
    start == 0 && end == right.text.len() && inserted.is_empty() && !left.is_literal()
}

pub(super) fn edit(
    document: &Document,
    index: usize,
    selection: Range<usize>,
    operation: Operation,
) -> Result<Option<Edit>, String> {
    let Some(block) = document.blocks.get(index) else {
        return Err("The selected block changed.".into());
    };
    if selection.start > selection.end || block.text.get(selection.clone()).is_none() {
        return Err("The visual selection changed.".into());
    }
    let source = &document.source;
    let Some(context) = prefix(source, block) else {
        return Ok(None);
    };
    let newline = if source
        .get(block.raw.end..)
        .is_some_and(|s| s.starts_with("\r\n"))
        || source[block.raw.clone()].contains("\r\n")
        || source.contains("\r\n")
    {
        "\r\n"
    } else {
        "\n"
    };
    match operation {
        Operation::Split => {
            if block.text.is_empty() && !matches!(block.kind, Kind::Paragraph) {
                let replacement = context.exit;
                let anchor = context.start + replacement.len();
                return validate(
                    source,
                    context.start..block.raw.end,
                    replacement,
                    anchor,
                    "",
                    Kind::Paragraph,
                    0,
                )
                .map(Some);
            }
            let left = protect_spaces(
                &block
                    .inline_slice(source, 0..selection.start)
                    .map_err(str::to_string)?,
            );
            let right = protect_spaces(
                &block
                    .inline_slice(source, selection.end..block.text.len())
                    .map_err(str::to_string)?,
            );
            let first = format!("{}{left}", context.first);
            let (separator, mut kind) = match block.kind {
                Kind::Paragraph | Kind::Heading(_) => (
                    format!("{newline}{newline}{}", context.next),
                    Kind::Paragraph,
                ),
                Kind::Quote => (
                    format!(
                        "{newline}{}{newline}{}",
                        context.first.trim_end(),
                        context.next
                    ),
                    Kind::Quote,
                ),
                _ => (format!("{newline}{}", context.next), block.kind),
            };
            let anchor = context.start + first.len() + separator.len();
            // Empty list items with nested children may have no Paragraph node.
            // The transient editor must target its own checkbox, not the old one.
            if matches!(kind, Kind::Task(_, _)) {
                let check = context.next.rfind('[').ok_or("Missing task marker")?;
                kind = Kind::Task(false, anchor - context.next.len() + check + 1);
            } else if let Kind::Numbered(number) = kind {
                kind = Kind::Numbered(
                    number
                        .checked_add(1)
                        .ok_or("List numbering exceeds the supported range")?,
                );
            }
            let edit = validate(
                source,
                context.start..block.raw.end,
                format!("{first}{separator}{right}"),
                anchor,
                &block.text[selection.end..],
                kind,
                0,
            )?;
            let expected_left = &block.text[..selection.start];
            if !expected_left.is_empty()
                && !edit.document.blocks.iter().any(|b| {
                    b.raw.start >= context.start && b.raw.end <= anchor && b.text == expected_left
                })
            {
                return Err(
                    "This split would change the first half of the block. The source is unchanged."
                        .into(),
                );
            }
            Ok(Some(edit))
        }
        Operation::Backward | Operation::Forward => {
            if !selection.is_empty() {
                return Ok(None);
            }
            let (a, b) = match operation {
                Operation::Backward if selection.start == 0 && index > 0 => (index - 1, index),
                Operation::Forward
                    if selection.end == block.text.len() && index + 1 < document.blocks.len() =>
                {
                    (index, index + 1)
                }
                _ => return Ok(None),
            };
            let left = &document.blocks[a];
            let right = &document.blocks[b];
            let (Some(left_context), Some(right_context)) =
                (prefix(source, left), prefix(source, right))
            else {
                return Ok(None);
            };
            if left_context.family != right_context.family || left.raw.end > right_context.start {
                return Ok(None);
            }
            let gap = source
                .get(left.raw.end..right_context.start)
                .ok_or("Invalid block boundary")?;
            if !gap
                .chars()
                .all(|c| c.is_whitespace() || context.quote && c == '>')
            {
                return Ok(None);
            }
            let inline = combine(
                left,
                right,
                left.inline_slice(source, 0..left.text.len())
                    .map_err(str::to_string)?,
                right
                    .inline_slice(source, 0..right.text.len())
                    .map_err(str::to_string)?,
            );
            let expected = format!("{}{}", left.text, right.text);
            let replacement = format!("{}{}", left_context.first, protect_spaces(&inline));
            validate(
                source,
                left_context.start..right.raw.end,
                replacement,
                left_context.start,
                &expected,
                left.kind,
                left.text.len(),
            )
            .map(Some)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Operation, edit};
    use crate::markdown_editor::document::Document;

    fn apply(
        source: &str,
        index: usize,
        selection: std::ops::Range<usize>,
        operation: Operation,
    ) -> super::Edit {
        let doc = Document::parse(source).unwrap();
        edit(&doc, index, selection, operation).unwrap().unwrap()
    }

    #[test]
    fn split_preserves_inline_styles_unicode_links_and_unrelated_source() {
        let split = apply(
            "before\n\nA **żółw** and [link](https://example.com)\n\n<Widget />",
            1,
            6..6,
            Operation::Split,
        );
        assert_eq!(
            split.document.source,
            "before\n\nA **żó**\n\n**łw** and [link](https://example.com)\n\n<Widget />"
        );
        assert_eq!(split.focus.text, "łw and link");
        assert_eq!(split.caret, 0);
    }

    #[test]
    fn enter_splits_headings_to_body_and_replaces_visual_selections() {
        assert_eq!(
            apply("# Hello world", 0, 5..6, Operation::Split)
                .document
                .source,
            "# Hello\n\nworld"
        );
        assert_eq!(
            apply("Hello world", 0, 5..5, Operation::Split)
                .document
                .source,
            "Hello\n\n&#32;world"
        );
        assert_eq!(
            apply("**hello world**", 0, 6..6, Operation::Split)
                .document
                .source,
            "**hello&#32;**\n\n**world**"
        );
        assert_eq!(apply("Hello", 0, 5..5, Operation::Split).focus.text, "");
    }

    #[test]
    fn lists_continue_numbering_and_reset_the_new_task_checkbox() {
        assert_eq!(
            apply("- one two", 0, 3..4, Operation::Split)
                .document
                .source,
            "- one\n- two"
        );
        assert_eq!(
            apply("9) one two", 0, 3..4, Operation::Split)
                .document
                .source,
            "9) one\n10) two"
        );
        assert_eq!(
            apply("- [x] one two", 0, 3..4, Operation::Split)
                .document
                .source,
            "- [x] one\n- [ ] two"
        );
        assert_eq!(apply("- ", 0, 0..0, Operation::Split).document.source, "");
        assert_eq!(apply("## ", 0, 0..0, Operation::Split).document.source, "");
    }

    #[test]
    fn quote_paragraphs_and_crlf_are_preserved() {
        assert_eq!(
            apply("> one two\r\n", 0, 3..4, Operation::Split)
                .document
                .source,
            "> one\r\n>\r\n> two\r\n"
        );
        assert_eq!(
            apply("one\r\n\r\ntwo\r\n", 1, 0..0, Operation::Backward)
                .document
                .source,
            "onetwo\r\n"
        );
    }

    #[test]
    fn boundary_merge_preserves_styles_and_places_the_caret_at_the_join() {
        let merged = apply(
            "**one**\n\n**two**\n\nuntouched",
            1,
            0..0,
            Operation::Backward,
        );
        assert_eq!(merged.document.source, "**onetwo**\n\nuntouched");
        assert_eq!(merged.caret, 3);
        assert_eq!(
            apply("one\n\n## two", 0, 3..3, Operation::Forward)
                .document
                .source,
            "onetwo"
        );
        assert_eq!(
            apply("- one\n- two", 1, 0..0, Operation::Backward)
                .document
                .source,
            "- onetwo"
        );
    }

    #[test]
    fn boundaries_do_not_eat_tables_code_or_different_containers() {
        for (source, index, caret) in [
            ("one\n\n```\ncode\n```", 0, 3),
            ("one\n\n- two", 0, 3),
            ("one\n\n---\n\ntwo", 0, 3),
        ] {
            let doc = Document::parse(source).unwrap();
            assert!(
                edit(&doc, index, caret..caret, Operation::Forward)
                    .unwrap()
                    .is_none()
            );
        }
        let doc = Document::parse("żółw").unwrap();
        assert!(edit(&doc, 0, 1..1, Operation::Split).is_err());
    }

    #[test]
    fn empty_task_split_keeps_the_new_checkbox_identity_with_nested_children() {
        let split = apply("- [x] one\n  - child", 0, 3..3, Operation::Split);
        assert_eq!(split.document.source, "- [x] one\n- [ ] \n  - child");
        let crate::markdown_editor::document::Kind::Task(checked, offset) = split.focus.kind else {
            panic!("new task must retain its own kind");
        };
        assert!(!checked);
        assert_eq!(&split.document.source[offset - 1..offset + 2], "[ ]");
        assert!(offset > split.document.source.find('x').unwrap());
    }
}
