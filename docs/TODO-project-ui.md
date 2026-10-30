# Project navigation and light-theme polish

- [x] Center Dashboard activity totals and soften light-theme dots.
- [x] Give recent-project tiles visible light-theme containers.
- [x] Place Dashboard data caveats at the bottom of the view.
- [x] Put the group-agent icon beside the group edit action.
- [x] Import multiple selected local folders or a collection of repositories.
- [x] Detect declared monorepos and display a badge.
- [x] Add a persisted project-list branch visibility preference.
- [x] Enlarge Settings; add Design → Application / Sidebar / Agent / Editor.
- [x] Fix Settings corner clipping in the light theme.
- [x] Hide unstarted session drafts without deleting any history.
- [x] Run regression tests, lint and formatting checks; rebuild and verify the app.

Validation: 290 tests passed (4 optional integration tests ignored), Clippy with
warnings denied, formatting/diff checks, release bundle and strict ad-hoc signature
verification passed. Visual GUI checks remain unavailable because the Orca CLI
cannot run in this environment; no paid AI prompts or real-repository mutations
were used for testing. Changes remain uncommitted and unpushed.

Repository discovery must be read-only and bounded, skip generated directories and
implicit symlinks, deduplicate canonical paths, and stop at a repository root so
submodules and monorepo packages do not become separate projects accidentally.

## Settings backdrop regression — 2026-10-30

- [x] Replace the full-viewport NSImage mask (AppKit scaling displaced/shrank the
  foreground cutout) with a native-point CAShapeLayer even-odd vector mask.
- [x] Include native bounds, coordinate orientation and backing scale in cache
  invalidation; suppress implicit animations and clear masks for menus/sheets.
- [x] Test the entire Settings foreground, zoom/Retina geometry and actual native
  offscreen compositing, including clearing the mask after closing the overlay.

Visual window automation remains unavailable: `orca skills get computer-use --json`
returns `zsh:1: command not found: orca`. Offscreen tests do not inspect or change
the user's running application, settings or session database.

Regression validation: 304 tests passed, 4 optional tests ignored; Clippy with
warnings denied, formatting and diff checks passed. Rebuilt `dist/Dunlin.app`
and verified its strict ad-hoc signature. No commit/push or user-data changes.

## Backdrop strength and remaining corners — 2026-10-30

- [x] Reduce the native-material blend from 100% to 28%; keep 18% dimming.
- [x] Give every custom foreground probe its actual radius, including the modal
  (12 pt) and compact status-bar (8 pt) branch pickers.
- [x] Use a rounded dimming path matching the native blur hole; rectangular dim
  cutouts left bright corner triangles even after fixing the native blur mask.
- [x] Test the tessellated dim path at all four corners, inside the panel, with
  offsets and invalid geometry; verify no missing/doubled dimming by area.

Validation: 306 tests passed, 4 optional tests ignored; Clippy `-D warnings`,
formatting/diff checks and strict ad-hoc bundle signature passed. Updated
`dist/Dunlin.app`; visual GUI validation remains unavailable. User data and the
running application were not changed; no commit/push.

## Native glass instead of alpha crossfade — 2026-10-30

- [x] Replace the 28% opacity blend with actual `NSGlassEffectView` regular glass
  on macOS 26+, gated by public runtime class/selector availability so the macOS
  15 deployment baseline is unchanged.
- [x] Use translucent `NSVisualEffectMaterial::Popover` within the window on
  older macOS, not the UnderWindowBackground material.
- [x] Keep the native effect at full alpha: glass/material controls translucency;
  reducing view alpha mixes sharp original text back into the background.
- [x] Preserve rounded dimming/vector masks, sharp foreground text, input
  passthrough and native Reduce Transparency behavior. No capture/private filters.
- [x] Test runtime ABI/selectors and fallback material. Validate native glass
  initialization, transparent content, layer mask and input passthrough in a
  separate main-thread AppKit diagnostic without opening a window.

Read-only system diagnostic: Reduce Transparency and Increase Contrast were both
off. Visual validation in the running app is still unavailable; no user setting
or session database was changed. This supersedes the previous 28% blend.

Validation: 308 tests passed, 4 optional tests ignored; Clippy `-D warnings`,
formatting/diff checks and strict ad-hoc signature passed. Rebuilt
`dist/Dunlin.app`. Binary symbol inspection confirmed no load-time dependency on
the macOS 26 glass class; older macOS reaches the runtime-gated Popover fallback.
No commit/push.

## Panel-local glass and a quiet background — 2026-10-30

This supersedes the full-window glass and strength experiments above. Only the
two approved visual changes are included; branch picker sizes are unchanged.

- [x] Move application overlays to a transparent owner-attached GPUI window,
  retaining a transparent component Root for dropdowns, text selection and Tab.
- [x] Insert native glass below its Metal controls and above the owning scene;
  measure rounded panel bounds including the one-point border. Use runtime-gated
  Liquid Glass regular, with a behind-window Popover fallback on macOS 15+.
- [x] Give Settings/branch panels a 14% tint, delicate border and soft shadow.
  Keep Settings responsive up to 1120 × 760; retain branch layouts 530 × 460 and
  300 × 170.
- [x] Keep the owner to 12% dimming and HeaderView material at 14% alpha; no
  full-window Liquid Glass. Respect native accessibility material preferences.
- [x] Synchronize native bounds and GPUI viewport caches; explicitly invalidate
  the overlay plane after owner changes. Transfer input focus only when needed.
- [x] Preserve native menu delegation, checkout/quit safeguards, prompt leases
  and cleanup, with in-window rendering if the transparent plane cannot attach.
- [x] Add debug-only `--glass-smoke <fresh-absolute-fixture-dir>` integration
  coverage: real native sibling order below controls, opening/resizing Settings,
  switching to the unchanged branch picker, actual input focus, closing/reopening
  and normal guarded Quit while Settings remains open. Fixtures isolate database,
  window geometry and logs; Custom ACP auto-connect, network and notifications
  are disabled. No prompts, screenshots or simulated user input.

Automated tests validate structure/lifecycle, not appearance. Visual approval is
still required after restarting the rebuilt bundle; GUI automation remains
unavailable. The user's session database and preferences are not used by smoke
fixtures. Earlier fixture runs only appended lifecycle diagnostics to the normal
log; current fixtures also isolate logging.

Validation: 311 tests passed, 4 optional tests ignored; workspace/all-target check,
Clippy `-D warnings`, formatting and diff checks passed. Two extended isolated
smoke runs (`panel-12`/`panel-13`) passed including sibling ordering and normal
Quit with a reopened panel. No commit/push or session/preference changes.
Rebuilt `dist/Dunlin.app` in release mode and verified its strict ad-hoc signature;
the bundled executable's `__TEXT` sections match the release build.
