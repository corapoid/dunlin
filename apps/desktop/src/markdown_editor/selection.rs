//! Document coordinates, Unicode movement and atomic multi-block replacements.
use super::{
    document::{Block, Document, Kind},
    structure,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Point {
    pub block: usize,
    pub offset: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Selection {
    pub anchor: Point,
    pub head: Point,
}
impl Selection {
    pub fn ordered(self) -> (Point, Point) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }
    pub fn validate(self, doc: &Document) -> Result<(), String> {
        for point in [self.anchor, self.head] {
            let block = doc
                .blocks
                .get(point.block)
                .ok_or("The document selection changed.")?;
            if !block.text.is_char_boundary(point.offset) {
                return Err("Invalid Unicode selection.".into());
            }
        }
        Ok(())
    }
    pub fn range(self, index: usize, len: usize) -> Option<Range<usize>> {
        let (a, b) = self.ordered();
        (index >= a.block && index <= b.block).then_some({
            if a.block == b.block {
                a.offset..b.offset
            } else if index == a.block {
                a.offset..len
            } else if index == b.block {
                0..b.offset
            } else {
                0..len
            }
        })
    }
    pub fn text(self, doc: &Document) -> Result<String, String> {
        self.validate(doc)?;
        let (a, b) = self.ordered();
        let mut result = String::new();
        for index in a.block..=b.block {
            let block = &doc.blocks[index];
            if index > a.block {
                let previous = &doc.blocks[index - 1];
                result.push_str(match (previous.kind, block.kind) {
                    (
                        Kind::Cell {
                            table: t, row: r, ..
                        },
                        Kind::Cell {
                            table: u, row: s, ..
                        },
                    ) if t == u => {
                        if r == s {
                            "\t"
                        } else {
                            "\n"
                        }
                    }
                    (
                        Kind::Bullet | Kind::Numbered(_) | Kind::Task(_, _) | Kind::Quote,
                        Kind::Bullet | Kind::Numbered(_) | Kind::Task(_, _) | Kind::Quote,
                    ) => "\n",
                    _ => "\n\n",
                });
            }
            result.push_str(&block.text[self.range(index, block.text.len()).unwrap()]);
        }
        Ok(result)
    }
}

pub(super) fn step(doc: &Document, point: Point, backwards: bool) -> Point {
    let text = &doc.blocks[point.block].text;
    if backwards {
        if point.offset == 0 {
            if point.block == 0 {
                point
            } else {
                Point {
                    block: point.block - 1,
                    offset: doc.blocks[point.block - 1].text.len(),
                }
            }
        } else {
            Point {
                offset: text
                    .grapheme_indices(true)
                    .take_while(|(i, _)| *i < point.offset)
                    .last()
                    .map_or(0, |(i, _)| i),
                ..point
            }
        }
    } else if point.offset == text.len() {
        if point.block + 1 >= doc.blocks.len() {
            point
        } else {
            Point {
                block: point.block + 1,
                offset: 0,
            }
        }
    } else {
        Point {
            offset: text
                .grapheme_indices(true)
                .find(|(i, _)| *i > point.offset)
                .map_or(text.len(), |(i, _)| i),
            ..point
        }
    }
}

pub(super) struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    pub document: Document,
    pub focus: Block,
    pub caret: usize,
}

/// Vertical table movement keeps the column; leaving a grid skips the rest of
/// its cells instead of treating a horizontal neighbour as the next text line.
pub(super) fn vertical_block(doc: &Document, index: usize, backwards: bool) -> Option<usize> {
    if let Kind::Cell {
        table, row, column, ..
    } = doc.blocks.get(index)?.kind
    {
        let target_row = if backwards {
            row.checked_sub(1)
        } else {
            row.checked_add(1)
        };
        if let Some(row)=target_row && let Some(target)=doc.blocks.iter().position(|b|matches!(b.kind,Kind::Cell{table:t,row:r,column:c,..} if t==table && r==row && c==column)) {return Some(target);}
        if backwards {
            return doc
                .blocks
                .iter()
                .position(|b| matches!(b.kind,Kind::Cell{table:t,..} if t==table))?
                .checked_sub(1);
        }
        let next = doc
            .blocks
            .iter()
            .rposition(|b| matches!(b.kind,Kind::Cell{table:t,..} if t==table))?
            + 1;
        return (next < doc.blocks.len()).then_some(next);
    }
    if backwards {
        index.checked_sub(1)
    } else {
        (index + 1 < doc.blocks.len()).then_some(index + 1)
    }
}

pub(super) fn replace(
    doc: &Document,
    selection: Selection,
    inserted: &str,
) -> Result<Edit, String> {
    selection.validate(doc)?;
    let (a, b) = selection.ordered();
    if a.block == 0
        && a.offset == 0
        && b.block + 1 == doc.blocks.len()
        && b.offset == doc.blocks[b.block].text.len()
    {
        let replacement = super::document::protect_spaces(&super::document::escaped(inserted));
        let document = Document::parse(&replacement)?;
        let focus = document
            .blocks
            .last()
            .cloned()
            .unwrap_or_else(|| Block::empty(0));
        let caret = focus.text.len();
        return Ok(Edit {
            range: 0..doc.source.len(),
            replacement,
            document,
            focus,
            caret,
        });
    }
    if a.block != b.block
        && let Some(edit) =
            structure::replace_across(doc, a.block, a.offset, b.block, b.offset, inserted)?
    {
        return Ok(edit);
    }
    // Mixed code/table/syntax endpoints keep their structural scaffolding. A
    // single source transaction updates every selected field, not just the head.
    let first = &doc.blocks[a.block];
    let last = &doc.blocks[b.block];
    let range = first.raw.start..last.raw.end;
    if range.start > range.end {
        return Err("Invalid document range".into());
    }
    let mut replacement = doc.source[range.clone()].to_string();
    let mut patches = Vec::new();
    let mut expected = String::new();
    for index in a.block..=b.block {
        let block = &doc.blocks[index];
        let selected = selection.range(index, block.text.len()).unwrap();
        let value = format!(
            "{}{}{}",
            &block.text[..selected.start],
            if index == a.block { inserted } else { "" },
            &block.text[selected.end..]
        );
        if index == a.block {
            expected = value.clone();
        }
        patches.push((
            block.raw.clone(),
            block
                .replace_visible(&doc.source, &value)
                .map_err(str::to_string)?,
        ));
    }
    for (raw, value) in patches.into_iter().rev() {
        replacement.replace_range(raw.start - range.start..raw.end - range.start, &value);
    }
    let mut source = doc.source.clone();
    source.replace_range(range.clone(), &replacement);
    let mut document = Document::parse(&source)?;
    let focus = document
        .blocks
        .iter()
        .find(|block| block.raw.start == first.raw.start && block.text == expected)
        .cloned();
    let focus = match focus {
        Some(block) => block,
        None if expected.is_empty() => {
            let block = Block::empty(first.raw.start);
            document.blocks.push(block.clone());
            document.blocks.sort_by_key(|b| b.raw.start);
            block
        }
        _ => return Err(
            "The replacement would change the selected block structure. The source is unchanged."
                .into(),
        ),
    };
    Ok(Edit {
        range,
        replacement,
        document,
        focus,
        caret: a.offset + inserted.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Point, Selection, replace, step, vertical_block};
    use crate::markdown_editor::document::Document;
    #[test]
    fn movement_keeps_graphemes_and_document_boundaries() {
        let doc = Document::parse("a👩‍💻e\u{301}\n\nżółw").unwrap();
        let a = Point {
            block: 0,
            offset: 1,
        };
        let b = step(&doc, a, false);
        assert_eq!(&doc.blocks[0].text[a.offset..b.offset], "👩‍💻");
        assert_eq!(step(&doc, b, true), a);
        assert_eq!(
            step(
                &doc,
                Point {
                    block: 1,
                    offset: 0
                },
                true
            )
            .offset,
            doc.blocks[0].text.len()
        );
    }
    #[test]
    fn copy_and_replace_cross_blocks_preserving_marks_and_unrelated_source() {
        let doc = Document::parse("**one**\n\n_two_\n\nuntouched").unwrap();
        let selection = Selection {
            anchor: Point {
                block: 0,
                offset: 1,
            },
            head: Point {
                block: 1,
                offset: 2,
            },
        };
        assert_eq!(selection.text(&doc).unwrap(), "ne\n\ntw");
        let edit = replace(&doc, selection, "X").unwrap();
        assert_eq!(edit.focus.text, "oXo");
        assert!(edit.document.source.ends_with("\n\nuntouched"));
        assert_eq!(edit.caret, 2);
    }
    #[test]
    fn table_ranges_copy_tabs_and_delete_contents_without_losing_grid() {
        let doc = Document::parse("| A | B |\n| - | - |\n| x | y |\n").unwrap();
        let selection = Selection {
            anchor: Point {
                block: 2,
                offset: 0,
            },
            head: Point {
                block: 3,
                offset: 1,
            },
        };
        assert_eq!(selection.text(&doc).unwrap(), "x\ty");
        assert_eq!(
            replace(&doc, selection, "").unwrap().document.source,
            "| A | B |\n| - | - |\n|  |  |\n"
        );
    }
    #[test]
    fn whole_document_delete_and_replace_include_structural_scaffolding() {
        let doc = Document::parse("# title\n\n```rs\nx\n```\n\nend\n").unwrap();
        let selection = Selection {
            anchor: Point {
                block: 0,
                offset: 0,
            },
            head: Point {
                block: 2,
                offset: 3,
            },
        };
        assert_eq!(replace(&doc, selection, "").unwrap().document.source, "");
        let edit = replace(&doc, selection, "# literal 👩‍💻").unwrap();
        assert_eq!(edit.focus.text, "# literal 👩‍💻");
    }
    #[test]
    fn partial_mixed_selection_preserves_code_and_table_structure() {
        let doc =
            Document::parse("before\n\n```rs\none\n```\n\n| A | B |\n| - | - |\n\nafter").unwrap();
        let selection = Selection {
            anchor: Point {
                block: 1,
                offset: 1,
            },
            head: Point {
                block: 2,
                offset: 1,
            },
        };
        let edit = replace(&doc, selection, "X").unwrap();
        assert_eq!(edit.focus.text, "oX");
        assert!(edit.document.source.contains("```rs\noX\n```\n\n|  | B |"));
        assert!(edit.document.source.ends_with("\n\nafter"));
    }
    #[test]
    fn table_vertical_arrows_keep_the_column_and_cross_grid_boundaries() {
        let doc = Document::parse("before\n\n| A | B |\n| - | - |\n| x | y |\n\nafter").unwrap();
        assert_eq!(vertical_block(&doc, 1, false), Some(3));
        assert_eq!(vertical_block(&doc, 4, true), Some(2));
        assert_eq!(vertical_block(&doc, 2, true), Some(0));
        assert_eq!(vertical_block(&doc, 3, false), Some(5));
    }
}
