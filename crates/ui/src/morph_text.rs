//! In-place transitions for short metadata, never document or streamed text.
use gpui_kit::base::StyledExt as _;
use gpui_kit::*;
use std::time::{Duration, Instant};
use unicode_segmentation::UnicodeSegmentation;

const DURATION: Duration = Duration::from_millis(220);
const MAX_GRAPHEMES: usize = 96;

/// A single-line, accessible native text transition. The first value is static.
/// Keep `id` stable and scope values to their session, repository or account.
#[derive(IntoElement)]
pub struct MorphText {
    id: ElementId,
    value: SharedString,
    scope: SharedString,
    style: StyleRefinement,
    reduce_motion: bool,
    numeric: bool,
}

impl MorphText {
    pub fn new(id: impl Into<ElementId>, value: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            value: value.into(),
            scope: SharedString::default(),
            style: StyleRefinement::default(),
            reduce_motion: false,
            numeric: false,
        }
    }

    pub fn scope(mut self, scope: impl Into<SharedString>) -> Self {
        self.scope = scope.into();
        self
    }

    /// Match digits by place rather than matching a digit in a different place.
    pub fn numeric(mut self) -> Self {
        self.numeric = true;
        self
    }

    pub fn reduce_motion(mut self, reduce_motion: bool) -> Self {
        self.reduce_motion = reduce_motion;
        self
    }
}

impl Styled for MorphText {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

struct Motion {
    scope: SharedString,
    value: SharedString,
    from: Option<SharedString>,
    started: Instant,
}

impl Motion {
    fn new(scope: SharedString, value: SharedString, now: Instant) -> Self {
        Self {
            scope,
            value,
            from: None,
            started: now,
        }
    }

    fn update(&mut self, scope: &SharedString, value: &SharedString, now: Instant, reduced: bool) {
        if reduced || self.scope != *scope {
            *self = Self::new(scope.clone(), value.clone(), now);
            return;
        }
        if now.duration_since(self.started) >= DURATION {
            self.from = None;
        }
        // Finish the current pair, then coalesce all intervening updates into
        // the latest value. There are never more than two copies of a glyph.
        if self.from.is_none() && self.value != *value {
            self.from = Some(std::mem::replace(&mut self.value, value.clone()));
            self.started = now;
        }
    }

    fn progress(&self, now: Instant) -> f32 {
        let t = (now.duration_since(self.started).as_secs_f32() / DURATION.as_secs_f32()).min(1.);
        1. - (1. - t).powi(3)
    }
}

/// Pairs of surviving graphemes, in logical order. Numeric runs align at the
/// decimal point (or the right edge for integers); punctuation stays with them.
fn matches(old: &[&str], new: &[&str], numeric: bool) -> Vec<(usize, usize)> {
    if numeric {
        let runs = |items: &[&str]| {
            let mut result = Vec::new();
            let mut i = 0;
            while i < items.len() {
                if items[i].as_bytes().first().is_some_and(u8::is_ascii_digit) {
                    let start = i;
                    i += 1;
                    while i < items.len()
                        && (items[i] == "."
                            || items[i] == ","
                            || items[i].as_bytes().first().is_some_and(u8::is_ascii_digit))
                    {
                        i += 1;
                    }
                    result.push(start..i);
                } else {
                    i += 1;
                }
            }
            result
        };
        let old_runs = runs(old);
        let new_runs = runs(new);
        if old_runs.is_empty() || old_runs.len() != new_runs.len() {
            return matches(old, new, false);
        }
        let anchor = |items: &[&str]| {
            items.iter().position(|s| *s == ".").unwrap_or_else(|| {
                items
                    .iter()
                    .rposition(|s| s.as_bytes().first().is_some_and(u8::is_ascii_digit))
                    .map_or(items.len(), |i| i + 1)
            })
        };
        let (mut old_end, mut new_end) = (0, 0);
        let mut result = Vec::new();
        for (a, b) in old_runs.iter().zip(&new_runs) {
            result.extend(
                matches(&old[old_end..a.start], &new[new_end..b.start], false)
                    .into_iter()
                    .map(|(i, j)| (i + old_end, j + new_end)),
            );
            let offset = anchor(&new[b.clone()]) as isize - anchor(&old[a.clone()]) as isize;
            result.extend(old[a.clone()].iter().enumerate().filter_map(|(i, value)| {
                let j = i as isize + offset;
                (j >= 0 && new[b.clone()].get(j as usize) == Some(value))
                    .then(|| (i + a.start, j as usize + b.start))
            }));
            old_end = a.end;
            new_end = b.end;
        }
        result.extend(
            matches(&old[old_end..], &new[new_end..], false)
                .into_iter()
                .map(|(i, j)| (i + old_end, j + new_end)),
        );
        return result;
    }
    let mut lengths = vec![vec![0; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lengths[i][j] = if old[i] == new[j] {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut result = Vec::new();
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            result.push((i, j));
            i += 1;
            j += 1;
        } else if lengths[i + 1][j] > lengths[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    result
}

fn shape(value: SharedString, style: &TextStyle, alpha: f32, window: &Window) -> ShapedLine {
    let mut run = style.to_run(value.len());
    run.color = run.color.opacity(alpha);
    window.text_system().shape_line(
        value,
        style.font_size.to_pixels(window.rem_size()),
        &[run],
        None,
    )
}

/// Split an already-shaped line, preserving kerning. If a grapheme boundary
/// crosses a ligature or visual order changes (RTL), animate the whole line
/// instead; never split a shaped glyph or corrupt a complex-script label.
fn pieces(line: &ShapedLine) -> Option<Vec<(Pixels, ShapedLine)>> {
    let indices: Vec<_> = line
        .runs
        .iter()
        .flat_map(|run| run.glyphs.iter().map(|g| g.index))
        .collect();
    if !indices.is_sorted() {
        return None;
    }
    let mut cursor = line.cursor();
    let mut result = Vec::new();
    for (start, grapheme) in line.text.grapheme_indices(true) {
        let end = start + grapheme.len();
        if end != line.len() && !indices.contains(&end) {
            return None;
        }
        let x = cursor.x_offset();
        result.push((x, cursor.take_until(end)));
    }
    Some(result)
}

impl RenderOnce for MorphText {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let reduced = self.reduce_motion || cx.reduce_motion() || self.value.contains(['\n', '\r']);
        let now = Instant::now();
        let state = window.use_keyed_state((self.id.clone(), "morph"), cx, |_, _| {
            Motion::new(self.scope.clone(), self.value.clone(), now)
        });
        let (from, value, progress) = state.update(cx, |state, _| {
            state.update(
                &self.scope,
                &self.value,
                now,
                reduced || state.value.contains(['\n', '\r']),
            );
            (state.from.clone(), state.value.clone(), state.progress(now))
        });
        let root = div()
            .id(self.id)
            .relative()
            .flex_shrink_0()
            .whitespace_nowrap()
            .role(Role::Label)
            .aria_label(self.value.clone())
            .refine_style(&self.style);
        // This primitive deliberately does not own paragraphs or wrapping.
        if self.value.contains(['\n', '\r']) {
            return root.child(self.value).into_any_element();
        }
        let style =
            window.with_text_style(Some(self.style.text.clone()), |window| window.text_style());
        let height = style.line_height_in_pixels(window.rem_size());
        let target = shape(value, &style, 1., window);
        let Some(from) = from else {
            let width = target.width().ceil();
            return root
                .w(width)
                .h(height)
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, cx| {
                            let _ = target.paint(
                                bounds.origin,
                                height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        },
                    )
                    .size_full(),
                )
                .into_any_element();
        };
        let previous = shape(from, &style, 1., window);
        // Match GPUI's ordinary text layout rounding at both endpoints.
        let width =
            previous.width().ceil() + (target.width().ceil() - previous.width().ceil()) * progress;
        let old: Vec<_> = previous.text.graphemes(true).collect();
        let new: Vec<_> = target.text.graphemes(true).collect();
        let granular = old.len().max(new.len()) <= MAX_GRAPHEMES;
        let paired = if granular {
            matches(&old, &new, self.numeric)
        } else {
            Vec::new()
        };
        let outgoing = shape(previous.text.clone(), &style, 1. - progress, window);
        let incoming = shape(target.text.clone(), &style, progress, window);
        let fragments = granular
            .then(|| {
                Some((
                    pieces(&previous)?,
                    pieces(&target)?,
                    pieces(&outgoing)?,
                    pieces(&incoming)?,
                ))
            })
            .flatten();
        // Only live transitions request frames. No timers or idle polling.
        window.request_animation_frame();
        root.w(width)
            .h(height)
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            let travel = height * 0.45;
                            let paint = |line: &ShapedLine,
                                         x: Pixels,
                                         y: Pixels,
                                         window: &mut Window,
                                         cx: &mut App| {
                                let _ = line.paint(
                                    bounds.origin + point(x, y),
                                    height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            };
                            if let Some((old, new, outgoing, incoming)) = fragments {
                                for &(i, j) in &paired {
                                    paint(
                                        &new[j].1,
                                        old[i].0 + (new[j].0 - old[i].0) * progress,
                                        px(0.),
                                        window,
                                        cx,
                                    );
                                }
                                for (i, (x, line)) in outgoing.iter().enumerate() {
                                    if !paired.iter().any(|&(old, _)| old == i) {
                                        paint(line, *x, -travel * progress, window, cx);
                                    }
                                }
                                for (j, (x, line)) in incoming.iter().enumerate() {
                                    if !paired.iter().any(|&(_, new)| new == j) {
                                        paint(line, *x, travel * (1. - progress), window, cx);
                                    }
                                }
                            } else {
                                paint(&outgoing, px(0.), -travel * progress, window, cx);
                                paint(&incoming, px(0.), travel * (1. - progress), window, cx);
                            }
                        });
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paired(old: &str, new: &str, numeric: bool) -> Vec<(usize, usize)> {
        matches(
            &old.graphemes(true).collect::<Vec<_>>(),
            &new.graphemes(true).collect::<Vec<_>>(),
            numeric,
        )
    }

    #[test]
    fn first_value_and_unchanged_values_are_static() {
        let now = Instant::now();
        let mut state = Motion::new("session".into(), "10".into(), now);
        state.update(&"session".into(), &"10".into(), now + DURATION, false);
        assert!(state.from.is_none());
    }

    #[test]
    fn transition_finishes_and_stops_requesting_frames() {
        let now = Instant::now();
        let mut state = Motion::new("s".into(), "10".into(), now);
        state.update(&"s".into(), &"11".into(), now, false);
        assert_eq!(state.from.as_deref(), Some("10"));
        assert_eq!(state.progress(now), 0.);
        state.update(&"s".into(), &"11".into(), now + DURATION, false);
        assert!(state.from.is_none());
        assert_eq!(state.progress(now + DURATION), 1.);
    }

    #[test]
    fn rapid_updates_coalesce_without_adding_ghosts_or_restarting() {
        let now = Instant::now();
        let mut state = Motion::new("s".into(), "0".into(), now);
        state.update(&"s".into(), &"1".into(), now, false);
        for i in 2..100 {
            state.update(
                &"s".into(),
                &i.to_string().into(),
                now + Duration::from_millis(i),
                false,
            );
        }
        assert_eq!(state.value.as_ref(), "1");
        assert_eq!(state.from.as_deref(), Some("0"));
        assert_eq!(state.started, now);
        state.update(&"s".into(), &"99".into(), now + DURATION, false);
        assert_eq!(state.value.as_ref(), "99");
        assert_eq!(state.from.as_deref(), Some("1"));
    }

    #[test]
    fn scope_changes_and_reduced_motion_snap_immediately() {
        let now = Instant::now();
        let mut state = Motion::new("a".into(), "1".into(), now);
        state.update(&"a".into(), &"2".into(), now, false);
        state.update(&"b".into(), &"500".into(), now, false);
        assert!(state.from.is_none());
        assert_eq!(state.value.as_ref(), "500");
        state.update(&"b".into(), &"501".into(), now, false);
        state.update(&"b".into(), &"502".into(), now, true);
        assert!(state.from.is_none());
        assert_eq!(state.value.as_ref(), "502");
    }

    #[test]
    fn digits_match_by_place_and_decimal_point() {
        assert_eq!(paired("101", "110", true), vec![(0, 0)]);
        assert_eq!(paired("9", "19", true), vec![(0, 1)]);
        assert_eq!(
            paired("$9.99", "$10.99", true),
            vec![(0, 0), (2, 3), (3, 4), (4, 5)]
        );
        assert_eq!(paired("6.6k", "6.8k", true), vec![(0, 0), (1, 1), (3, 3)]);
    }

    #[test]
    fn labels_preserve_surviving_graphemes_including_emoji() {
        assert_eq!(paired("RUNNING", "DONE", false), vec![(2, 2)]);
        assert_eq!(
            paired("a👩‍💻e\u{301}", "b👩‍💻e\u{301}", false),
            vec![(1, 1), (2, 2)]
        );
        assert!(paired("", "DONE", false).is_empty());
        assert!(paired("99", "100", true).is_empty());
    }

    #[test]
    fn numeric_labels_keep_prefixes_and_each_number_in_place() {
        let old = "Account 9%";
        let new = "Account 10%";
        let matches = paired(old, new, true);
        assert_eq!(&matches[..8], &(0..8).map(|i| (i, i)).collect::<Vec<_>>());
        assert_eq!(matches.last(), Some(&(9, 10)));
        let matches = paired("9 / 200 context", "10 / 200 context", true);
        assert!(
            matches.contains(&(4, 5)),
            "the capacity must not roll with the used count"
        );
        assert_eq!(paired("Account —", "Account 10%", true).len(), 8);
    }
}
