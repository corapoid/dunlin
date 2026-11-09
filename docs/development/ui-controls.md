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
| Ordinary labeled action | `action_button` |
| Quiet navigation / status trigger / toolbar icon | `button` / `icon_button` |
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

`action_button` uses a solid, theme-matched neutral fill, with a subtle raised edge
and inset highlight. Neutral actions are graphite in dark mode; reserve the
contrasting `primary` fill for the main action and semantic outlines for destructive
or warning actions. Hover and press have distinct palette tokens; pressing removes
the raised shadow and release restores hover. Colored outline actions keep at least
4.5:1 text contrast even when pressed. Use the shared settings recipe in settings; align action groups
to the right, outside descriptive text. Never apply this treatment to navigation,
tags or status chips: those retain the quiet `button` base. Ghost styling is for navigation, rich
rows and toolbar icons, not ordinary labeled actions. Button labels animate their
text and intrinsic width through `MorphText`; first render is static and both
system and application Reduce Motion disable transitions. Keep stable button IDs
across state changes and retain the plain label for accessibility.

The action audit covers credential settings/policies and the quick-key panel,
SSH/server administration, provider authorization, project/group forms, file/Git
confirmations, Run, source control, worktree/check/review actions and knowledge/spec
cards. Compact controls remain only for navigation, exclusive choices, branch/ref
chips, source-linked inline metadata and toolbar icons. Do not shrink an ordinary
action with `small`, `xsmall`, a custom height or ghost styling. Source-contract
tests guard the audited settings/forms and all `action_button` call sites; the
native fixture checks actual idle/hover/pressed/released paint for neutral,
primary and destructive-outline actions in both themes and densities.

Rich buttons wrap their contents in the kit's horizontal content container. Put
multi-line layouts in a single full-width child `div().flex().flex_col()`; styling
the button root as a column does not change the layout of its contents. Do not nest
independent actions inside a button. Compound project/tree/canvas surfaces may keep
their own interaction model, while their action buttons use the shared controls.

Document editors, terminal grids, graph canvases and the compound chat composer
are not ordinary form fields. Editors retain their borderless reading surface;
the composer owns a focus-aware border on its surrounding card. Changes to these
surfaces must not be applied indiscriminately to forms.

## Settings recipe

`crates/ui/src/settings.rs` owns settings typography, spacing, surfaces and controls.
Use `settings::input`, `search`, `button`, `segments` and `segment`; these wrap the
same native kit primitives, preserving keyboard focus, disabled/loading states and
animated action labels. They are not independent reimplementations of text editing
or accessibility. The Kobra examples inform composition and interaction, not a
React dependency in the native client.

`settings::Tokens` maps semantic surfaces/fields/ink/outlines to the application
`Palette` and supplies layout metrics. Do not hard-code a second dark palette in
settings. Skin work belongs in that mapping and the shared palette, not individual
pages. This is the styling foundation, not a shipped skin loader or skin selector.
Settings fields and actions share a 36-point comfortable height (density-adjusted)
and capsule rounding. Normal preferences use `group` and spacious wrapping `row`
layouts; `card` is reserved for forms, lists and bounded content. The sidebar keeps
the existing menu groups, labels and order. Page headings carry one subtitle rather
than several repeated scope notices.

The agent page separates detected/configured agents from a closed-by-default setup
catalog. The radio selects the default for new chats; a labeled **Show in agent
picker** checkbox only controls visibility. Neither installs or starts an agent.
Import history is shown only for supported readers with a detected CLI or adapter;
ACP verification is not required for read-only history imports. Uninstalled agents
have setup actions, not misleading zero-import rows. Existing hidden-agent
preferences, default selection and imported histories are preserved.

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

For opt-in visual review, run `DUNLIN_SETTINGS_CAPTURE=1 target/debug/dunlin
--settings-smoke <fresh-absolute-dunlin-settings-smoke-directory>`. It saves PNGs
of every settings page in both themes alongside fixture data, the expanded agent setup catalog, plus Credentials
hover/press/release and unsaved-form captures. The refresh gesture is canceled
before activation; the empty credential draft is discarded without a Keychain write.
Captures target only the fixture's own native window and do not automate OS
screen-recording consent. Review the images: layout/behavior assertions alone do
not establish that the interface looks correct.
