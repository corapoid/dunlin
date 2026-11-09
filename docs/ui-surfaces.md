# Rounded surfaces and edge ownership

`crates/ui/src/surfaces.rs` is the source of truth for corner tokens, surface
border widths, inner radii and native backdrop measurements. Existing
`scale::RADIUS_*` constants are compatibility aliases to these tokens.

## Rules

1. Use `SurfaceGeometry::{CARD,DIALOG,SETTINGS,SETTINGS_CONTENT,SETTINGS_CARD,
   CHOICE_CARD,AUTH_DIALOG,WORKSPACE,COMPOSER,TABLE,GRAPH_NODE}` for compound surfaces. `paint` sets the
   background, radius and border together. Use `radii::*` for decorative shapes
   and dense controls; use `rounded_full()` for circles/capsules. There are no
   literal corner sizes in application views. Geometry-derived project icons
   and canvas glyphs intentionally scale with their bounds.
2. **GPUI overflow masks are rectangular.** `rounded().overflow_hidden()` does
   not clip the children to the curve. Let the parent own the background whenever
   possible (Settings navigation is transparent). If a child paints up to an
   exposed edge, use `geometry.round_child(child, SurfaceEdges::Top/Bottom/...)`.
   Inner radius is `max(outer_radius - border_width, 0)`. Internal joins stay square.
3. Use `settings::card_header`, `code_header` and `card_action_row` rather than a
   square header fill or an ordinary small-radius button at a card edge. Hover,
   selected and focus states follow the same geometry. Expanded action cards round
   only their top edge. Theme previews use the choice card's inner radius too.
4. Editors, terminals, tabs and split joins retain their own layout. Do not add
   global padding or globally round every tab. Opaque gutter/selection paint must
   remain inside the curve; the editor font preview reserves only its bottom edge.
   Preserve the first selected folder tab's intentionally square top-left join.
   Thin graph-node progress indicators use `bottom_indicator` to stay between
   corner centres, while their headers use the node's inner top curves. List
   separators belong between rows, not across a rounded card's bottom edge.
5. Native overlays use `effects::panel_probe(backdrop, geometry, scroll)`, with the
   **same geometry** as their GPUI frame. It restores the measured border to obtain
   the complete outer bounds. Scroll probes follow the fixed viewport, not the
   translated child. No separate radius or guessed one-pixel expansion at call sites.
6. Use `segmented_bar` for multi-colour capsules. Each colour paints the same
   rounded shape through a rectangular segment mask, so even tiny first/last
   segments cannot square off the ends.
   Shared `progress_bar` keeps rounded moving caps and clips sub-height fills to
   the track's curve. Sidebar, account and settings meters use this helper too.

## Regression coverage

```sh
cargo test -p dunlin-ui --lib
cargo test -p dunlin ui_contract_tests
cargo build -p dunlin
target/debug/dunlin --surfaces-smoke
target/debug/dunlin --settings-smoke /absolute/path/dunlin-settings-smoke-unique
python3 scripts/release_qa.py --native-only
```

The new surface scenario is registered in the desktop native-smoke registry and
therefore runs in the existing CI/release native suite. It uses no application
database, credentials, agents or network. It covers both palettes, both densities,
two window widths, fractional layout coordinates, hover/focus, expanded/selected
actions, scrolling, tiny bar segments and a negative control reproducing the old
Settings navigation bug. The negative control must fail the paint checker.

Settings acceptance checks all 15 pages in both palettes/densities at two sizes,
including the external shell and every visible badge-sized or larger rounded painted surface in the
content viewport. Existing workspace/Git tests use the same checker for reading
surfaces, editor gutters, selected tab joins and dock layouts. The checker samples
a grid across each curve and respects device-pixel snapping and scroll masks.
Deferred tooltips/popovers escaping both the surface bounds and its clip are
independent overlays, not child bleed; they are checked on their own surfaces.
Geometry unit tests also cover 1×, 1.25×, 1.5×, 2× and 3× coordinate transforms;
native backdrop tests cover AppKit masks/scales separately.

Scene assertions are not screenshots: they catch quad fills and solid border bleed, but do not
prove font/image rasterization or native material compositing on every display.
For visual acceptance also inspect light/dark mode, Reduce Transparency, full
screen, window drag/resize and moving between 1×/2× displays on macOS.
