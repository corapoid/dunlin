//! Source-only transformations. Byte ranges are checked before slicing; no
//! parsing/reserializing the document, so unknown Markdown stays byte-identical.
use std::ops::Range;

#[derive(Clone, Copy, Debug)]
pub(super) enum Format {
    Bold,
    Italic,
    Strike,
    InlineCode,
    Heading,
    Bullet,
    Numbered,
    Task,
    Quote,
    CodeBlock,
    Link,
    Image,
    Table,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    pub selection: Range<usize>,
}

fn max_ticks(text: &str) -> usize {
    text.split(|c| c != '`').map(str::len).max().unwrap_or(0)
}

pub(super) fn edit(
    source: &str,
    selection: Range<usize>,
    format: Format,
) -> Result<Edit, &'static str> {
    if selection.start > selection.end || source.get(selection.clone()).is_none() {
        return Err("The selection changed. Select the text again.");
    }
    let selected = &source[selection.clone()];
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let wrap = |before: &str, after: &str, placeholder: &str| {
        let text = if selected.is_empty() {
            placeholder
        } else {
            selected
        };
        Edit {
            range: selection.clone(),
            replacement: format!("{before}{text}{after}"),
            selection: (selection.start + before.len())
                ..(selection.start + before.len() + text.len()),
        }
    };
    let toggle = |before: &str, after: &str, placeholder: &str| {
        if !selection.is_empty()
            && selection.start >= before.len()
            && source.get(selection.start - before.len()..selection.start) == Some(before)
            && source.get(selection.end..selection.end + after.len()) == Some(after)
        {
            Edit {
                range: selection.start - before.len()..selection.end + after.len(),
                replacement: selected.into(),
                selection: selection.start - before.len()..selection.end - before.len(),
            }
        } else {
            wrap(before, after, placeholder)
        }
    };
    let block = |body: String| {
        let before = if selection.start > 0 && !source[..selection.start].ends_with('\n') {
            newline
        } else {
            ""
        };
        let after = if selection.end < source.len() && !source[selection.end..].starts_with('\n') {
            newline
        } else {
            ""
        };
        Edit {
            range: selection.clone(),
            selection: (selection.start + before.len())
                ..(selection.start + before.len() + body.len()),
            replacement: format!("{before}{body}{after}"),
        }
    };
    Ok(match format {
        Format::Bold => toggle("**", "**", "bold text"),
        Format::Italic => toggle("_", "_", "italic text"),
        Format::Strike => toggle("~~", "~~", "text"),
        Format::InlineCode => {
            let ticks = "`".repeat(max_ticks(selected) + 1);
            let pad = if selected.starts_with('`')
                || selected.ends_with('`')
                || (selected.starts_with(' ')
                    && selected.ends_with(' ')
                    && !selected.trim().is_empty())
            {
                " "
            } else {
                ""
            };
            wrap(&format!("{ticks}{pad}"), &format!("{pad}{ticks}"), "code")
        }
        Format::Link => {
            let mut result = wrap("[", "](https://example.com)", "link text");
            let url = result.range.start + result.replacement.len() - "https://example.com)".len();
            result.selection = url..(url + "https://example.com".len());
            result
        }
        Format::Image => wrap("![", "](image.png)", "image description"),
        Format::CodeBlock => {
            let ticks = "`".repeat((max_ticks(selected) + 1).max(3));
            let code = if selected.is_empty() {
                "code"
            } else {
                selected
            };
            let closing_newline = if code.ends_with('\n') { "" } else { newline };
            let mut result = block(format!(
                "{ticks}{newline}{code}{closing_newline}{ticks}{newline}"
            ));
            let start = result.selection.start + ticks.len() + newline.len();
            result.selection = start..(start + code.len());
            result
        }
        Format::Table => block(format!(
            "| Column 1 | Column 2 |{newline}| --- | --- |{newline}| Value | Value |{newline}"
        )),
        Format::Heading | Format::Bullet | Format::Numbered | Format::Task | Format::Quote => {
            let start = source[..selection.start].rfind('\n').map_or(0, |n| n + 1);
            let end_from = if !selection.is_empty() && source.as_bytes()[selection.end - 1] == b'\n'
            {
                selection.end - 1
            } else {
                selection.end
            };
            let end = source[end_from..]
                .find('\n')
                .map_or(source.len(), |n| end_from + n);
            // Keep a CR from the final line's CRLF outside the replacement.
            let end = if end > start && source.as_bytes()[end - 1] == b'\r' {
                end - 1
            } else {
                end
            };
            let original = &source[start..end];
            let prefix = match format {
                Format::Heading => "## ",
                Format::Bullet => "- ",
                Format::Task => "- [ ] ",
                Format::Quote => "> ",
                _ => "",
            };
            let toggle = !prefix.is_empty()
                && !original.is_empty()
                && original
                    .split_inclusive('\n')
                    .all(|line| line.starts_with(prefix));
            let mut replacement = String::new();
            for (index, line) in original.split('\n').enumerate() {
                if index > 0 {
                    replacement.push('\n');
                }
                if toggle {
                    replacement.push_str(&line[prefix.len()..]);
                } else {
                    if matches!(format, Format::Numbered) {
                        replacement.push_str(&format!("{}. ", index + 1));
                    } else {
                        replacement.push_str(prefix);
                    }
                    let heading = line.bytes().take_while(|b| *b == b'#').count();
                    let content = if matches!(format, Format::Heading)
                        && (1..=6).contains(&heading)
                        && line.as_bytes().get(heading) == Some(&b' ')
                    {
                        &line[heading + 1..]
                    } else {
                        line
                    };
                    replacement.push_str(content);
                }
            }
            let selection = start..start + replacement.len();
            Edit {
                range: start..end,
                replacement,
                selection,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{Format, edit};

    fn apply(source: &str, range: std::ops::Range<usize>, format: Format) -> String {
        let e = edit(source, range, format).unwrap();
        let mut value = source.to_string();
        value.replace_range(e.range, &e.replacement);
        value
    }

    #[test]
    fn wraps_unicode_selection_and_preserves_unrelated_unknown_syntax() {
        let source = "---\ncustom: true\n---\nżółw\n<Widget />";
        let start = source.find('ż').unwrap();
        assert_eq!(
            apply(source, start..start + "żółw".len(), Format::Bold),
            "---\ncustom: true\n---\n**żółw**\n<Widget />"
        );
        assert!(edit(source, start + 1..start + 2, Format::Bold).is_err());
        assert!(edit(source, 0..source.len() + 1, Format::Bold).is_err());
    }

    #[test]
    fn empty_selection_inserts_and_selects_a_placeholder_or_link_destination() {
        let e = edit("", 0..0, Format::Bold).unwrap();
        assert_eq!(e.replacement, "**bold text**");
        assert_eq!(&e.replacement[e.selection], "bold text");
        let e = edit("word", 0..4, Format::Link).unwrap();
        assert_eq!(&e.replacement[e.selection], "https://example.com");
    }

    #[test]
    fn line_actions_do_not_include_a_next_line_start_or_normalize_crlf() {
        assert_eq!(
            apply("one\ntwo\nthree", 0..4, Format::Bullet),
            "- one\ntwo\nthree"
        );
        assert_eq!(
            apply("one\r\ntwo\r\n", 0..10, Format::Quote),
            "> one\r\n> two\r\n"
        );
        assert_eq!(apply("- one\n- two", 0..11, Format::Bullet), "one\ntwo");
        assert_eq!(apply("one\ntwo", 0..7, Format::Numbered), "1. one\n2. two");
        assert_eq!(apply("", 0..0, Format::Task), "- [ ] ");
        assert_eq!(apply("title", 0..5, Format::Heading), "## title");
        assert_eq!(apply("# title", 0..7, Format::Heading), "## title");
        assert_eq!(apply("## title", 0..8, Format::Heading), "title");
        assert_eq!(apply("żółw", 0.."żółw".len(), Format::Bullet), "- żółw");
        assert_eq!(apply("**żółw**", 2..2 + "żółw".len(), Format::Bold), "żółw");
    }

    #[test]
    fn code_delimiters_cannot_be_closed_by_the_selected_text() {
        assert_eq!(apply("a`b", 0..3, Format::InlineCode), "``a`b``");
        assert_eq!(apply("`x`", 0..3, Format::InlineCode), "`` `x` ``");
        assert_eq!(
            apply("```\nx", 0..5, Format::CodeBlock),
            "````\n```\nx\n````\n"
        );
        assert_eq!(
            apply("a\r\nb", 0..4, Format::CodeBlock),
            "```\r\na\r\nb\r\n```\r\n"
        );
    }
}
