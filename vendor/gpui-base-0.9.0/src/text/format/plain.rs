//! Literal selectable text: no Markdown parsing or HTML whitespace cleanup.
use super::super::{
    document::ParsedDocument,
    node::{BlockNode, Paragraph},
};
use gpui::SharedString;
use std::sync::Arc;

pub(crate) fn parse(source: &str) -> Result<ParsedDocument, SharedString> {
    let mut paragraph = Paragraph::default();
    paragraph.push_str(source);
    Ok(ParsedDocument {
        source: source.to_string().into(),
        blocks: Arc::new(vec![BlockNode::Paragraph(paragraph)]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_text_preserves_whitespace_and_markup_without_interpreting_it() {
        let source = "  Żółty  <tag>& **literal**\n\tend\n";
        let document = parse(source).unwrap();
        let BlockNode::Paragraph(paragraph) = &document.blocks[0] else {
            panic!("plain paragraph");
        };
        assert_eq!(paragraph.text(), source);
        assert_eq!(document.source.as_ref(), source);
    }
}
