# UI input and adapter discovery handoff

## Settings shortcut, search shade and smaller avatar — 2026-11-06

- Restored Settings at the bottom of the navigation rail. It dispatches the same
  application action as Cmd+,; the account-menu entry remains available.
- Project search and the command palette paint a 14% black fullscreen shade
  behind the search panel, without adding blur or changing modal/focus guards.
  Escape and outside clicks retain their existing dismissal behavior.
- The account avatar is 22 px instead of 28 px (including image and fallback),
  while its 32 px click target stays unchanged.
- Native glass acceptance clicks the restored Settings button, verifies bottom
  placement and avatar geometry, and checks the fullscreen painted shade in both
  command and project-search modes. Desktop/UI tests pass (406 + 14).

## Project actions, group dialog and harness visibility — 2026-11-06

- The title-bar dropdown now only switches projects. All former actions live in
  the sidebar project context menu, which keeps Open project and adds trust,
  Finder reveal, group creation and local/remote import. Removed the project-row
  ellipsis and inline group picker. The larger context menu scrolls in small
  windows rather than extending beyond the viewport.
- Move to group uses the existing detached modal-window infrastructure on macOS
  (in-window fallback elsewhere). Existing assignment validation, persistence,
  No group and New group remain. Escape/outside click/Cancel dismiss without
  moving anything; navigation stays blocked while the dialog is open.
- Trust actions now accept the target project index: right-clicking an inactive
  project cannot connect/disconnect the unrelated active project's harness.
  Untrust still stops runtimes that actually depend on the target project.
- Harness choices no longer show ACP command subtitles. Settings retains the full
  catalog with Shown/Hidden buttons; these do not select a default or mutate a
  runtime. Persisted `hidden_harnesses` defaults to empty for old settings and is
  included in validated portable backups. Chat, project and group choices filter
  hidden harnesses; existing sessions and configured defaults remain untouched.
- SSH key input and chooser have matching explicit control heights. The chooser
  overrides the shared button's self-centering so it aligns with the input rather
  than the complete labeled field. Single-line Input height uses Styled::h
  because Input::h only applies to multiline fields.
- Native workspace acceptance clicks Move to group and Untrust for an inactive
  target, checks the detached dialog, assignment and Escape dismissal, compares
  SSH input/button centers and heights, clicks the visibility control without
  changing the default, and checks hide/show filtering and persisted preferences.
  Backup tests cover visibility export/import and invalid labels.

## Project search, reading zoom and recent-tab restore — 2026-11-06

- **⌘⌥P / View → Search Projects…** opens the existing palette in project-only
  mode. The magnifier beside Projects' `+` opens the same view. Names and paths
  match all query terms; paths disambiguate similarly named projects. Enter/click
  uses the existing guarded `select_project` path; Escape selects nothing. The
  normal command palette keeps ⌘K/⌘⇧P. Opening project search does not reload Git
  branches. The title-bar project dropdown is flat at every group depth; sidebar
  hierarchy indentation and collapse behavior are unchanged.
- **⌘+ / ⌘− / ⌘0** change/reset workspace reading size, including chat paragraphs,
  composer, code editors, terminal metrics, Graph cards and Trace labels. Native
  menu actions also work when focus is inside the composer. Sidebar/tab/Settings
  typography and the saved code-font preference are unaffected. Zoom is bounded
  to 70–200% and lasts for this run, not across a restart.
- Terminal glyph advances, row height, PTY rows/columns and pointer selection use
  the same zoom. Existing dock calibration and PTY identity are retained. Removed
  Dunlin's extra shell-prompt row fill only: ANSI colors, search highlights,
  selections, shell integration and shell configuration are unchanged.
- Shared tab state tracks recent selections separately from opening/drag order.
  Closing an active tab restores the last still-open tab in that pane, across
  file/chat/Git/Spec/terminal types. Inactive closes do not steal focus; stale
  file-close callbacks cannot close a different tab. Rekeyed chats retain history,
  and project switches discard stale selections. A restored selection does not
  record the transient fallback Chat as a user visit. Newly opened chats focus
  their composer instead of leaving keyboard focus on the now-hidden Git view.
- Native workspace acceptance exercises the real shortcut and magnifier, search
  selection/cancellation, the flat dropdown, actual editor line-height growth,
  terminal PTY column changes and chat paragraph growth. It closes new files,
  terminals and chats back to the previous Git tab, plus inactive/stale closes.
  Unit checks cover zoom bounds/reversibility and pane-local recent selection.
- The vertical navigation rail is unchanged. A compact section picker in the
  sidebar header is the proposed alternative, pending a product decision.

## Narrow Secrets, settings navigation and Usage — 2026-11-06

- Secrets headers/actions wrap and fields stack in narrow docks. Quick-key
  controls use the same medium size as inputs. Both credential forms offer only
  15 min, 1 h and 8 h; existing longer-lived policies are not revoked or rewritten.
- Sidebar has its own Workspace settings entry. Factory reset is in Recovery,
  before About, with the original confirmation and deletion semantics.
- Local billing has rolling 1d/7d/30d/all filters (all by default). Every token,
  cost, attribution and provider/model/project aggregate uses the selected range;
  account allowance and monthly budget alerts retain their independent meanings.
  Missing timestamps appear only in all. Daily totals no longer stop at 90 days.
- Billing breakdowns are stacked full-width; the log-evidence list is removed
  from Usage, not from diagnostics. Token labels scale through B (10^9) and T
  (10^12), so 19963.9M is 19.96B, not 19.96T.
- Account limits and local billing are cached for five minutes. Automatic
  project/harness/turn events and popover opening respect the account cache;
  explicit Refresh/Scan logs still bypass it. Stale data stays visible during a
  refresh. The usage popover reserves space for both results and refresh status.
- JSONL billing parsing projects metadata while skipping conversation/tool
  content. Discovery continues across batches; SQLite streams rows in a read-only
  snapshot instead of running repeated offset queries. The former
  ten-second and 100,000-row cutoffs no longer silently omit archive history.
  Safety bounds remain: 32 MiB per record and a million dedupe identities per
  provider scan. Genuine incomplete scans still warn, rather than claiming $0.

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| Local logs own historical usage; live allowances are separate | `crates/usage/src/billing.rs`: `scan_all`, `summarize_range`; `apps/desktop/src/usage.rs`: `Scope`, `Cache`, `refresh_billing` | current source | High | None in local aggregation path | Billing boundary/pagination tests |
| Sonnet/Haiku 5.5 rates include caching and Haiku's 100k prompt tier | Official `platform.claude.com/docs/en/models/{sonnet,haiku}-5-5/overview`; `billing.rs`: `price` | verified 2026-11-06 | High | No actual invoice reconciliation | Price/cache tier tests |
| A Codex backend alias does not identify a historical billable API model | `openai/codex` PR #18169 delegates the alias to backend/catalog mapping; OpenAI rate-card documentation describes auto-review without a versioned per-record mapping | checked 2026-11-06 | Partial | `codex-auto-review` → exact model/rate for that event | Inspect authoritative provider metadata if made available; keep unpriced meanwhile |

Proven path: read-only local provider logs/SQLite → deduped billing events →
time-filtered aggregates → Usage. Only the unfiltered aggregate is explicitly
synced to a relay; no raw messages or newly added event timestamps are uploaded.
Relay diagnostic lists remain bounded while aggregate totals cover the archive.
Zero-token `<synthetic>` events are known-free; synthetic events with tokens and
unknown models are not relabeled as free. Desktop maintainers own future alias
verification; inferred alias prices and hiding truthful warnings are rejected.

## Settings, terminal input and tab/history cleanup — 2026-11-06

- Settings pages now fill the content width with equal gutters (8 points shared
  plus the page's normal 24-point inset); the header uses the matching 32 points.
  The old left-aligned 760-point page cap caused the oversized right margin.
- The terminal focus context disables the UI kit's Tab/Shift+Tab focus bindings,
  leaving those keys to the PTY (`09` and `ESC [ Z`). Keyboard navigation in forms
  remains accessible. Dock terminals also receive input while Git history is open.
  Native acceptance captures an actual Tab byte from the isolated PTY and verifies
  that focus does not move; it does not depend on the developer's shell completion.
- Session status and full local `YYYY-MM-DD HH:MM` timestamp share the metadata
  line. Group headings use the date only, not a separate group for every minute.
- The overflow action is now **Close all tabs**, covering visible chats, files,
  workspace terminals, Git and Spec tabs. Dirty files retain Save/Discard/Cancel;
  Cancel retains every tab. Closing does not delete sessions or stop chat agents.
  Workspace terminal shells close; dock/dependency output is not a workspace tab.
- Git commits use the existing GPUI uniform virtual list. Ref/author labels are
  bounded and the header is consolidated. Native Git acceptance scrolls a
  500-commit fixture to the last row and back in dark and light/narrow windows.
- Empty Graph/Trace views inherit the reading surface instead of painting a
  different color beneath its padded edges. The light view switcher uses opaque
  theme surfaces without stacked native materials/shadows; dark glass is retained.

### Name/key storage contract (confirmed)

The new Settings form and the bottom Secrets panel default to **Name + Key**.
Agent access is an explicit advanced option. Plain keys have no expiry, hosts,
HTTP methods, project grants, recipes or operations, and cannot auto-approve.
Switching an existing credential to “Store only” removes its access policy on
Save, not when the toggle is clicked. Existing configured credentials open in
advanced mode. No schema field or wildcard destination was introduced.

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| Values and policies belong to the desktop Keychain; SQLite is metadata only | `crates/vault/src/store.rs`: `KeychainStore`, `ServiceKeychainStore::desktop`, separate value/policy service names; `apps/desktop/src/vault/mod.rs`: `create_vault_secret`, `update_vault_secret` | current source; security-framework 3.7 API | High | Real Keychain prompt/write not exercised | Optional isolated macOS integration/manual check |
| A name/key pair may be stored but cannot grant broker access | `crates/domain/src/vault.rs`: `SecretPolicy::is_storage_only`, `validate`; `crates/vault/src/broker.rs`: `authorize`, `stored_only_keys_are_valid_but_unlisted_and_unusable_without_reading_the_value` | policy v1, current source | High | None in local authorization path | Existing broker regression test |
| All creation UIs use the same write authority and shared labeled fields | `apps/desktop/src/vault/settings.rs`: `Draft::read`, `secret_field`; `apps/desktop/src/vault/panel.rs`: create callback; both call `create_vault_secret` | current source | High | Real provider usage intentionally not tested | Native form smoke plus broker tests |

Proven resource path: form inputs → `NewSecret` → validated `Secret`/`SecretPolicy`
→ Keychain value/policy under an internal ID; SQLite stores display metadata under
that same ID. Broker alias lookup → Keychain policy validation → project grant
authorization → value read only after authorization. No external provider mapping
or SDK operation is needed. Raw-key export and automatic unrestricted agent
access are rejected interpretations of the name/key request. Older v1 readers
that reject an empty destination policy fail closed rather than granting access.
No unresolved contract edge remains; real Keychain UI behavior is a verification
limit. Desktop maintainers own that optional platform check; automated tests use
fixture values/in-memory storage and never read a real credential.

## Visible reading/settings corners — 2026-11-06

- GPUI's overflow mask is rectangular: rounding a parent does not clip child
  backgrounds. Terminal bodies and the Settings header now inherit the parent
  fill instead of painting over its curves.
- Code editors keep gutter/selection paint inside horizontal corner-safe insets;
  no vertical viewport padding was restored. File maps retain full viewport height,
  round their own right background and keep canvas highlights away from that curve.
- Native workspace acceptance inspects painted quads, their clipping masks and
  corner radii for code editors with/without the map, Markdown, the first-tab join, standalone
  terminals and the inner Settings panel (including Harnesses in the light theme).
  These checks catch rectangular child fills that layout bounds alone missed.
  The existing quad inspection API is available in debug builds only; release
  rendering and the renderer's clipping implementation are unchanged.

## Gray chrome, bounded rows and import sources — 2026-11-06

- Light chrome now uses neutral gray at 84% opacity rather than 96%-opaque cream.
  Foreground/secondary text remains contrast-tested against both black and white
  backdrops. Reduce Transparency/non-macOS still use an opaque gray fallback.
  Settings remain accessible through the account menu and Cmd+,; their rail icon
  is removed.
- Tab drags show a floating preview with the real tab title/type, while the
  insertion underline and tab-strip-only drop scope remain. Git history joins
  files/terminals in the edge-to-edge pane path, eliminating the outer padding bands.
- The delete dialog reserves fixed icon space and a shrinking, wrapping text
  column. Its panel is viewport-bounded and scrollable; action buttons can wrap.
  No destructive warning is truncated or removed.
- Project rows use their actual clamped width minus group indentation. Below
  320 points they omit the monorepo badge; below 260 points names truncate.
  Wider wrapped names are capped at two lines. Full project/monorepo data stays
  unchanged and remains available outside the narrow row.
- Native acceptance covers floating drag feedback, Git viewport edges, deletion
  warning containment, a long project name in a 220-point sidebar, neutral gray
  translucency and the Add project source menu. No real account import is used.

## Workspace surfaces and new tab types — 2026-11-06

- The toolbar selector no longer loses two pixels of layout to its border. Equal
  fixed-width labels align with the selection; a second localized AppKit material
  follows the lens, which expands 14% while held. Motion/transparency accessibility
  fallbacks remain. Native smoke checks the expanded geometry after spring settling.
- File/terminal reading surfaces have no outer vertical padding. The file map
  reaches its viewport edges; pane bottoms remain rounded above the dock, and an
  unselected/empty strip rounds the upper-left corner. Empty panes show a muted,
  single-color Bird illustration and a Chat action rather than a bottom-aligned button.
- The tab overflow menu opens below its button and includes Split view, Terminal
  tab and persistent Show/Hide file map. A short `+` click still creates Chat;
  holding for 350 ms opens the kit menu with Chat, Empty file and Terminal. Arrow
  Down and right-click also expose it. Leaving the trigger/releasing cancels the
  hold, and modal/navigation changes clear transient menu state.
- Untitled files reuse editor buffers, undo, minimap and dirty-close guards. They
  never autosave or stat a made-up path. Cmd+S opens native Save As, confined to a
  new file in the current local project; collisions/symlink escapes are refused.
  Cancelling/error keeps the buffer. Save All stops at an untitled buffer instead
  of reporting it saved; repeat the action after choosing its path. Relay scratch
  saving is not implemented and never silently writes to the desktop filesystem.
- Workspace terminals reuse canonical PTYs and the existing viewport/input path,
  participate in tab ordering, pane moves and navigation, and remain separate from
  the dock layout. Closing one kills only that PTY. They are not restored on restart.
  Agent selectors stay hidden for terminal tabs.
- Settings use rounded, softly shadowed surfaces instead of an outer frame and
  common card outlines; actual inputs/selection indicators keep their focus cues.
  Account details omit idle status and manual Refresh. Opening them invokes the
  existing scoped, deduplicated usage refresh; real context measurements can still
  appear in the status bar, but idle placeholders do not.
- Verification: desktop unit suite including new-file collision/escape checks,
  native file-map/lens/hold-menu/scratch/PTY/dock-preservation smoke, existing
  controls/glass checks and vendor integrity. OS Save As and remote PTYs still
  require manual acceptance; tests use isolated local fixtures, not user projects.

## Tab drops, account menu and chat controls — 2026-11-06

- Workspace tab drop handlers now live only on tab strips and individual tabs.
  A thin insertion underline replaces the pane-wide colored backdrop; the generic
  drag-preview box is gone. Dropping on a tab changes its position, including in
  an unsplit workspace; dropping on strip space appends it. Cross-pane moves keep
  the canonical buffer/draft. Project/group hints check their own payload types
  through the reviewed `App::has_active_drag_of` addition, not any active drag.
- The account menu uses separate name/account labels and ordinary action/check
  rows instead of a wide embedded density switch. Settings and theme actions remain.
- Shift+Tab cycles only advertised harness modes while the chat composer is
  focused; the existing busy/disconnected checks and harness request path remain.
  Elsewhere Shift+Tab retains normal reverse focus traversal.
- Agent Markdown already supports labeled and bare web links. Native control
  smoke now clicks both in light and dark themes with a recording callback; the
  production callback routes allowed HTTP(S)/mailto links through `App::open_url`
  to system default handlers and refuses local/executable schemes.
- The status-bar branch picker grows with the loaded branch count to ten rows,
  bounded by available window height. Filtering does not resize it; keyboard
  scrolling uses the same geometry. Longer lists remain scrollable.

## Toolbar and tab controls — 2026-11-06

- Apple references: [Segmented controls](https://developer.apple.com/design/human-interface-guidelines/segmented-controls)
  and [Liquid Glass](https://developer.apple.com/documentation/technologyoverviews/liquid-glass),
  read through their official DocC JSON. The selector uses three equal, text-only
  segments for related views, spring selection, a small pressed lens expansion,
  pointer scrubbing with commit on release, focusable controls and arrow/Home/End
  keyboard support. It keeps existing command shortcuts.
- `views/view_switcher.rs` reuses `effects::ControlGlass`, backed by the existing
  public AppKit `NSGlassEffectView` capability checks on macOS 26+ and Popover
  material fallback on macOS 15. It is not a native UIKit segmented control.
  Native material remains below GPUI text and never takes pointer input. Reduce
  Transparency uses a solid surface; system/app Reduce Motion suppresses travel
  and pressed expansion. Hidden selectors clear their native material.
- The reviewed component fork animates only folder-tab fill; labels, close buttons
  and content do not fade. Folder bars have no background beyond their selected
  shape, and suffix padding puts the close control at the trailing edge. The
  application supplies its Reduce Motion setting to document and tool tabs.
- Sessions use equal-height general/project regions with independent scrolling,
  omit harness/model names and retain explicit deletion confirmation behind a
  small neutral icon. Scratch-chat contexts keep their general-only history.
  Each session uses at most two lines: title/date, then truncated context and a
  small status immediately before Delete. The status bar shares the window's
  chrome rather than drawing a separate band; light chrome uses a strong cream
  tint while dark chrome keeps its existing native translucency.
- Verification: selector position unit test, native toolbar drag and keyboard
  smoke, equal-height session probes, tab close/drag regression checks and vendor
  integrity verification. No user conversations or production files are used.

Scope: group-name input, project-list dismissal and Harnesses settings.

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| The active modal had a native first responder different from its GPUI view; restoring it after native style/parent changes fixes the input route | `apps/desktop/src/glass_smoke.rs`: `assert_native_input_responder`, `post_native_key`; `apps/desktop/src/overlay_window.rs`: `native::attach`, `makeFirstResponder` | gpui-pre-macos 0.3.9 / working tree | High: native responder regression failed before the fix | Physical keyboard/IME QA remains manual | Run AppKit-key smoke and then check a rebuilt app |
| The project list lacked outside dismissal; component menus already provide it | `apps/desktop/src/views/sidebar.rs`, `project_menu`; `vendor/gpui-component-0.8.0/src/menu/popup_menu.rs`, `on_mouse_down_out` | Working tree / component 0.8.0 | High | None | Native smoke |
| Claude Code's separate ACP executable is `claude-agent-acp`, not `claude`; the reported install is home-local | `~/node_modules/@agentclientprotocol/claude-agent-acp/package.json`, `bin`; executable symlink `~/node_modules/.bin/claude-agent-acp`; `apps/desktop/src/harness_discovery.rs`: `home_bins`, `SPECS` | Installed 0.100.1-preview.4, inspected 2026-11-05 | High | Actual authentication/agent connection was not exercised | Read-only scan, then user-controlled connection |
| Discovery previously omitted NVM version bins | `apps/desktop/src/harness_discovery.rs`, `search_paths`, `nvm_bins` | Working tree | High | Other custom npm prefixes are not inferred | Explicit Custom ACP path |
| Set up must not synchronously open a browser or pretend to install a package | `apps/desktop/src/views/settings.rs`: `harness_setup`; `apps/desktop/src/harness_discovery.rs`: `show_harness_setup`, `open_harness_docs`, `rescan_harnesses` | Working tree | High | Browser launch itself is not tested against user preferences | In-app setup/async rescan smoke; browser opener uses the tested bounded process helper |
| Settings selection changes only the new-conversation default | `apps/desktop/src/views/settings.rs`, `harnesses`; `apps/desktop/src/harness.rs`, `connect_harness` executable guard | Working tree | High | None | Native smoke checks the existing session remains unchanged |

Proven retrieval path: Settings Scan → `discover` → `search_paths` → executable
metadata → `Detected.adapter` → settings status. Launch independently resolves
the same command and search paths. Detection does not execute adapters, authenticate,
or install software. Missing files remain unavailable; rescans read fresh metadata.

Rejected equivalence: an installed Claude CLI does not prove an ACP adapter exists.
No catalog names, session identities or stored settings formats change. New
discovery scope covers the local user's NVM version bins and home-local npm bins;
repository dependencies and remote detection are unchanged.

Verification: desktop unit tests, `--glass-smoke` (group typing/Enter, outside click,
radio selection, new NVM adapter rescan), and `--onboarding-smoke`. Next owner: app
maintainer for native keyboard and real adapter connection QA; no provider request
or user installation is required by the automated fixtures.

## Chrome, navigation and confirmations — 2026-11-05

- Harness status and its connection/error detail moved out of the title-bar badge
  into the selected Local host/relay row. Non-selected devices do not borrow the
  active session's status. The host connection dot remains in the title bar.
- Back/Forward destinations contain content only: project, relevant session,
  main/file/Git/spec surface, Settings/Dashboard/Security. Sidebar section/layout,
  window bounds, panel widths and sidebar collapse do not create destinations;
  file/Git destinations do not record unrelated session changes. Returning to
  content does not restore old window dimensions. Dirty-editor/navigation guards
  remain in place.
- Usage & Limits uses the same single-color Gauge icon as other rail controls.
  Projects, their sessions and inline group editor are inset another 24 px within
  sidebar group sections; ungrouped projects and the flat List are unchanged.
- Add projects starts at the user's home. Folder labels are left-aligned within
  full-width buttons. Entries are actual directories only, excluding dot names,
  symlinks, and macOS Finder-hidden flags. Up stops at home; an explicitly entered
  absolute path remains supported (this is not a filesystem sandbox).
- `crates/ui/src/prompts.rs` installs a single themed GPUI prompt builder. The
  inventory of `window.prompt`/`w.prompt` covers AI/manual commit, Git/checks/
  review/comparison/budget/subtask actions, Explorer, task runner, spec/security,
  vault, skill deletion, editor conflicts and factory reset. These callers retain
  their original warnings, answer indices and post-confirmation scope/stale checks.
  Escape chooses only an explicit Cancel answer; Enter on the dialog itself
  cannot authorize the destructive option. Buttons remain keyboard-focusable;
  long detail text scrolls and actions remain outside that scroll area. Native
  backdrop glass is cleared while a GPUI confirmation is active so it cannot blur
  or cover the new in-window panel. OS file/keychain/permission sheets remain native.

Acceptance fixtures add checks for content-only Back after layout/resize changes,
themed prompt mouse selection and exact three-answer Escape semantics. Folder
tests include dot folders, Finder-hidden flags, files and directory symlinks; the
import smoke also asserts that its initial browser path is home. No AI request,
real credential action or production-repository confirmation is executed by these
checks. Final execution results are recorded below.

Initial automated results: 373 desktop tests, the UI prompt cancellation test and
3 UI asset integration tests passed. Native workspace smoke passed content-only
navigation, device-menu status placement, project indentation and themed prompt
mouse/Escape checks. A first extension of the smoke dispatched Escape while the
owner entity was borrowed; input dispatch now happens outside that borrow, like
the existing native click tests. Project-import smoke and glass/input smoke passed.
No provider session, actual AI commit, vault mutation or factory reset was run.
The two non-"Cancel" cancellation labels are marked explicitly as cancel answers
at their call sites: vault **Deny** and quit **Keep Running**. Their labels and
indices are unchanged; Escape/default Enter cannot grant a credential or quit.
The final workspace fixture also checks default Enter with an explicitly typed
non-"Cancel" cancellation label. Dialogs expose a Dialog role, accessible title,
tab group and ordinary focusable application buttons. Source search found no
separate NSAlert/runModal confirmation implementation bypassing the shared builder.
Final build and test rerun: 373 desktop tests and 4 UI tests passed. The standalone
bundle is `dist/ui-polish/Dunlin UI Polish.app`; `codesign --verify --deep --strict`
and `git diff --check` passed. Older bundles were preserved. Pixel-by-pixel manual
review, production relay/harness connections and actual destructive confirmations
remain untested; automated confirmations use harmless fixture-only receivers.
The final packaged executable passed the full extended native workspace smoke,
including selected-device status placement, indentation, resize/navigation,
mouse selection, Escape and safe default Enter with the typed Keep Running label.

## Lighter shell and consolidated usage — 2026-11-06

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| A blurred native window was hidden by an opaque component root | `vendor/gpui-component-0.8.0/src/root.rs: WindowState::style`; `apps/desktop/src/app.rs: DunlinApp::new` | GPUI Kit 0.8.0 / working tree | High | Visual appearance on the user's display | Native workspace smoke checks the root override; manual desktop review |
| Rail movement came from adding left padding only after collapse finished | `apps/desktop/src/app.rs: Render` sidebar wrapper | Working tree | High | None | Measure the same rail button during collapse and expansion |
| Account allowance is not session cost or ACP context usage | `usage.rs: usage_target, active_usages, account_status`; `app/overlays.rs: status_bar` | Working tree | High | Live allowance remains provider-dependent | Existing scope tests plus missing/invalid/session/estimated-limit regressions |

Proven paths:
`active session -> usage_target (environment/harness/provider/model) -> active_usages -> account_status`;
`active session -> SessionSummary.cost_usd -> session-cost label`;
`active harness -> ACP usage -> context label`.
These are separate values, displayed together, never converted into each other.
The status popover retains account windows, reset times, cached/unavailable detail,
refresh, Console link and full Usage settings. No new credential reads, provider
requests, relay endpoints or storage migrations are introduced. The obsolete
sidebar-collapse preference remains readable for compatibility but has no UI.

The title bar is 38 px, document tabs use a borderless segmented treatment,
reading surfaces remain opaque, and Reduce Transparency retains an opaque shell.
Compact projects target 32 px rows; session metadata remains in accessible labels
and tooltips rather than occupying three lines. Rail glyphs are 20 px within fixed
36 px targets. Next owner: user for visual acceptance on the target display;
maintainer for the native shell/usage-popover and existing input regressions.

Verification: 380 desktop, 6 UI and 3 asset tests passed. Native workspace smoke
checks the transparent root override, 38 px title bar, 32–34 px compact project
rows, fixed rail coordinates through both animation directions, and the bottom-right
usage popover including Escape dismissal. Controls (both themes), glass/input,
Markdown light/narrow and Git safety smokes passed. No visual/material acceptance
on the user's display or release packaging is claimed.

## Browser-shaped tabs and dark glass — 2026-11-06

Document tabs now use GPUI Kit 0.8.0's existing `Folder` variant: rounded upper
corners, outward lower curves, inactive separators and no outline/shadow capsule.
The active fill and Chat/Graph/Trace reading surface share `Palette.card`.
The strip remains 28 px; its `+` is the trailing scrolling-strip element, followed
by the existing pinned file-tab menu. Selection, close callbacks, dirty-buffer
guards and session lifecycle are unchanged. Native label-icon/close-center and
new-tab adjacency checks cover this layout.

Dark chrome uses an 82% dark tint over the native material, versus 18% in light
mode. The backdrop is still blurred and translucent; Reduce Transparency remains
opaque. GPUI's macOS appearance override already sets Aqua/Dark Aqua through
`NSApplication.setAppearance`; no second appearance implementation was added.
A unit check composites the dark tint over white, and native theme-switch checks
verify both automatic and explicit appearance paths use the corresponding tint.

Verification: 380 desktop, 7 UI and 3 asset tests passed. The native workspace
smoke passed new-tab adjacency (measured after the trailing close button, not
the label), icon/close alignment, theme tint switching and existing workspace
safeguards. The isolated controls smoke passed selection and pointer/Space close
isolation in both themes. Glass/native input, Markdown light/narrow editing and
Git dirty-buffer/mutation-lock regressions also passed. Visual/material acceptance on the user's display and
release packaging remain unverified.

## Unified glass and larger controls — 2026-11-06

- The 36 px Folder-tab strip and workspace wrapper no longer paint opaque
  backgrounds: inactive tabs expose the app's native glass. The selected tab
  still joins the opaque reading surface. The reading wrapper reserves its
  rounded lower foot because GPUI's child overflow mask is rectangular.
- Composer separation uses a coordinated soft fill and small shadow, not an
  outline or a contrasting harness-header band. Dark neutral/hover tokens share
  a blue-gray family; recent-project tiles use quiet borders. Source Control and
  Security sidebar cards use a subtle foreground wash over the shell instead of
  opaque dark rectangles. Warning/error semantics remain unchanged.
- Title navigation uses 20 px glyphs in centered 32 px targets; macOS traffic
  lights are aligned to the 38 px title bar. The repository-host glyph is 20 px.
  Project-picker rows are 42 px with 24 px icons; menu actions use medium sizing.
  Remove Project retains its danger treatment and confirmation without an outline.
- Explicit and system appearance changes interpolate palette colors, glass alpha
  and component tokens over 240 ms. A new change starts from the visible palette
  and invalidates the previous timer. Startup is immediate; Reduce Motion skips
  transitions. No dependencies, stored formats or provider behavior change.

Checks include interpolation endpoints/direction, continuous glass alpha,
navigation/tab/picker geometry, close isolation, automatic/explicit theme changes,
reversing a transition and Reduce Motion. Display-level material acceptance and
release packaging are still manual boundaries.

Verification: 380 desktop, 8 UI and 3 asset tests passed. Native workspace,
light/dark controls, glass/input, Markdown light/narrow and Git safety smokes
passed. The glass fixture now locates the branch trigger by measured bounds,
rather than the obsolete x=45 coordinate displaced by the larger hosting icon.

## Profile, GitLab imports and project-list contract — 2026-11-06

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| Connecting is not a started conversation | `harness.rs: live_agents`; `sidebar.rs: project_sessions`; `session_list.rs: has_user_message` | Working tree | High | Native transient-state regression | Exercise connection-only drafts and first user messages |
| Current hosting identity comes from the authenticated user, not Git author configuration or repository owner | GitHub REST `GET /user`; GitLab REST `GET /user`; official Users API docs | APIs inspected 2026-11-06 | High | Real accounts intentionally not queried during development | Stub CLI responses and malformed identities |
| GitLab projects use `path_with_namespace`, including subgroups, not GitHub `full_name` | GitLab Projects API `GET /projects/:id` and `GET /projects?membership=true&search=…`; `git/hosting.rs: RepositoryLink::parse` | API/docs inspected 2026-11-06 | High | `glab` is not installed on this machine | Fake glab API, pagination and clone fixture |
| CLI authentication owns credentials | `github.rs: cli_command`; official `glab api` / `glab repo clone` docs | Existing gh integration / current glab docs | High | Live GitLab login remains external | Assert explicit hostname, GET, bounded output, no overwrite |

Proven resource paths: local CLI account → authenticated `GET /user` → account
ID, name/login and avatar URL; explicit gitlab.com URL → validated nested project
path → URL-encoded project API ID → `path_with_namespace` → confirmed CLI clone.
Search uses GitLab's search parameter, not GitHub's qualifier language. The app
does not infer an account from Git commit identity and never reads/stores tokens.
Existing public-host scope remains github.com/gitlab.com; self-managed hosts are
not silently accepted. Avatar transport must be HTTPS, host-allowlisted, bounded
and unauthenticated, with no redirects. Empty/offline identity has a neutral fallback.

Implementation scope: shared buttons and compact toolbar exceptions; host menu;
session visibility; profile menu/startup refresh; GitLab status/imports; Comfortable
project metadata ordering, preferences and native/unit fixtures. No session deletion,
credential changes, automatic clones or disabled metadata preferences. Maintainer
owns fixture verification; user owns real-account and display acceptance.

Confirmed implementation paths:
- `hosting_account.rs` reads only `/user` through the explicit provider CLI;
  `user_profile.rs` caches identities for five minutes and refreshes from the
  avatar menu or explicit CLI Check Again. The current repository's hosting
  provider selects the preferred authenticated profile, then GitHub/GitLab order
  is the fallback. Name/login initials remain available if avatar retrieval fails.
- `gitlab.rs` validates public GitLab paths, paginates membership search in
  100-project pages, refuses results over 1000 and delegates cloning to the same
  non-overwriting, symlink-rejecting helper as GitHub. New clones remain untrusted.
  Local GitLab topics use the Projects API; no unsupported relay endpoint is added.
- `sidebar.rs` no longer lets connection-only live status bypass draft visibility.
  Comfortable orders repository name, branch/local changes, topics, language.
  Compact shows no inline metadata or invisible language/topic filters; preferences
  remain editable and persisted. Tooltips retain the language breakdown.
- Ordinary shared buttons use medium sizing and natural-width alignment; toolbar
  segments/icons and explicit full-width menu/action rows retain their purpose.
  The host menu places status beside the host name and never borrows unrelated
  global notices as connection errors.

Rejected interpretations: a connected agent is not a started conversation;
Git author/repository owner is not the signed-in account; GitHub search qualifiers
are not GitLab search syntax. The only remaining edges are live-account/actual
image delivery and display acceptance, deliberately not exercised against real
accounts. Next owner: maintainer for release packaging, user for real-account QA.

Verification: 382 desktop tests, 8 UI tests and 3 asset tests passed. The full
144-test workspace suite passed with `--test-threads=2`; an added GitLab metadata
regression then passed with all 5 focused GitLab tests. The initial highly
parallel workspace run hit two unrelated Git fixture timeouts; neither recurred
in the bounded-concurrency full run. Native project-import/profile/menu, workspace
(including connection-only drafts), controls light/dark, glass/input, Markdown
light/narrow and Git safety smokes passed sequentially. Workspace confirmation
fixtures explicitly refresh status after their file write before requesting
preflight, rather than racing the file watcher. No guards were weakened, no live
credentials/hosting writes were exercised and no release bundle was rebuilt.

## Workspace panes, connection dialog and group cleanup contract — 2026-11-06

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| SSH is a terminal transport, not a relay/remote-agent grant | `terminal/mod.rs: open_ssh_terminal`; `crates/terminal/src/lib.rs: SpawnOptions::ssh`; `remote.rs: pair` | Working tree | High | No SSH-to-relay bootstrap exists | Reuse explicit SSH/config/identity and existing relay pairing without inventing remote capabilities |
| Cleanup must distinguish generated ignored data from source | `git/hosting.rs`, `git/history.rs`, existing bounded Git helpers; `platform.rs: trash` | Working tree | High | No group cleanup planner exists | Add ignored/untracked allowlist preview, fresh identity checks, then recoverable system Trash |
| Terminal rendering uses code font plus one fixed symbol fallback | `terminal/mod.rs: terminal_font, cell_width, render_row`; `settings.rs: font_picker`; GPUI `Styled::font` | GPUI pre 0.3.9 / working tree | High | Real installed Nerd Font names must enter the fallback list | Verify styled text receives fallbacks and shape representative PUA glyphs |
| Current document state has one focused editor/session but canonical file buffers already belong to tabs | `workbench.rs: State, Tab, activate_file_tab, stash_editor_tab` | Working tree | High | Independent pane selection/ownership must not duplicate or replace buffers | Inspect connected rendering/callbacks, then test concurrent panes and dirty-close guards |

Scope: shell scale/corners, Security navigation migration into Settings, Explorer
root removal, group hierarchy/cleanup, terminal toolbar/fonts, dedicated relay/SSH
connection dialog, two document panes and dashboard copy. No blanket `git clean`,
recursive deletion of arbitrary paths, key material persistence, host-key bypass,
implicit remote shell, credential installation or automatic agent reconnection.

Implementation: two in-memory tab groups share canonical `workbench::Tab`
editors and parked session runtimes. Pane mouse/key capture activates the correct
selection before callbacks; failed session activation stops input propagation.
Composers are session-owned (including session-ID rekey), chat scroll handlers
target their session, and graph/trace selection/search state is pane-owned.
Splitting, moving a selected tab and merging do not close documents or agents.
The overflow action explicitly closes **all project file tabs**, not just one pane.
Pane layout is intentionally not persisted across restarts.

Group Clear previews only ignored, untracked `node_modules`, `.turbo`,
`.pytest_cache`, `__pycache__` and `.next/cache` directories. It revalidates root
and directory identities, ignore/tracked state, group membership and app activity
before using system Trash. Symlinks are never traversed; external installers and
build tools must be stopped by the user. A directory already moved before a later
failure remains recoverable in Trash; disk space is freed when Trash is emptied.

Connection UI uses the existing attached native modal plane. Relay pairing and
SSH are separate modes; an SSH identity is a user-selected file path, never key
material copied into settings. Explicit identities disable multiplex reuse and
require private filesystem permissions. No host-key checks are disabled.

Verification additions: generated-folder ignore/allowlist/tracked/symlink/stale-root
fixtures, explicit SSH identity/port/permission tests, installed Nerd/Powerline/NF
font matching, pane ownership/rekey tests, and native workspace scenarios for
canonical dirty-buffer retention/cancel-close, independent conversation drafts,
blocked session switching, and connection modal open/cancel without networking.
`git check-ignore` deliberately has no `--literal-pathspecs`: it accepts literal
paths and rejects that option; the first cleanup regression run caught this.

Native verification: fresh-fixture workspace/pane acceptance passed, followed
sequentially by project-import/profile, controls, glass/input, Markdown and Git
safety smokes. The dirty-pane fixture uses the editor's real `replace` operation:
programmatic `set_value` intentionally does not emit an edit/change event.
No real SSH/relay connection, private key, repository cleanup or system Trash
operation was exercised. Pixel/material acceptance on the user's display and a
release `.app` build remain outside this verification; no commit/push requested.

Full bounded-concurrency run passed: desktop 387, terminal 27, UI 8, maintained
assets 3, workspace 149 (`cargo test -p dunlin -p dunlin-ui -p dunlin-terminal
-p dunlin-workspace --locked -- --test-threads=2`). Both pane ownership/rekey
tests also passed against the later working tree. The repository was changing
concurrently in unrelated import/backup areas; those changes were preserved.
Final desktop rerun passed all 391 tests; the rebuilt debug executable also
passed workspace acceptance again with both relay and SSH dialog modes rendered
before cancellation. `git diff --check` passed.
