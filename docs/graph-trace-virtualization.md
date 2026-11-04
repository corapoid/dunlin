# Graph / Trace auto-layout, virtualization and focus QA

Status: implementation in the working tree; automated invariants are covered.
Manual VoiceOver/keyboard acceptance is still required and is listed below.
This work does not change the recorded transcript, ACP events, costs or token
reporting.

## Goal

Close the remaining Graph/Trace gap without pretending that a large graph is a
small one:

- deterministic tidy-tree auto-layout that handles arbitrary depth and a 10k+
  visible forest without recursion or quadratic parent scans;
- full rendering virtualization for the graph viewport, the minimap and the
  trace waterfall;
- keyboard selection that stays on a real row, scrolls the selected row into
  view, and exposes tree semantics to assistive technology;
- automated fixtures that make the 10k claim falsifiable, plus an explicit
  manual focus/a11y matrix.

## Implementation plan

1. **Layout**
   - Replace the depth-capped repeated-scan layout with a linear tidy-tree pass.
   - Index parents once, detach malformed/cyclic links, pack variable-width
     subtrees in input order and centre a parent over its children.
   - Compute depth-local row heights so variable card heights never overlap.
   - Keep positions deterministic for the same visible set, so streaming does
     not shuffle already-settled branches.
2. **Graph virtualization**
   - Keep the full laid-out graph for size/SVG export, but create GPUI elements
     only for nodes and edges intersecting the scroll viewport plus overscan.
   - Scale geometry uniformly at every zoom step; removing per-card minimum
     clamps prevents cards and hit targets overlapping at 50%.
   - Aggregate the minimap into a bounded status-cell grid instead of one
     element per node, and draw the current viewport outline.
   - Re-render on wheel scroll so the culled slice follows the viewport.
3. **Trace virtualization**
   - Return row indices from the waterfall builder instead of cloning every
     span.
   - Render rows through `uniform_list`, which asks for only the visible range
     and uses the stored `UniformListScrollHandle` for keyboard reveal.
   - Keep the sticky axis/header outside the virtual list.
4. **Focus and accessibility**
   - One tab-reachable composite focus handle for the active Graph/Trace tree.
   - Tree/tree-item roles, labels, descriptions, levels, sibling positions,
     selected/expanded state and `aria-activedescendant` on the selected row.
   - Arrow/Home/End/PageUp/PageDown/Enter/Space/Escape handling; Left collapses
     then moves to the parent, Right expands then enters the first child.
   - Normalize selection after search, filtering or collapse and scroll the
     selected graph node/trace row into view.
   - Buttons, filters, minimap and detail actions carry explicit labels.
5. **Verification**
   - Unit fixtures build 10k-node chains, a 10k-wide fan-out and a 10k-row trace
     to exercise layout, culling, minimap aggregation and row indexing.
   - Focus invariants are tested as pure selection/normalization helpers.
   - Manual matrix below remains separate from the automated pass.

## Automated coverage

The desktop test module `apps/desktop/src/activity.rs` covers:

- 10,001-node deep chain: every node laid out, finite geometry, no overlap
  between levels and no recursion depth limit;
- 10,001-node forest: viewport culling returns fewer than 100 elements for a
  1200 × 800 viewport;
- minimap aggregation stays bounded and keeps the worst status in a cell;
- tree metadata exposes levels, sibling position/count and expandability;
- node/span ARIA labels include status, keep unknown durations explicit and
  bound long task text;
- selection moves clamp at both ends and normalize to the first real row when
  the previous target disappears;
- Fit zoom is bounded and never returns NaN;
- 10,001-row trace waterfall returns indices in request order without cloning
  each span.

The normal gate remains:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p dunlin -p dunlin-relay
```

## Manual focus/a11y QA

Run the packaged app with both themes and the narrowest supported window.

| # | Case | Expected |
| --- | --- | --- |
| G1 | Tab into Graph; use Up/Down/Home/End/PageUp/PageDown | Selection moves, is visible, and the graph scrolls to it; the search box keeps focus when it owns focus. |
| G2 | Right on an agent, then Right again; Left twice | First Right expands; second enters the first child; Left collapses or moves to the parent without losing the tree focus. |
| G3 | Search hides the selected node | Selection moves to the first visible match (or clears when there is none); no aria-activedescendant points at an unrendered node. |
| G4 | VoiceOver on Graph | The tree is announced with the graph label; each card exposes title, kind, status, duration and model/task detail; selected/expanded states and levels are spoken. |
| G5 | Scroll and zoom Graph; press Fit | Only nearby cards appear; no overlap at 50%; the minimap viewport outline follows the scroll; Fit centres content without clearing selection. |
| G6 | Trace: Up/Down/Home/End/PageUp/PageDown | The 10k row fixture scrolls only the needed rows; selection stays visible and the sticky axis remains aligned. |
| G7 | Trace filters, Requests, Expand all | Filter changes keep a valid selection; collapsing a selected request keeps it visible; details and buttons have meaningful names. |
| G8 | Reduce Motion, light/dark, narrow window | Entrance/pulse animations disappear, colors remain distinguishable, labels do not clip and focus is never trapped. |

Record PASS/FAIL/DEFERRED with build, commit, macOS version, display size and
VoiceOver version. A green test run is not evidence for VoiceOver or TCC.
