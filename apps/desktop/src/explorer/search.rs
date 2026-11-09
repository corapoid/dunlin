//! Filename-only speed search over the entire scanned tree, including collapsed folders.
use dunlin_workspace::tree::{FileEntry, FileTree};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Match {
    pub path: String,
    pub directory: bool,
}

#[derive(Default)]
pub(crate) struct Search {
    pub query: String,
    pub matches: Vec<Match>,
    pub index: Option<usize>,
    expanded_before: Option<Vec<String>>,
}

impl Search {
    pub fn current(&self) -> Option<&Match> {
        self.index.and_then(|index| self.matches.get(index))
    }

    pub fn set_query(&mut self, query: String, tree: &mut FileTree) {
        if query.is_empty() {
            self.clear(tree);
            return;
        }
        if self.expanded_before.is_none() {
            self.expanded_before = Some(tree.expanded_paths());
        }
        self.query = query;
        self.refresh(tree);
    }

    pub fn clear(&mut self, tree: &mut FileTree) {
        if let Some(expanded) = self.expanded_before.take() {
            tree.restore_expanded(&expanded);
        }
        self.query.clear();
        self.matches.clear();
        self.index = None;
    }

    pub fn backspace(&mut self, tree: &mut FileTree) {
        let end = self
            .query
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index);
        self.set_query(self.query[..end].to_string(), tree);
    }

    /// Reapply after a query edit or an asynchronous local/remote tree refresh.
    pub fn refresh(&mut self, tree: &mut FileTree) {
        let Some(expanded) = &self.expanded_before else {
            return;
        };
        tree.restore_expanded(expanded);
        let selected = self.current().map(|entry| entry.path.clone());
        self.matches.clear();
        let query = fold_query(&self.query);
        collect_matches(&mut tree.entries, &query, &mut self.matches);
        self.index = selected
            .and_then(|path| self.matches.iter().position(|entry| entry.path == path))
            .or_else(|| (!self.matches.is_empty()).then_some(0));
    }

    pub fn move_selection(&mut self, direction: isize) {
        if self.matches.is_empty() {
            self.index = None;
            return;
        }
        self.index = Some(match self.index {
            Some(index) => {
                (index as isize + direction).rem_euclid(self.matches.len() as isize) as usize
            }
            None if direction < 0 => self.matches.len() - 1,
            None => 0,
        });
    }

    pub fn select_path(&mut self, path: &str) {
        self.index = self.matches.iter().position(|entry| entry.path == path);
    }

    /// A folder may have been collapsed manually since the last query edit.
    pub fn reveal_current(&self, tree: &mut FileTree) -> Option<usize> {
        let path = &self.current()?.path;
        reveal_ancestors(&mut tree.entries, path);
        visible_index(&tree.entries, path, &mut 0)
    }
}

fn collect_matches(entries: &mut [FileEntry], query: &str, matches: &mut Vec<Match>) -> bool {
    let mut any = false;
    for entry in entries {
        let matched = fold_query(&entry.name).contains(query);
        if matched {
            matches.push(Match {
                path: entry.path.clone(),
                directory: entry.is_dir,
            });
        }
        let descendant = collect_matches(&mut entry.children, query, matches);
        if descendant {
            entry.expanded = true;
        }
        any |= matched || descendant;
    }
    any
}

fn reveal_ancestors(entries: &mut [FileEntry], path: &str) -> bool {
    for entry in entries {
        if entry.path == path {
            return true;
        }
        if reveal_ancestors(&mut entry.children, path) {
            entry.expanded = true;
            return true;
        }
    }
    false
}

fn visible_index(entries: &[FileEntry], path: &str, index: &mut usize) -> Option<usize> {
    for entry in entries {
        if entry.path == path {
            return Some(*index);
        }
        *index += 1;
        if entry.is_dir && entry.expanded {
            if let Some(found) = visible_index(&entry.children, path, index) {
                return Some(found);
            }
        }
    }
    None
}

/// Return original UTF-8 byte ranges, even when lowercasing changes byte length
/// or expands a character (e.g. İ). Rendering and navigation share this matcher.
pub(crate) fn match_ranges(name: &str, query: &str) -> Vec<Range<usize>> {
    folded_match_ranges(name, &fold_query(query))
}

fn fold_query(query: &str) -> String {
    query.chars().flat_map(char::to_lowercase).collect()
}

fn folded_match_ranges(name: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut folded = String::with_capacity(name.len());
    let mut original = Vec::with_capacity(name.len());
    for (start, ch) in name.char_indices() {
        for lower in ch.to_lowercase() {
            original.extend(std::iter::repeat_n(
                start..start + ch.len_utf8(),
                lower.len_utf8(),
            ));
            folded.push(lower);
        }
    }
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (start, matched) in folded.match_indices(query) {
        let range = original[start].start..original[start + matched.len() - 1].end;
        if let Some(previous) = ranges
            .last_mut()
            .filter(|previous| previous.end >= range.start)
        {
            previous.end = previous.end.max(range.end);
        } else {
            ranges.push(range);
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, path: &str, children: Vec<FileEntry>) -> FileEntry {
        FileEntry {
            name: name.into(),
            path: path.into(),
            is_dir: !children.is_empty(),
            expanded: false,
            depth: 0,
            size: 0,
            children,
        }
    }

    fn tree() -> FileTree {
        FileTree {
            root: "/project".into(),
            entries: vec![
                entry(
                    "src",
                    "/project/src",
                    vec![entry(
                        "nested",
                        "/project/src/nested",
                        vec![
                            entry("Match.rs", "/project/src/nested/Match.rs", vec![]),
                            entry("other.rs", "/project/src/nested/other.rs", vec![]),
                        ],
                    )],
                ),
                entry("match.txt", "/project/match.txt", vec![]),
                entry("README.md", "/project/README.md", vec![]),
            ],
            error: None,
            total_size: 0,
        }
    }

    #[test]
    fn searches_collapsed_descendants_and_expands_only_ancestors() {
        let mut tree = tree();
        let mut search = Search::default();
        search.set_query("MATCH".into(), &mut tree);
        assert_eq!(search.matches.len(), 2);
        assert_eq!(
            search.current().unwrap().path,
            "/project/src/nested/Match.rs"
        );
        assert_eq!(
            tree.expanded_paths(),
            ["/project/src", "/project/src/nested"]
        );
        assert_eq!(search.reveal_current(&mut tree), Some(2));
        // Non-matching rows are still present, but skipped by keyboard navigation.
        search.move_selection(1);
        assert_eq!(search.current().unwrap().path, "/project/match.txt");
        assert_eq!(search.reveal_current(&mut tree), Some(4));
        search.move_selection(1);
        assert_eq!(search.index, Some(0));
        search.move_selection(-1);
        assert_eq!(search.index, Some(1));
    }

    #[test]
    fn clearing_and_empty_query_restore_original_expansion() {
        let mut tree = tree();
        tree.entries[0].expanded = true;
        let before = tree.expanded_paths();
        let mut search = Search::default();
        search.set_query("match".into(), &mut tree);
        search.set_query("README".into(), &mut tree);
        assert_eq!(tree.expanded_paths(), before);
        search.set_query("match".into(), &mut tree);
        search.set_query(String::new(), &mut tree);
        assert_eq!(tree.expanded_paths(), before);
        assert!(search.matches.is_empty());
        assert!(search.current().is_none());
    }

    #[test]
    fn no_results_does_not_select_or_open_an_unmatched_entry() {
        let mut tree = tree();
        let mut search = Search::default();
        search.set_query("missing".into(), &mut tree);
        search.move_selection(1);
        search.move_selection(-1);
        assert!(search.current().is_none());
        assert_eq!(search.reveal_current(&mut tree), None);
        assert!(tree.expanded_paths().is_empty());
    }

    #[test]
    fn matching_a_folder_does_not_expand_unrelated_children() {
        let mut tree = tree();
        let mut search = Search::default();
        search.set_query("nested".into(), &mut tree);
        assert!(search.current().unwrap().directory);
        assert_eq!(tree.expanded_paths(), ["/project/src"]);
    }

    #[test]
    fn navigation_reveals_a_manually_collapsed_match() {
        let mut tree = tree();
        let mut search = Search::default();
        search.set_query("match".into(), &mut tree);
        tree.collapse_all();
        assert_eq!(search.reveal_current(&mut tree), Some(2));
        assert_eq!(tree.expanded_paths().len(), 2);
    }

    #[test]
    fn rescan_preserves_query_selection_and_original_expansion() {
        let mut initial = tree();
        let mut search = Search::default();
        search.set_query("match".into(), &mut initial);
        search.move_selection(1);
        let mut refreshed = tree();
        search.refresh(&mut refreshed);
        assert_eq!(search.current().unwrap().path, "/project/match.txt");
        assert_eq!(refreshed.expanded_paths().len(), 2);
        refreshed.entries.remove(1);
        search.refresh(&mut refreshed);
        assert_eq!(
            search.current().unwrap().path,
            "/project/src/nested/Match.rs"
        );
        search.clear(&mut refreshed);
        assert!(refreshed.expanded_paths().is_empty());
    }

    #[test]
    fn backspace_removes_a_whole_grapheme_and_restores_tree_when_empty() {
        let mut tree = tree();
        let mut search = Search::default();
        search.set_query("match".into(), &mut tree);
        search.backspace(&mut tree);
        assert_eq!(search.query, "matc");
        assert_eq!(search.matches.len(), 2);
        search.set_query("a\u{301}👩‍💻".into(), &mut tree);
        search.backspace(&mut tree);
        assert_eq!(search.query, "a\u{301}");
        search.backspace(&mut tree);
        assert!(search.query.is_empty());
        assert!(tree.expanded_paths().is_empty());
    }

    #[test]
    fn unmatched_mouse_selection_resumes_at_first_or_last_match() {
        let mut tree = tree();
        let mut search = Search::default();
        search.set_query("match".into(), &mut tree);
        search.select_path("/project/match.txt");
        assert_eq!(search.index, Some(1));
        search.select_path("/project/README.md");
        search.move_selection(1);
        assert_eq!(search.index, Some(0));
        search.select_path("/project/README.md");
        search.move_selection(-1);
        assert_eq!(search.index, Some(1));
    }

    #[test]
    fn refresh_after_escape_does_not_restart_search_or_expand_folders() {
        let mut tree = tree();
        let mut search = Search::default();
        search.set_query("match".into(), &mut tree);
        search.clear(&mut tree);
        search.refresh(&mut tree);
        assert!(search.query.is_empty());
        assert!(search.matches.is_empty());
        assert!(tree.expanded_paths().is_empty());
        // Starting another search takes a new expansion snapshot.
        tree.entries[0].expanded = true;
        search.set_query("match".into(), &mut tree);
        search.clear(&mut tree);
        assert_eq!(tree.expanded_paths(), ["/project/src"]);
    }

    #[test]
    fn highlights_all_occurrences_with_original_unicode_byte_ranges() {
        assert_eq!(match_ranges("Match-MATCH.rs", "match"), [0..5, 6..11]);
        assert_eq!(match_ranges("ŻÓŁĆ.rs", "żółć"), [0..8]);
        assert_eq!(match_ranges("ΟΣ.rs", "ΟΣ"), [0..4]);
        assert_eq!(match_ranges("İx.rs", "x"), [2..3]);
        assert_eq!(match_ranges("İx.rs", "i"), [0..2]);
        assert_eq!(match_ranges("a🦀b.rs", "🦀"), [1..5]);
        assert!(match_ranges("file.rs", "").is_empty());
        assert!(match_ranges("file.rs", "missing").is_empty());
    }
}
