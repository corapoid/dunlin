//! Source-backed editable blocks/cells. Only changed ranges are rewritten;
//! unusual syntax remains editable literally, never lossily reserialized.
use markdown::{ParseOptions, mdast::Node};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Marks {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Leaf {
    pub visible: Range<usize>,
    pub raw: Range<usize>,
    pub text: String,
    pub marks: Marks,
    code: bool,
    boundaries: Option<Vec<(usize, usize)>>,
}
#[derive(Clone, Debug)]
struct Inline {
    raw: Range<usize>,
    leaf: Option<usize>,
    children: Vec<Inline>,
    keep_empty: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Paragraph,
    Heading(u8),
    Bullet,
    Numbered(u32),
    Quote,
    Task(bool, usize),
    Cell {
        table: usize,
        row: usize,
        column: usize,
        columns: usize,
        align: markdown::mdast::AlignKind,
    },
    Code,
    Metadata,
    Html,
    Definition,
    Rule,
    Literal,
}

#[derive(Clone, Debug)]
enum Content {
    Inline,
    Literal,
    Fenced {
        opening: Range<usize>,
        body: Range<usize>,
        closing: Range<usize>,
        marker: char,
        length: usize,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Block {
    pub raw: Range<usize>,
    pub kind: Kind,
    pub text: String,
    pub leaves: Vec<Leaf>,
    pub editable: bool,
    pub containers: Vec<super::containers::Container>,
    tree: Inline,
    content: Content,
}
impl Block {
    pub fn empty(at: usize) -> Self {
        Self {
            raw: at..at,
            kind: Kind::Paragraph,
            text: String::new(),
            leaves: vec![Leaf {
                visible: 0..0,
                raw: at..at,
                text: String::new(),
                marks: Marks::default(),
                code: false,
                boundaries: None,
            }],
            editable: true,
            containers: Vec::new(),
            content: Content::Inline,
            tree: Inline {
                raw: at..at,
                leaf: Some(0),
                children: Vec::new(),
                keep_empty: true,
            },
        }
    }

    pub fn source_selection(&self, visible: Range<usize>) -> Option<Range<usize>> {
        if !matches!(self.content, Content::Inline) {
            return None;
        }
        let point = |at: usize, end: bool| {
            let leaf = if end {
                self.leaves
                    .iter()
                    .rev()
                    .find(|l| l.visible.start < at && l.visible.end >= at)
            } else {
                self.leaves
                    .iter()
                    .find(|l| l.visible.start <= at && l.visible.end > at)
            }
            .or_else(|| self.leaves.last())?;
            if leaf.code {
                return None;
            }
            let local = at.saturating_sub(leaf.visible.start).min(leaf.text.len());
            Some(leaf.raw.start + leaf.raw_offset(local)?)
        };
        let start = point(visible.start, false)?;
        Some(
            start..if visible.is_empty() {
                start
            } else {
                point(visible.end, true)?
            },
        )
    }

    pub fn replace_visible(&self, source: &str, new: &str) -> Result<String, &'static str> {
        if new == self.text {
            return source
                .get(self.raw.clone())
                .map(str::to_string)
                .ok_or("The source changed.");
        }
        match &self.content {
            Content::Literal => return Ok(new.into()),
            Content::Fenced {
                opening,
                body,
                closing,
                marker,
                length,
            } => {
                let fence = marker.to_string().repeat(
                    (*length).max(
                        new.lines()
                            .map(|line| {
                                line.trim_start()
                                    .chars()
                                    .take_while(|c| c == marker)
                                    .count()
                                    + 1
                            })
                            .max()
                            .unwrap_or(3),
                    ),
                );
                let original = source.get(self.raw.clone()).ok_or("The source changed.")?;
                let newline = if original.contains("\r\n") {
                    "\r\n"
                } else {
                    "\n"
                };
                let value = new.replace("\r\n", "\n").replace('\n', newline);
                let prefix = source
                    .get(self.raw.start..opening.start)
                    .ok_or("Invalid fence")?;
                let info = source.get(opening.end..body.start).ok_or("Invalid fence")?;
                let suffix = source
                    .get(closing.end..self.raw.end)
                    .ok_or("Invalid fence")?;
                let closing_indent = source.get(body.end..closing.start).ok_or("Invalid fence")?;
                return Ok(format!(
                    "{prefix}{fence}{info}{value}{}{closing_indent}{fence}{suffix}",
                    if value.is_empty() { "" } else { newline }
                ));
            }
            Content::Inline => {}
        }
        let (changed, inserted) = difference(&self.text, new);
        let boundary = changed
            .is_empty()
            .then(|| {
                self.leaves.iter().rposition(|l| {
                    l.visible.end == changed.start && l.visible.start < changed.start
                })
            })
            .flatten();
        let target = boundary
            .or_else(|| {
                self.leaves
                    .iter()
                    .position(|l| l.visible.start <= changed.start && changed.start < l.visible.end)
            })
            .or_else(|| {
                self.leaves
                    .iter()
                    .rposition(|l| l.visible.end == changed.start)
            })
            .unwrap_or(0);
        let mut values = Vec::new();
        for (index, leaf) in self.leaves.iter().enumerate() {
            let a = changed
                .start
                .saturating_sub(leaf.visible.start)
                .min(leaf.text.len());
            let b = changed
                .end
                .saturating_sub(leaf.visible.start)
                .min(leaf.text.len());
            let value = if index == target || a < b {
                format!(
                    "{}{}{}",
                    &leaf.text[..a],
                    if index == target { inserted } else { "" },
                    &leaf.text[b..]
                )
            } else {
                leaf.text.clone()
            };
            values.push(value);
        }
        emit(
            &self.tree,
            &self.leaves,
            &values,
            source,
            matches!(self.kind, Kind::Cell { .. }),
        )
        .map(|(text, _)| text)
    }

    fn literal(raw: Range<usize>, kind: Kind, source: &str) -> Self {
        let mut block = Self::empty(raw.start);
        block.text = source[raw.clone()].into();
        block.tree.raw = raw.clone();
        block.raw = raw;
        block.kind = kind;
        block.leaves.clear();
        block.content = Content::Literal;
        block
    }

    pub fn is_literal(&self) -> bool {
        !matches!(self.content, Content::Inline)
    }

    /// Balanced Markdown for a visual text slice, with the block's heading /
    /// container markers excluded. Only this touched block is reconstructed.
    pub fn inline_slice(
        &self,
        source: &str,
        selection: Range<usize>,
    ) -> Result<String, &'static str> {
        if self.is_literal() || self.text.get(selection.clone()).is_none() {
            return Err("Invalid prose selection");
        }
        if self.leaves.iter().any(|leaf| leaf.text.is_empty())
            && source
                .get(self.raw.clone())
                .is_some_and(|raw| raw.contains("!["))
        {
            return Err("Split around the image rather than its empty description.");
        }
        let values = self
            .leaves
            .iter()
            .map(|leaf| {
                let start = selection
                    .start
                    .saturating_sub(leaf.visible.start)
                    .min(leaf.text.len());
                let end = selection
                    .end
                    .saturating_sub(leaf.visible.start)
                    .min(leaf.text.len());
                leaf.text
                    .get(start..end)
                    .map(str::to_string)
                    .ok_or("Invalid UTF-8 prose selection")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut tree = self.tree.clone();
        if tree.leaf.is_none()
            && let (Some(first), Some(last)) = (tree.children.first(), tree.children.last())
        {
            tree.raw = first.raw.start..last.raw.end;
        }
        slice_tree(&tree, &self.leaves, &values, source)
    }

    /// Keep a native editor and its selection alive while Enter/paste creates
    /// multiple paragraphs. Join source maps, never serialize the whole file.
    pub fn join(source: &str, raw: Range<usize>, kind: Kind, parts: &[Block]) -> Option<Self> {
        let mut block = Self::empty(raw.start);
        block.raw = raw.clone();
        block.kind = kind;
        block.containers = parts
            .first()
            .map(|p| p.containers.clone())
            .unwrap_or_default();
        block.text.clear();
        block.leaves.clear();
        block.tree = Inline {
            raw: raw.clone(),
            leaf: None,
            children: Vec::new(),
            keep_empty: true,
        };
        let mut at = raw.start;
        for part in parts {
            if part.raw.start < at || part.raw.end > raw.end || part.is_literal() {
                return None;
            }
            if part.raw.start > at {
                block.push_gap(source, at..part.raw.start)?;
            }
            let mut tree = part.tree.clone();
            fn shift(tree: &mut Inline, n: usize) {
                if let Some(i) = &mut tree.leaf {
                    *i += n;
                }
                for child in &mut tree.children {
                    shift(child, n);
                }
            }
            shift(&mut tree, block.leaves.len());
            let start = block.text.len();
            for mut leaf in part.leaves.clone() {
                leaf.visible.start += start;
                leaf.visible.end += start;
                block.leaves.push(leaf);
            }
            block.text.push_str(&part.text);
            block.tree.children.push(tree);
            at = part.raw.end;
        }
        if at < raw.end {
            block.push_gap(source, at..raw.end)?;
        }
        Some(block)
    }

    fn push_gap(&mut self, source: &str, raw: Range<usize>) -> Option<()> {
        let (text, boundaries) = decode_text(source.get(raw.clone())?);
        let start = self.text.len();
        self.text.push_str(&text);
        self.tree.children.push(Inline {
            raw: raw.clone(),
            leaf: Some(self.leaves.len()),
            children: Vec::new(),
            keep_empty: true,
        });
        self.leaves.push(Leaf {
            visible: start..self.text.len(),
            raw,
            text,
            marks: Marks::default(),
            code: false,
            boundaries: Some(boundaries),
        });
        Some(())
    }
}

impl Leaf {
    fn raw_offset(&self, visible: usize) -> Option<usize> {
        self.boundaries.as_ref().map_or(Some(visible), |map| {
            map.iter().find(|(v, _)| *v == visible).map(|(_, raw)| *raw)
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct Document {
    pub source: String,
    pub blocks: Vec<Block>,
}
impl Document {
    pub fn parse(source: &str) -> Result<Self, String> {
        if !super::preview_allowed(source) {
            return Err(
                "Visual mode is limited to 128 KiB / 2,000 lines. Use Raw for this file.".into(),
            );
        }
        let mut options = ParseOptions::gfm();
        options.constructs.frontmatter = true;
        let ast = markdown::to_mdast(source, &options)
            .map_err(|_| "Could not parse Markdown. The original source is unchanged.")?;
        let mut blocks = Vec::new();
        let mut budget = 10_000;
        flatten(&ast, source, None, 0, &mut budget, &mut blocks)?;
        super::containers::annotate(&ast, source, &mut blocks);
        if blocks.len() > 1000 {
            return Err("Visual mode is limited to 1,000 blocks. Use Raw for this file.".into());
        }
        if blocks.is_empty() || source.ends_with("\n\n") || source.ends_with("\r\n\r\n") {
            blocks.push(Block::empty(source.len()));
        }
        Ok(Self {
            source: source.into(),
            blocks,
        })
    }
}

fn range(node: &Node) -> Result<Range<usize>, String> {
    let p = node.position().ok_or("Missing Markdown source location")?;
    Ok(p.start.offset..p.end.offset)
}
fn flatten(
    node: &Node,
    source: &str,
    context: Option<Kind>,
    depth: usize,
    budget: &mut usize,
    blocks: &mut Vec<Block>,
) -> Result<(), String> {
    if depth > 48 || *budget == 0 {
        return Err("Markdown is too deeply nested for visual editing. Use Raw.".into());
    }
    *budget -= 1;
    match node {
        Node::Root(root) => {
            for child in &root.children {
                flatten(child, source, context, depth + 1, budget, blocks)?;
            }
        }
        Node::List(list) => {
            for (i, item) in list.children.iter().enumerate() {
                flatten(
                    item,
                    source,
                    Some(if list.ordered {
                        Kind::Numbered(list.start.unwrap_or(1) + i as u32)
                    } else {
                        Kind::Bullet
                    }),
                    depth + 1,
                    budget,
                    blocks,
                )?;
            }
        }
        Node::ListItem(item) => {
            let context = if let Some(checked) = item.checked {
                let start = range(node)?.start;
                let end = item
                    .children
                    .first()
                    .map(range)
                    .transpose()?
                    .map_or(start, |r| r.start);
                source
                    .get(start..end)
                    .and_then(|prefix| prefix.find('['))
                    .map(|n| Kind::Task(checked, start + n + 1))
                    .or(context)
            } else {
                context
            };
            if item.children.is_empty() {
                let mut block = Block::empty(range(node)?.end);
                block.kind = context.unwrap_or(Kind::Bullet);
                blocks.push(block);
            }
            for child in &item.children {
                flatten(child, source, context, depth + 1, budget, blocks)?;
            }
        }
        Node::Blockquote(quote) => {
            if quote.children.is_empty() {
                let mut block = Block::empty(range(node)?.end);
                block.kind = Kind::Quote;
                blocks.push(block);
            }
            for child in &quote.children {
                flatten(child, source, Some(Kind::Quote), depth + 1, budget, blocks)?;
            }
        }
        Node::FootnoteDefinition(note) => {
            for child in &note.children {
                flatten(child, source, context, depth + 1, budget, blocks)?;
            }
        }
        Node::Table(table) => {
            let id = range(node)?.start;
            for (r, row) in table.children.iter().enumerate() {
                if let Some(cells) = row.children() {
                    for (c, cell) in cells.iter().enumerate() {
                        flatten(
                            cell,
                            source,
                            Some(Kind::Cell {
                                table: id,
                                row: r,
                                column: c,
                                columns: table.align.len(),
                                align: table
                                    .align
                                    .get(c)
                                    .copied()
                                    .unwrap_or(markdown::mdast::AlignKind::None),
                            }),
                            depth + 1,
                            budget,
                            blocks,
                        )?;
                    }
                }
            }
        }
        Node::Paragraph(_) | Node::Heading(_) | Node::TableCell(_) => {
            let raw = range(node)?;
            let kind = match node {
                Node::Heading(h) => Kind::Heading(h.depth),
                _ => context.unwrap_or(Kind::Paragraph),
            };
            let mut block = Block {
                raw: raw.clone(),
                kind,
                text: String::new(),
                leaves: Vec::new(),
                editable: true,
                containers: Vec::new(),
                content: Content::Inline,
                tree: Inline {
                    raw,
                    leaf: None,
                    children: Vec::new(),
                    keep_empty: true,
                },
            };
            match inline(node, source, Marks::default(), &mut block, depth, budget) {
                Ok(tree) => block.tree = tree,
                Err(_) => block = Block::literal(block.raw, kind, source),
            }
            if block.leaves.is_empty() && block.editable {
                // Empty ATX headings have no child text node.
                let end = block.raw.end;
                block.leaves.push(Leaf {
                    visible: 0..0,
                    raw: end..end,
                    text: String::new(),
                    marks: Marks::default(),
                    code: false,
                    boundaries: None,
                });
                block.tree.children.push(Inline {
                    raw: end..end,
                    leaf: Some(0),
                    children: Vec::new(),
                    keep_empty: true,
                });
            }
            blocks.push(block);
        }
        Node::Code(code) => {
            let raw = range(node)?;
            let literal = &source[raw.clone()];
            let indent = literal.len() - literal.trim_start_matches([' ', '\t']).len();
            let marker = literal[indent..].chars().next().unwrap_or(' ');
            let length = literal[indent..]
                .chars()
                .take_while(|c| *c == marker)
                .count();
            let mut block = Block::literal(raw.clone(), Kind::Code, source);
            if matches!(marker, '`' | '~')
                && length >= 3
                && let Some(first) = literal.find('\n')
            {
                let last_line = literal.rfind('\n').map_or(0, |n| n + 1);
                let closing = &literal[last_line..];
                let closing_indent = closing.len() - closing.trim_start_matches([' ', '\t']).len();
                let closing_len = closing[closing_indent..]
                    .chars()
                    .take_while(|c| *c == marker)
                    .count();
                if last_line > first && closing_len >= length {
                    block.text = code.value.clone();
                    block.content = Content::Fenced {
                        opening: raw.start + indent..raw.start + indent + length,
                        body: raw.start + first + 1..raw.start + last_line,
                        closing: raw.start + last_line + closing_indent
                            ..raw.start + last_line + closing_indent + closing_len,
                        marker,
                        length,
                    };
                }
            }
            blocks.push(block);
        }
        _ => {
            let raw = range(node)?;
            let kind = match node {
                Node::Html(_) => Kind::Html,
                Node::Yaml(_) | Node::Toml(_) => Kind::Metadata,
                Node::Definition(_) => Kind::Definition,
                Node::ThematicBreak(_) => Kind::Rule,
                _ => Kind::Literal,
            };
            blocks.push(Block::literal(raw, kind, source));
        }
    }
    Ok(())
}

fn inline(
    node: &Node,
    source: &str,
    mut marks: Marks,
    block: &mut Block,
    depth: usize,
    budget: &mut usize,
) -> Result<Inline, String> {
    if depth > 48 || *budget == 0 {
        return Err("Markdown nesting limit".into());
    }
    *budget -= 1;
    let raw = range(node)?;
    let literal = source
        .get(raw.clone())
        .ok_or("Invalid Markdown source location")?;
    let mut tree = Inline {
        raw: raw.clone(),
        leaf: None,
        children: Vec::new(),
        keep_empty: matches!(
            node,
            Node::Paragraph(_) | Node::Heading(_) | Node::TableCell(_)
        ),
    };
    let leaf = match node {
        Node::Text(text) if text_boundaries(literal, &text.value).is_some() => {
            Some((text.value.clone(), false))
        }
        Node::InlineCode(code) => {
            marks.code = true;
            Some((code.value.clone(), true))
        }
        Node::Strong(_) => {
            marks.bold = true;
            None
        }
        Node::Emphasis(_) => {
            marks.italic = true;
            None
        }
        Node::Delete(_) => {
            marks.strike = true;
            None
        }
        Node::Link(_) => {
            marks.link = true;
            None
        }
        Node::LinkReference(_) => {
            marks.link = true;
            None
        }
        Node::Break(_) => Some(("\n".into(), false)),
        Node::Html(html)
            if matches!(
                html.value.to_ascii_lowercase().as_str(),
                "<br>" | "<br/>" | "<br />"
            ) =>
        {
            Some(("\n".into(), false))
        }
        Node::Image(image) => {
            return image_inline(raw, &image.alt, source, marks, block);
        }
        Node::ImageReference(image) => {
            return image_inline(raw, &image.alt, source, marks, block);
        }
        Node::Html(_) | Node::FootnoteReference(_) => {
            marks.code = true;
            Some((literal.to_string(), false))
        }
        Node::Paragraph(_) | Node::Heading(_) | Node::TableCell(_) => None,
        _ => return Err("Unsupported inline projection".into()),
    };
    if let Some((text, code)) = leaf {
        let start = block.text.len();
        block.text.push_str(&text);
        tree.leaf = Some(block.leaves.len());
        let boundaries = (!code && literal != text).then(|| {
            text_boundaries(literal, &text)
                .unwrap_or_else(|| vec![(0, 0), (text.len(), literal.len())])
        });
        block.leaves.push(Leaf {
            visible: start..block.text.len(),
            raw,
            text,
            marks,
            code,
            boundaries,
        });
    } else if let Some(children) = node.children() {
        for child in children {
            tree.children
                .push(inline(child, source, marks, block, depth + 1, budget)?);
        }
    }
    Ok(tree)
}

fn image_inline(
    raw: Range<usize>,
    alt: &str,
    source: &str,
    mut marks: Marks,
    block: &mut Block,
) -> Result<Inline, String> {
    let literal = &source[raw.clone()];
    let mut depth = 0;
    let mut escaped = false;
    let end = literal
        .char_indices()
        .skip(2)
        .find_map(|(i, c)| {
            if escaped {
                escaped = false;
                return None;
            }
            if c == '\\' {
                escaped = true;
                return None;
            }
            if c == '[' {
                depth += 1;
            }
            if c == ']' {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            None
        })
        .ok_or("Invalid image label")?;
    let label = raw.start + 2..raw.start + end;
    let start = block.text.len();
    block.text.push_str(alt);
    marks.italic = true;
    let leaf = block.leaves.len();
    block.leaves.push(Leaf {
        visible: start..block.text.len(),
        raw: label.clone(),
        text: alt.into(),
        marks,
        code: false,
        boundaries: Some(decode_text(&source[label.clone()]).1),
    });
    Ok(Inline {
        raw,
        leaf: None,
        children: vec![Inline {
            raw: label,
            leaf: Some(leaf),
            children: Vec::new(),
            keep_empty: true,
        }],
        keep_empty: true,
    })
}

fn decode_text(raw: &str) -> (String, Vec<(usize, usize)>) {
    let mut value = String::new();
    let mut boundaries = vec![(0, 0)];
    let mut at = 0;
    while at < raw.len() {
        let tail = &raw[at..];
        if tail.starts_with('\\') && tail.as_bytes().get(1).is_some_and(u8::is_ascii_punctuation) {
            value.push(tail.as_bytes()[1] as char);
            at += 2;
        } else if tail.starts_with('&')
            && let Some(end) = tail
                .char_indices()
                .take_while(|(i, _)| *i <= 33)
                .find_map(|(i, c)| (c == ';').then_some(i))
        {
            let name = &tail[1..end];
            let decoded =
                if let Some(number) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                    u32::from_str_radix(number, 16)
                        .ok()
                        .map(|_| markdown::decode_numeric(number, 16))
                } else if let Some(number) = name.strip_prefix('#') {
                    number
                        .parse::<u32>()
                        .ok()
                        .map(|_| markdown::decode_numeric(number, 10))
                } else {
                    markdown::decode_named(name, true)
                };
            if let Some(decoded) = decoded {
                value.push_str(&decoded);
                at += end + 1;
            } else {
                value.push('&');
                at += 1;
            }
        } else if tail.starts_with("\r\n") {
            value.push('\n');
            at += 2;
        } else {
            let c = tail.chars().next().unwrap();
            value.push(c);
            at += c.len_utf8();
        }
        boundaries.push((value.len(), at));
    }
    (value, boundaries)
}

fn text_boundaries(raw: &str, expected: &str) -> Option<Vec<(usize, usize)>> {
    let (decoded, boundaries) = decode_text(raw);
    if decoded == expected {
        return Some(boundaries);
    }
    // MDAST strips quote markers and list continuation indentation from text,
    // but byte positions still span them. Retain those bytes in the source map.
    let mut output: Vec<(usize, usize)> = Vec::new();
    let mut raw_at = 0;
    let mut visible_at = 0;
    let mut lines = expected.split_inclusive('\n');
    for (index, line) in raw.split_inclusive('\n').enumerate() {
        let target = lines.next()?;
        let prefix = (0..=line.len())
            .filter(|n| line.is_char_boundary(*n))
            .take(257)
            .take_while(|n| line[..*n].chars().all(|c| matches!(c, ' ' | '\t' | '>')))
            .find(|n| (index > 0 || *n == 0) && decode_text(&line[*n..]).0 == target)?;
        let (_, map) = decode_text(&line[prefix..]);
        for (visible, raw) in map {
            let point = (visible_at + visible, raw_at + prefix + raw);
            if output.last().is_some_and(|last| last.0 == point.0) {
                *output.last_mut().unwrap() = point;
            } else {
                output.push(point);
            }
        }
        visible_at += target.len();
        raw_at += line.len();
    }
    (lines.next().is_none()).then_some(output)
}

pub(super) fn escaped(value: &str) -> String {
    let mut result = String::new();
    for c in value.chars() {
        if c.is_ascii_punctuation() {
            result.push('\\');
        }
        result.push(c);
    }
    result
}

fn emit(
    tree: &Inline,
    leaves: &[Leaf],
    values: &[String],
    source: &str,
    cell: bool,
) -> Result<(String, bool), &'static str> {
    let original = source
        .get(tree.raw.clone())
        .ok_or("The source changed. Reopen this block.")?;
    if let Some(index) = tree.leaf {
        let leaf = &leaves[index];
        let value = &values[index];
        if value == &leaf.text {
            return Ok((original.into(), false));
        }
        if value.is_empty() {
            return Ok((String::new(), true));
        }
        if leaf.code {
            let ticks = "`".repeat(value.split(|c| c != '`').map(str::len).max().unwrap_or(0) + 1);
            let pad = if value.starts_with('`')
                || value.ends_with('`')
                || (value.starts_with(' ') && value.ends_with(' '))
            {
                " "
            } else {
                ""
            };
            let value = if cell {
                value.replace('|', "\\|")
            } else {
                value.clone()
            };
            return Ok((format!("{ticks}{pad}{value}{pad}{ticks}"), true));
        }
        let (changed, inserted) = difference(&leaf.text, value);
        let (Some(a), Some(b)) = (leaf.raw_offset(changed.start), leaf.raw_offset(changed.end))
        else {
            return Ok((encode_text(value, cell), true));
        };
        return Ok((
            format!(
                "{}{}{}",
                &original[..a],
                encode_text(inserted, cell),
                &original[b..]
            ),
            true,
        ));
    }
    let mut output = String::new();
    let mut at = tree.raw.start;
    let mut changed = false;
    let mut has_text = false;
    for child in &tree.children {
        let (text, edited) = emit(child, leaves, values, source, cell)?;
        output.push_str(
            source
                .get(at..child.raw.start)
                .ok_or("Invalid Markdown source map")?,
        );
        has_text |= !text.is_empty();
        output.push_str(&text);
        changed |= edited;
        at = child.raw.end;
    }
    if !changed {
        return Ok((original.into(), false));
    }
    if !has_text && !tree.keep_empty {
        return Ok((String::new(), true));
    }
    output.push_str(
        source
            .get(at..tree.raw.end)
            .ok_or("Invalid Markdown source map")?,
    );
    Ok((output, true))
}

fn slice_tree(
    tree: &Inline,
    leaves: &[Leaf],
    values: &[String],
    source: &str,
) -> Result<String, &'static str> {
    fn selected(tree: &Inline, values: &[String]) -> bool {
        tree.leaf.is_some_and(|i| !values[i].is_empty())
            || tree.children.iter().any(|child| selected(child, values))
    }
    if !selected(tree, values) {
        return Ok(String::new());
    }
    if let Some(i) = tree.leaf {
        let text = emit(tree, leaves, values, source, false)?.0;
        return Ok(if !leaves[i].code && values[i] != leaves[i].text {
            protect_spaces(&text)
        } else {
            text
        });
    }
    let mut result = String::new();
    let mut at = tree.raw.start;
    for child in &tree.children {
        result.push_str(
            source
                .get(at..child.raw.start)
                .ok_or("Invalid inline source map")?,
        );
        result.push_str(&slice_tree(child, leaves, values, source)?);
        at = child.raw.end;
    }
    result.push_str(
        source
            .get(at..tree.raw.end)
            .ok_or("Invalid inline source map")?,
    );
    // Underscore emphasis cannot survive an intraword join. Use equivalent
    // asterisk delimiters for touched emphasis wrappers, not for literal text.
    if let (Some(first), Some(last)) = (tree.children.first(), tree.children.last()) {
        let prefix = source.get(tree.raw.start..first.raw.start).unwrap_or("");
        let suffix = source.get(last.raw.end..tree.raw.end).unwrap_or("");
        if matches!(prefix, "_" | "__")
            && prefix == suffix
            && result.starts_with(prefix)
            && result.ends_with(suffix)
        {
            let stars = "*".repeat(prefix.len());
            return Ok(format!(
                "{stars}{}{stars}",
                &result[prefix.len()..result.len() - suffix.len()]
            ));
        }
    }
    Ok(result)
}

pub(super) fn protect_spaces(value: &str) -> String {
    let start = value.len() - value.trim_start_matches([' ', '\t']).len();
    let end = value.trim_end_matches([' ', '\t']).len().max(start);
    let encode = |text: &str| text.replace(' ', "&#32;").replace('\t', "&#9;");
    format!(
        "{}{}{}",
        encode(&value[..start]),
        &value[start..end],
        encode(&value[end..])
    )
}

fn encode_text(value: &str, cell: bool) -> String {
    let escaped = escaped(value);
    if cell {
        escaped.replace("\r\n", "\n").replace('\n', "<br>")
    } else {
        escaped
    }
}

pub(super) fn difference<'a>(old: &str, new: &'a str) -> (Range<usize>, &'a str) {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    (prefix..old.len() - suffix, &new[prefix..new.len() - suffix])
}

#[cfg(test)]
mod tests {
    use super::{Document, Kind};
    fn replace(source: &str, index: usize, text: &str) -> String {
        let doc = Document::parse(source).unwrap();
        let block = &doc.blocks[index];
        let value = block.replace_visible(source, text).unwrap();
        let mut result = source.to_string();
        result.replace_range(block.raw.clone(), &value);
        result
    }
    #[test]
    fn visual_text_has_marks_but_no_markdown_delimiters() {
        let doc = Document::parse("# Title\n\nA **bold** _italic_ [link](https://example.com).\n")
            .unwrap();
        assert_eq!(doc.blocks[0].kind, Kind::Heading(1));
        assert_eq!(doc.blocks[0].text, "Title");
        assert_eq!(doc.blocks[1].text, "A bold italic link.");
        assert!(doc.blocks[1].leaves.iter().any(|l| l.marks.bold));
        assert!(doc.blocks[1].leaves.iter().any(|l| l.marks.link));
    }
    #[test]
    fn visual_edits_preserve_markers_and_unrelated_opaque_source() {
        let source = "# Title\n\nA **bold** word.\n\n<Widget key='untouched' />\n";
        assert_eq!(
            replace(source, 1, "A bolder word."),
            "# Title\n\nA **bolder** word.\n\n<Widget key='untouched' />\n"
        );
        assert_eq!(
            replace(source, 0, "Changed"),
            "# Changed\n\nA **bold** word.\n\n<Widget key='untouched' />\n"
        );
    }
    #[test]
    fn deletion_across_marks_never_leaves_unbalanced_delimiters() {
        assert_eq!(replace("A **bold** word", 0, "A bond"), "A **bon**d");
        assert_eq!(replace("A **bold** word", 0, "A  word"), "A  word");
    }
    #[test]
    fn unicode_and_literal_markdown_typed_visually_are_not_reinterpreted() {
        assert_eq!(replace("żółw", 0, "żółw*"), "żółw\\*");
        let doc = Document::parse("- one\n- two\n").unwrap();
        assert_eq!(doc.blocks[0].kind, Kind::Bullet);
        assert_eq!(
            replace("- one\n- two\n", 0, "changed"),
            "- changed\n- two\n"
        );
    }
    #[test]
    fn tables_and_entities_are_editable_without_lossy_document_conversion() {
        let source = "| A | B |\n| - | - |\n| x | y |\n\nEntity &amp; text\n";
        let doc = Document::parse(source).unwrap();
        assert!(doc.blocks.iter().all(|b| b.editable));
        assert!(matches!(
            doc.blocks[0].kind,
            Kind::Cell {
                row: 0,
                column: 0,
                ..
            }
        ));
        assert_eq!(doc.blocks.last().unwrap().text, "Entity & text");
        assert_eq!(doc.source, source);
        assert!(Document::parse("").unwrap().blocks[0].editable);
    }

    #[test]
    fn checkboxes_frontmatter_and_escaped_text_keep_their_source_locations() {
        let doc = Document::parse("---\ncustom: true\n---\n\n- [x] done\n").unwrap();
        assert!(doc.blocks[0].editable);
        assert!(matches!(doc.blocks[1].kind, Kind::Task(true, _)));
        let source = "Literal \\*star\\* and żółw";
        assert!(Document::parse(source).unwrap().blocks[0].editable);
        assert_eq!(
            replace(source, 0, "Literal *star* and żółwie"),
            "Literal \\*star\\* and żółwie"
        );
    }

    #[test]
    fn all_gfm_blocks_have_an_editable_projection() {
        let source = "---\ntitle: Example\n---\n\n# Heading\n\nA &amp; **bold** [link][ref] with ![alt](image.png).\n\n> quoted\n\n- [x] task\n\n```rust\nlet x = 1;\n```\n\n| A | B |\n| :- | -: |\n| x | y |\n\n<div>safe literal HTML</div>\n\n---\n\n[ref]: https://example.com\n\n[^note]: footnote\n";
        let doc = Document::parse(source).unwrap();
        assert!(doc.blocks.iter().all(|b| b.editable));
        for block in &doc.blocks {
            assert_eq!(
                block.replace_visible(source, &block.text).unwrap(),
                source[block.raw.clone()]
            );
        }
        assert!(
            doc.blocks
                .iter()
                .any(|b| b.kind == Kind::Code && b.text == "let x = 1;")
        );
        assert!(doc.blocks.iter().any(|b| b.kind == Kind::Html));
        assert!(doc.blocks.iter().any(|b| b.kind == Kind::Metadata));
    }

    #[test]
    fn edits_table_cells_entities_images_breaks_and_reference_labels() {
        let table = "| A | B |\n| :- | -: |\n| x | **y** |\n";
        assert_eq!(
            replace(table, 2, "x|z\nnext"),
            "| A | B |\n| :- | -: |\n| x\\|z<br>next | **y** |\n"
        );
        let edited = replace(table, 3, "yes");
        assert_eq!(edited, "| A | B |\n| :- | -: |\n| x | **yes** |\n");
        assert_eq!(
            replace("An &amp; &#x1F600; entity", 0, "An & 😀 entity!"),
            "An &amp; &#x1F600; entity\\!"
        );
        assert_eq!(
            replace("![alt](../image.png \"title\")", 0, "new alt"),
            "![new alt](../image.png \"title\")"
        );
        assert_eq!(
            replace("[label][ref]\n\n[ref]: https://example.com", 0, "new label"),
            "[new label][ref]\n\n[ref]: https://example.com"
        );
        assert_eq!(replace("one  \ntwo", 0, "one\nthree"), "one  \nthree");
    }

    #[test]
    fn fenced_code_is_edited_without_markers_and_cannot_close_its_own_fence() {
        let source = "before\n\n```rust example\nlet x = 1;\n```\n\nafter\n";
        assert_eq!(
            replace(source, 1, "let x = 2;\n```\nend"),
            "before\n\n````rust example\nlet x = 2;\n```\nend\n````\n\nafter\n"
        );
        assert_eq!(
            replace("~~~js\r\nx\r\n~~~\r\n", 0, "y\nz"),
            "~~~js\r\ny\r\nz\r\n~~~\r\n"
        );
        assert_eq!(replace("```\nx\n```", 0, "x\n"), "```\nx\n\n```");
        assert_eq!(replace("```\n```", 0, "\n"), "```\n\n\n```");
    }

    #[test]
    fn paragraph_enter_can_keep_a_live_projection_across_new_blocks() {
        let source = "A **bold** paragraph\n\nNext";
        let doc = Document::parse(source).unwrap();
        let joined =
            super::Block::join(source, 0..source.len(), Kind::Paragraph, &doc.blocks).unwrap();
        assert_eq!(joined.text, "A bold paragraph\n\nNext");
        assert_eq!(
            joined
                .replace_visible(source, "A bold paragraph\n\nNext!")
                .unwrap(),
            "A **bold** paragraph\n\nNext\\!"
        );
    }

    #[test]
    fn nested_multiline_prose_keeps_container_markers_out_of_the_editor() {
        let source = "> first\n> **second** line\n\n- list\n  continuation\n";
        let doc = Document::parse(source).unwrap();
        assert_eq!(doc.blocks[0].text, "first\nsecond line");
        assert!(!doc.blocks[0].is_literal());
        assert_eq!(
            replace(source, 0, "first\nchanged line"),
            "> first\n> **changed** line\n\n- list\n  continuation\n"
        );
        assert_eq!(doc.blocks[1].text, "list\ncontinuation");
        assert!(!doc.blocks[1].is_literal());
    }
}
