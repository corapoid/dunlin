# Native metadata text motion

`dunlin_ui::MorphText` animates short, single-line labels in GPUI. No JavaScript
or webview is involved. It is intentionally not used for streaming answers,
reasoning bodies, code, diffs, terminal output or editable text.

Enabled surfaces:

- Status bar: session cost, account allowance and context/token metadata.
- Chat: tool status and the composer prompt-queue count.
- Graph and Trace: individual summary counts, costs, tokens and duration.
- Git: total, staged and working-tree change counts.
- Account usage popover: percentages and matching 220 ms progress-bar motion.

Use a stable ID and a scope identifying the session, repository or provider:

```rust,ignore
MorphText::new("session-cost", format!("${cost:.2}"))
    .numeric()
    .scope(session.id.clone())
    .reduce_motion(app.reduce_motion)
```

First values and scope changes render immediately. Subsequent changes use a
220 ms cubic ease-out. Matching graphemes remain visible; changed graphemes
slide vertically and fade. Numeric matching aligns digits by place. Width
changes share the same timeline, so neighboring metadata moves with the value.
Fast changes finish the current pair before adopting the latest value, without
stacking outgoing copies. Values are real formatted data, not interpolated or
invented intermediate numbers. Accessibility always exposes the latest value.

Full lines are shaped with the inherited native font, preserving kerning.
Grapheme boundaries respect combining marks and emoji. Ligatures and reordered
scripts fall back to a whole-line transition rather than splitting a glyph.
Long labels avoid the quadratic grapheme diff. Both the application preference
and system Reduce Motion snap directly to the latest value. Settled labels do
not request animation frames or run timers.

## Regression checks

```sh
cargo test -p dunlin-ui --lib morph_text
cargo run -p dunlin-ui --example morph_text_smoke
cargo check -p dunlin
```

The isolated smoke opens its own temporary native window, checks all six label
types against plain native text metrics in both themes, tests rapid updates,
scope changes and both motion preferences, verifies idle rendering stops, and
exits automatically. It does not connect agents or touch application data.
