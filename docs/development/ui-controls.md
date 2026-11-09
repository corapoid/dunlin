# Shared UI controls

Ordinary controls come from `dunlin-ui`, not direct kit constructors. They share
the application palette, typography, rounding, focus and interaction states.

Compound cards, panels and native overlays follow the shared
[surface geometry and edge-ownership rules](ui-surfaces.md).

| Purpose | Constructor |
| --- | --- |
| Form field | `input(state)` |
| Dense field | `compact_input(state)` |
| Search | `search_input(state, accessible_name)` |
| Command or argv | `code_input(state)` |
| Multi-line form | `textarea(state)` |
| Chat composer inside its focus-aware card | `composer_input(state)` |
| Action / toolbar icon | `button` / `icon_button` |
| Exclusive choice | `choice_card` with `choice_indicator` |
| Rich selectable/action row | `action_row` |
| Boolean preference | `switch` / `checkbox` |

Use medium fields with ordinary form actions, and small fields/actions in dense
toolbars. Search fields default to small; use the kit's medium size in forms.
Do not replace semantic sizes with arbitrary heights, background colors or static
frames. Form and compact variants intentionally differ in size, not visual language.

Use `primary`, `danger().outline()`, `warning().outline()` and other semantic button
variants instead of hand-painted colors. Busy actions use `disabled` or `loading`,
not opacity plus an early return in their click handler. Keep defensive handler
checks as a second boundary.

Rich buttons wrap their contents in the kit's horizontal content container. Put
multi-line layouts in a single full-width child `div().flex().flex_col()`; styling
the button root as a column does not change the layout of its contents. Do not nest
independent actions inside a button. Compound project/tree/canvas surfaces may keep
their own interaction model, while their action buttons use the shared controls.

Document editors, terminal grids, graph canvases and the compound chat composer
are not ordinary form fields. Editors retain their borderless reading surface;
the composer owns a focus-aware border on its surrounding card. Changes to these
surfaces must not be applied indiscriminately to forms.

## Verification

- `cargo test -p dunlin-ui --lib`
- `cargo test -p dunlin ui_contract_tests`
- `cargo run -p dunlin-ui --example form_controls_smoke` (independent of desktop)
- `cargo build -p dunlin` then `target/debug/dunlin --controls-smoke`
- `python3 scripts/release_qa.py --native-only` runs this and all other registered
  native UI scenarios after building the desktop/mock agent; PR CI runs the same
  suite. See [test quality](../quality/test-quality.md) for isolation and coverage scope.

The native control smoke uses a standalone fixture, not the application's data.
It checks both themes and densities, aligned field/action geometry, pointer focus,
disabled fields/actions, loading actions, choices, keyboard activation, tabs, menus
and links. The source-contract checks guard shared constructor usage but do not
replace native behavior or visual acceptance.
