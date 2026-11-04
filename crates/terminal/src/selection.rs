//! Screen-coordinate selection used by the terminal UI.
//!
//! Coordinates are absolute screen lines rather than viewport rows so a
//! selection survives scrolling and scrollback changes. Line numbers are
//! expressed as `i64` because the visible viewport can be above line zero while
//! the terminal is still being resized.

/// One cell in the terminal's absolute line space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub line: i64,
    pub col: u16,
}

impl Point {
    pub fn new(line: i64, col: u16) -> Self {
        Self { line, col }
    }
}

/// An anchor and a moving head. The pair is normalized only when read so
/// dragging through the anchor keeps the original direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Point,
    pub head: Point,
}

impl Selection {
    pub fn new(anchor: Point, head: Point) -> Self {
        Self { anchor, head }
    }

    /// Select one full line.
    pub fn line(line: i64, cols: u16) -> Self {
        let cols = cols.max(1);
        Self::new(Point::new(line, 0), Point::new(line, cols - 1))
    }

    pub fn normalized(self) -> (Point, Point) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    pub fn start(self) -> Point {
        self.normalized().0
    }

    pub fn end(self) -> Point {
        self.normalized().1
    }

    /// A plain click leaves the anchor and head at the same cell. Callers use
    /// this to fall back to the whole buffer instead of copying one character.
    pub fn is_empty(self) -> bool {
        self.anchor == self.head
    }

    pub fn contains(self, line: i64, col: u16) -> bool {
        if self.is_empty() {
            return false;
        }
        let (start, end) = self.normalized();
        Point::new(line, col) >= start && Point::new(line, col) <= end
    }

    /// Inclusive column range selected on `line`, if any.
    pub fn columns_on(self, line: i64, cols: u16) -> Option<(u16, u16)> {
        if self.is_empty() || cols == 0 {
            return None;
        }
        let (start, end) = self.normalized();
        if line < start.line || line > end.line {
            return None;
        }
        let last = cols - 1;
        let first_col = if line == start.line {
            start.col.min(last)
        } else {
            0
        };
        let last_col = if line == end.line {
            end.col.min(last)
        } else {
            last
        };
        (first_col <= last_col).then_some((first_col, last_col))
    }

    /// Extend the head while preserving the original anchor.
    pub fn extend(mut self, head: Point) -> Self {
        self.head = head;
        self
    }
}

/// A terminal line containing only word characters, used for double-click.
/// `cells` is indexed by terminal column, so wide continuations should already
/// be represented by spaces. The returned range is inclusive.
pub fn word_bounds(cells: &[char], clicked_col: u16) -> Option<(u16, u16)> {
    if cells.is_empty() {
        return None;
    }
    let index = usize::from(clicked_col).min(cells.len() - 1);
    let is_word = |ch: char| ch.is_alphanumeric() || "_-./:@~+=?&%$#".contains(ch);
    if !is_word(cells[index]) {
        // Clicking a run of spaces selects that run; clicking other punctuation
        // falls back to the cell itself.
        let target = cells[index];
        if !target.is_whitespace() {
            return Some((index as u16, index as u16));
        }
    }
    let predicate: fn(char) -> bool = if is_word(cells[index]) {
        is_word
    } else {
        char::is_whitespace
    };
    let mut start = index;
    while start > 0 && predicate(cells[start - 1]) {
        start -= 1;
    }
    let mut end = index;
    while end + 1 < cells.len() && predicate(cells[end + 1]) {
        end += 1;
    }
    Some((start as u16, end as u16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_normalizes_and_reports_line_ranges() {
        let selection = Selection::new(Point::new(4, 9), Point::new(2, 3));
        assert_eq!(selection.start(), Point::new(2, 3));
        assert_eq!(selection.end(), Point::new(4, 9));
        assert_eq!(selection.columns_on(2, 10), Some((3, 9)));
        assert_eq!(selection.columns_on(3, 10), Some((0, 9)));
        assert_eq!(selection.columns_on(4, 10), Some((0, 9)));
        assert_eq!(selection.columns_on(5, 10), None);
        assert!(selection.contains(3, 0));
        assert!(!selection.contains(1, 9));
    }

    #[test]
    fn empty_selection_is_not_a_copy_range() {
        let selection = Selection::new(Point::new(1, 1), Point::new(1, 1));
        assert!(selection.is_empty());
        assert_eq!(selection.columns_on(1, 20), None);
        assert!(!selection.contains(1, 1));
    }

    #[test]
    fn word_bounds_cover_unicode_words_and_paths() {
        let text: Vec<char> = "run src/main.rs --flag".chars().collect();
        let (start, end) = word_bounds(&text, 6).unwrap();
        assert_eq!(
            text[start as usize..=end as usize]
                .iter()
                .collect::<String>(),
            "src/main.rs"
        );
        let (start, end) = word_bounds(&text, 0).unwrap();
        assert_eq!(
            text[start as usize..=end as usize]
                .iter()
                .collect::<String>(),
            "run"
        );
        let unicode: Vec<char> = "héllo".chars().collect();
        let (start, end) = word_bounds(&unicode, 1).unwrap();
        assert_eq!(
            unicode[start as usize..=end as usize]
                .iter()
                .collect::<String>(),
            "héllo"
        );
        assert_eq!(word_bounds(&[], 0), None);
    }

    #[test]
    fn word_click_on_spaces_selects_the_space_run() {
        let text: Vec<char> = "a   b".chars().collect();
        let (start, end) = word_bounds(&text, 2).unwrap();
        assert_eq!((start, end), (1, 3));
    }
}
