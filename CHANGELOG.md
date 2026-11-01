# Changelog

All notable changes to Dunlin are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Add release metadata/artifact verification, adversarial packaging tests,
  isolated automatic QA reports and a read-only repository-policy auditor.
- Add pinned cargo-deny auditing and CodeQL security scans; keep discovered
  unmaintained-dependency and policy findings visible instead of waiving them.
- Add complete-tree integrity and original archive provenance for nine maintained
  dependency forks, signed-mainline source authorization and main-push CI attestations.
- Add a read-only source-authorization rehearsal workflow, with explicit checks/PR
  API access and no release/tag mutation, to validate the real workflow token.
- Record approved solo-maintainer policy and beta-only manual acceptance deferral;
  require real trusted checks/signatures without pretending independent approval.

### Fixed

- Remove bincode, instant, paste, rustybuzz and ttf-parser from the entire lockfile
  without advisory exceptions. Migrate clocks/macros, resvg/font shaping and all
  nested bundled syntax/theme/license dumps; preserve source data and exact licenses.
- Pin workflow actions to verified commit SHAs and enforce eight app-bound checks,
  signed squash merges and no repository bypass actors.
- Address full Rust CodeQL findings: guard nullable rav1e C API handles/results,
  avoid raw session/name/startup-error logging, and constrain Git to literal
  path operands. Document six individually demonstrated analyzer false positives
  without disabling queries or excluding patched Rust sources.
- Harden subsequent releases with exact tag/version checks, least-privilege
  build/publish separation, artifact provenance, final-byte checksums and
  changelog-backed release notes. Sign app and relay; require explicit Accepted
  notary reports, staple before packaging and never label signed-only as notarized.
- Validate bundle metadata before cleanup and derive the default bundle version
  from Cargo. Support disposable output directories without replacing installed apps.
- Draw both native glass-smoke windows before geometry checks, avoiding stale
  background-window frames without weakening layout/focus/cleanup assertions.
- Point website download metadata at the verified published beta.3 assets and
  derive its page title/URLs from one version value, rather than stale beta.1 links.

## [0.1.0-beta.3] — 2026-11-01

Agent workbench and GitHub CI/PR integration, safer local/relay Git workflows,
editable visual Markdown, and inherited project/group runtime settings. Preserve
existing conversations, editor buffers and undo; publishing this version does not
replace or restart an installed application or relay.

Validated with 487 passing workspace tests (4 optional integration tests ignored),
strict Clippy, formatting, desktop/relay builds, native dark/light-narrow smokes
and paired loopback Git HTTP smoke. Manual dialog/accessibility, real GitHub and
deployed-relay acceptance remain pending. Worktrees are not sandboxes, reported
USD is not a guaranteed billing cap, and Rust AST impact is not compiler/LSP-complete.

### Added

- Add a local GitHub.com `gh` integration in the repository tab: exact-commit CI
  workflow/job/step status polling, completed-job logs, PR files/checks/discussion
  and fresh confirmed create/edit/comment/review/draft/state/merge and CI actions.
  Pin account/repository/attempt/commit identity; no implicit push or local fallback.
- Add client-context inspection with wire-envelope parity, explicit versioned
  decision memory, saved-file task baselines/review/selective restore and a confirmed
  argv-based local checks/problems center. Preserve drafts; evidence never starts
  an agent.
- Add durable task request/time/reported-USD budgets with pre-wire reservation,
  foreground/background cancellation and permission guards, plus safe released-
  history retention. Non-USD/invalid costs and unknown child accounting never
  become zero USD; ceilings are not guaranteed provider billing caps.
- Add separately confirmed same-commit A/B worktrees, identical one-shot agent
  dispatch with shared limits, observed models/diff/check/cost evidence and human
  preference without merge/copy. Retain partial trees; never replay paid work on
  restart. Add an incremental bounded Rust AST impact map with candidate tests,
  source verification and evidence → draft; not compiler/LSP-complete semantics.
  Cover two real local mock-ACP connections in an isolated native smoke.
- Add explicitly confirmed local/relay stash, worktree, merge, rebase and
  cherry-pick operations with continue/abort, content-backed preflight, semantic
  index checks, one-use server plans and separate grants for external worktree
  deletion. Preserve canonical editor buffers, undo and review protection across
  watchers, dismissed dialogs and failed reloads; no automatic force/stash/retry.
- Add opt-in inline HEAD blame for unchanged source lines, author/commit hover and
  commit-history navigation, asynchronous exact-line mapping and scoped local/relay
  caching. Add isolated native Git and paired loopback HTTP smoke coverage.
- Add Markdown file tabs with editable Visual/Raw modes, a selection-aware
  formatting toolbar and previews backed by the current unsaved source. Preserve
  undo/redo, save/conflict handling and per-tab state without lossily serializing
  the document. Block automatic images/resources, confirm HTTP(S) links and bound
  preview/format sizes. Add isolated real-buffer editing/undo/save/resource smokes.
- Add a read-only repository-state view in the Git tab for local and relay
  projects: stash entries, linked/detached/locked/prunable worktree metadata,
  per-worktree operation markers and unmerged index stages. Add a scoped,
  project-granted `/v1/git/overview` endpoint, bounded payload validation,
  preservation/parity fixtures and an authenticated stash/worktree smoke check.
  Inventory reads are non-mutating; separately confirmed actions are described above.
- Add read-only Git history through relay: versioned graph, commit comparison and
  per-file patch endpoints, project grants, repository-scope pinning, device/grant
  rechecks and a four-read concurrency limit retained after HTTP cancellation.
  Desktop uses the same history tab with bounded responses, no redirects and no
  local/legacy fallback. Remote branch creation remains disabled. Add local/relay
  parity, authorization, stale scope, cancellation and transport regression tests.
- Move the local Git DAG from its overlay into a project-scoped workbench tab.
  Add full commit messages, changed-file lists and bounded immutable per-file
  patches, including root commits, explicit merge-parent comparisons, renames,
  deletions, binary/mode-only and symlink changes. Preserve Chat/dirty buffers,
  reject stale async results and retargeted roots, and retain mutation ownership
  after closing the tab. External diff/textconv/lazy fetch remain disabled.
- Add persisted group defaults and a cycle-safe, source-aware resolver for global
  → ancestor groups → project precedence. Edit them from a group's context menu;
  retain separate Rename, show per-value origins in project/group settings, and
  reset explicit overrides without copying inherited values. Merge models per
  harness and environment per name, validate combined limits, and preserve group
  defaults through reparenting/deletion/Undo. Schema v7 preserves existing data.
  Group agents still enforce their strictest member's effective client policy;
  corrupt ancestry fails closed. No connected agents are restarted.
- Add an explicit **Apply saved model to current session** action using the
  effective project/group default for the current harness. Require an idle ACP
  session and an advertised, visible model; preserve other conversations and saved
  defaults on rejection, without sending prompts or restarting the process.
- Plan the remaining subsystems in `docs/IMPLEMENTATION-PLAN.md`, with dependencies,
  acceptance criteria and explicit relay/security boundaries.
- Add **Project settings → Model & environment…** for local projects: per-harness
  default ACP model IDs and validated literal child-process environment overrides.
  Select defaults before activating new sessions; refuse unavailable/rejected
  defaults rather than silently sending prompts to another model. Resumed sessions
  retain their model. Environment changes take effect at the next local connection,
  never mutate the app environment or travel through relay, and do not restart agents.
  Environment values are stored unencrypted locally, not in Keychain; not for secrets.

### Changed

- Default Markdown to an editable rendered document with a Raw switch, replacing
  Source/Preview/Split. Keep visual editors mounted so a first click positions the
  caret without changing text size or layout. Edit table cells, fenced code, prose,
  entities, multiline lists/quotes and image descriptions; unsupported syntax
  remains editable literally rather than being rewritten by a serializer.
- Simplify Markdown controls to one mode toggle, grouped formatting icons and an
  Insert menu. Add contextual table row/column operations, alignment, Tab/Shift+Tab
  cell navigation and final-cell row append with stale/locked-file guards.
- Reduce Markdown typography and spacing to editor proportions. Add native Enter
  splits, compatible boundary Backspace/Delete merges, source-mapped selection and
  canonical undo/save while preserving Shift+Enter and fenced-code blank lines.
- Refine the Git inspector with flat filename-first rows, file icons, colored
  statuses, bounded file lists, compact commit headers and quiet borderless controls.
  Reveal branch creation on demand and retain explicit merge-parent navigation.

### Fixed

- Prevent the Markdown Insert dropdown from drawing a bright rectangular backdrop
  by using standard non-modal menu rows instead of modal foreground probes.
- Keep Git-modified buffers and their undo history protected after watcher events,
  dismissed conflict dialogs and failed reloads; only explicit reload replaces text.
- Reject non-USD, missing, negative and non-finite ACP cost reports as unknown rather
  than labeling them USD. Deny tool grants after budget stop/expiry/storage failure,
  but let the final reserved client request finish when only its request cap is spent.
- Refuse stale comparison starts, changed adapter/settings/budget consent and hook-
  modified checkout pairs. Retain partial trees, never replay paid work on restart,
  and keep release tombstones until older overlapping budgets are forgotten safely.

## [0.1.0-beta.2] — 2026-10-30

Project/workbench expansion, safer local Git workflows, portable session history
imports and localized native macOS glass. Existing histories and preferences
are preserved; imports do not resume or send prompts to source sessions.

### Added

- Add preview-first harness session file imports (OpenCode JSON, Claude/Codex
  JSONL and portable custom ACP), canonical project matching, manual assignment,
  source-ID deduplication and atomic insert-only transcript persistence.
- Add a local graphical Git DAG/ref history overlay, commit/ref navigation and
  branch creation from a selected commit without checkout or worktree changes.
- Add embedded custom project icons with automatic-icon reset, a local-time Home
  greeting and native macOS File/View/Repository menus including Settings.
- Reconcile stale development milestones and publish `docs/REMAINING-WORK.md`.
- Add multi-folder/collection repository imports with canonical deduplication,
  bounded read-only scanning, worktree support and persistent partial-import
  reports. Stop at repository roots; do not split monorepos/submodules or switch
  through every imported project. Add declared-monorepo badges.
- Add Design settings grouped into Application / Sidebar / Agent / Editor,
  including a persisted project-list branch visibility toggle. Enlarge Settings
  responsively with rounded, panel-local native glass in both themes.
- Add nested logical project groups, persistent List/Tree views, full group-path
  assignment labels, name/modification sorting and group reparenting by drag/menu.
  Reject cycles, recover visible orphan/cyclic groups and migrate existing groups
  to roots. Promote direct memberships transactionally on deletion; atomic Undo
  preserves later user moves and existing frozen group-agent contexts.
- Add a persistent, window-clamped 220–560-point sidebar resize edge with a
  double-click reset to 278 points.
- Show last branch commit committer dates and upstream observations on recent
  Dashboard tiles. Cache bounded, non-mutating remote checks only for visible
  trusted local projects; provide manual refresh and distinguish cached counts,
  unknown remote objects, missing branches and failed checks without fetch/pull.
- Add searchable quick branch selection to Dashboard project tiles, Source Control,
  the status bar and command palette. Require explicit saving of unsaved buffers
  and separate consent to carry staged/unstaged/untracked Git changes; revalidate
  repository scope/snapshot and block conflicting agents/mutations and quitting
  during checkout. Add authenticated, project-granted safe relay endpoints with
  no fallback to older checkout APIs.
- Add a right-side file minimap with click-to-scroll, a visible viewport, local
  Git HEAD change markers and a separate unsaved-edit rail. Compute bounded
  overviews off-thread, retain per-tab state and persist Settings → Editor's
  File minimap visibility without resetting existing preferences.
- Automatically discover bounded local project favicons/logos from HTML,
  web manifests, Next.js routes and conventional assets; share the resulting icon
  across Dashboard tiles, project rows and the switcher. Cache small SVG/PNG/JPEG/
  ICO rasters, refresh in the background and retain monograms for missing icons,
  remote or untrusted projects. No AI calls, config execution or repository writes.
- Add project-scoped Close all file tabs with Save All / Discard / Cancel,
  retaining Chat and running agents.
- Add Settings → Skills with a persisted global instruction library, editing,
  deletion and explicit ownership-checked OpenCode/Claude/Codex/custom exports.
  Preserve unmanaged, symlinked or externally edited files and supporting files.
- Add authentication-aware Commit on AI using the exact last-dispatched
  harness/model/provider, an explicit cost/commit confirmation and a separate
  conversation. Bundle an editable Conventional Commits workflow that respects
  repository policy/history, coherent grouping, hooks and partial staging.
  Retain manual commits when authentication/model identity is unavailable;
  cancel preparation on navigation and prevent competing Dunlin mutations.
- Add project-to-group drops, branch/dirty summaries and multi-project agent
  sessions with persistent fixed root manifests and restrictive approval policy.
- Add independent editor/Chat tabs, real syntax highlighting, save-all and
  multi-tab quit protection; bounded PNG/JPG/SVG and native paged PDF previews.
- Add Explorer rename, system Trash/Finder actions and authenticated, scoped
  relay preview/rename/non-recursive-delete endpoints.
- Add Dashboard with live agent ownership, configurable recorded-activity heatmaps
  and a Dashboard/last-project startup preference. Retain local session runtimes
  across navigation with unique event generations, eight-adapter budgeting,
  scoped permission handling and background-aware shutdown/history deletion.
- Add persistent Account usage collapse, reversible sidebar motion and an
  input-transparent, panel-local native Settings glass.
- Default the agent graph to agents/subagents, revealing only the clicked
  agent's tools, and export the disclosed layout as standalone SVG using a native
  save dialog. Size the canvas/export to include offscreen nodes.
- Add request-level Trace disclosure, Collapse to Requests / Expand all and a
  token column. Persist optional ACP end-turn usage snapshots; attribute only
  known cumulative-counter differences, never context occupancy or guessed
  per-tool/model consumption. Keep unsupported/legacy usage unknown.
- Add a persistent Chat streaming toggle, enabled by default and independent
  of Final answer only, so incoming response chunks can appear before completion.
- Add Explorer file/folder creation from the toolbar and context menus, with
  folder-aware destinations and drag-and-drop target highlighting. Move items
  within a project; copy Finder drops into local projects without deleting
  originals or overwriting collisions. Preflight batches, stage bounded imports,
  protect Git metadata and retarget open editors after successful moves.
- Add authenticated relay endpoints for folder creation, internal moves and
  explicit Git initialization. These operations require the updated relay;
  Finder imports remain local-only.
- Group Chat appearance in its own settings tab: optional local message times,
  recorded agent-turn duration, response-logo toggle, reasoning and final-only
  presentation. Preserve unknown timing in legacy histories instead of inventing it.
- Detect supported local harness CLIs and ACP adapters in PATH and common install
  folders; distinguish missing adapters, offer setup links and rescan without
  running/installing/authenticating an agent or changing a current conversation.
- Add a persistent Chat setting for final-answer-only presentation: hide workflow
  detail and wait for the completed root reply, while retaining errors and
  permission gates. Render assistant Markdown with headings, emphasis, tables,
  lists, links and code blocks without discarding raw history.
- Replace Mention / Tools with an MCP count and popup. Discover local OpenCode
  server status and support reversible, session-only exclusions via permissions,
  effective next turn without restarting or disconnecting shared servers.
- Add an explicit OpenCode Free filter with verified zero-cost models only,
  original ACP IDs, filtered recents and a matching send guard. Exclude unknown
  prices, paid context/cache tiers and Go-only promotions; never fall back to paid.
- Show a dedicated provider picker and direct sign-in action beside the OpenCode
  harness. Other harnesses retain their own authentication with no provider-account
  controls in the composer or settings.
- OpenCode provider/account controls, including OpenCode Go inside the OpenCode
  harness, a native sign-in dialog with masked API-key entry and supported
  browser OAuth methods, and automatic provider/model refresh without a new
  conversation. Credentials are persisted only by OpenCode; no Terminal or
  temporary sign-in scripts are needed.
- Provider-filtered enabled models and three persistent, deduplicated recent
  models per provider/harness/environment. Prevent sending through a different
  billing provider than the selected filter, preserving the composer text.
- VS Code-inspired chat composer with inline environment, harness, mode/approval
  and model dropdowns; centered layout for a new conversation.
- ACP agent-mode discovery and switching through config options or
  `session/set_mode`, including live mode updates.
- Model picker populated by the connected ACP harness, supporting session
  config options and the legacy model-selection extension.
- A remove action in each project's sidebar menu, including the last project.
- Static marketing site in `web/` with a native product showcase, a download
  page, privacy and terms pages, and a generated social card.

### Fixed

- Localize native Liquid Glass to measured panel bounds on macOS 26+, with a
  translucent AppKit Popover fallback on macOS 15+. Render overlays in a
  transparent owner-attached plane so glass stays below sharp panel controls,
  not over the entire app. Settings/branch panels use a 14% tint, delicate borders
  and shadows; branch layouts remain 530 × 460 / 300 × 170. Use only 12% dimming
  and a weak HeaderView material at 14% alpha behind them. Preserve focus, native
  menu actions and prompt leases; fall back to in-window dialogs on plane failure.
  Add isolated debug smoke coverage for native sibling order, Settings resizing,
  branch input focus, close/reopen and normal Quit with a panel open.
- Fix Settings being obscured or appearing as a displaced, clipped fragment on
  macOS. Replace the full-window NSImage blur mask with an even-odd vector layer
  mask in native points, refresh it for native size/backing-scale changes and
  suppress implicit mask animations. Add native offscreen compositing tests for
  the complete foreground cutout at different UI/Retina scales.
- Center Dashboard activity totals/captions, soften light-theme activity dots and
  remove their glow, outline recent-project tiles and pin data caveats to its
  bottom footer. Replace the group-agent button with a header icon beside Edit.
- Hide unstarted session drafts and adjust sidebar counts without deleting
  conversations or hiding established histories on read failures/loading.
- Unify dialog/popover/menu backgrounds around a measured, input-transparent
  backdrop with light dimming and subtle native macOS background blur in both themes.
  Replace Settings-only fixed-size cutouts, keep foreground panels/dropdowns sharp
  through scrolling/resizing and avoid stacking dim layers. Track native prompt
  and file-picker lifetimes without changing confirmation semantics or safety gates.
- Keep the branch picker at a fixed height while loading/filtering or reporting
  errors, scrolling overflowing branches instead of shifting the dialog. Open a
  compact searchable list directly above the status-bar branch badge rather than
  the centered modal, retaining all existing checkout confirmations and locks.
- Remove the session-information band above Chat while retaining reasoning and
  session actions in the titlebar. Neutralize the group-agent button and move
  bottom project-creation controls into Projects/group context menus.
- Center Dashboard content in a bounded column, including recent tiles, activity
  metrics, dots, legend and captions; keep overflow scrollable on smaller windows.
- Harden branch checkout against option/revision injection, ignored-file
  overwrites, stale plans, nested project scope and in-progress Git operations.
  Keep native Git conflict/worktree protection; never automatically stash, reset,
  merge or discard work.
- Render PDF pages at the actual raster scale rather than centering an unscaled
  page; preserve rotation, crop origins and upright orientation. Test bitmap
  colors/locations, not just nonempty output.
- Paint image/PDF previews into the measured viewport independently of intrinsic
  image layout, constrain ancestor flex heights, remove the PNG encoding/decoding
  round trip and cache bounded PDF page rasters for faster navigation.
- Make entire project group sections visible drop targets, add destination labels
  and an opaque floating drag card, and replace the small group-agent icon with
  an explicit Start group agent action showing the project count.
- Place Open project in a dedicated bottom footer on recent-project tiles.
- Remove the Activity section's card background/rounded container on Dashboard,
  keeping its recorded dot map directly on the page background.
- Fit graphic previews and PDF pages to the available viewport without intrinsic
  image dimensions stretching the layout as the window/sidebar resizes.
- Open staged/unstaged Source Control changes as asynchronous request-owned diffs,
  not editor files. Bound and confine new-file previews; never fabricate a staged
  patch for an untracked file or invoke external diff/text-conversion programs.
- Move Dashboard metrics into the titlebar's Chat/Graph/Trace slot, remove the
  activity outline and use the existing theme palette instead of mint/green.
  Show three recent project tiles when no sessions are active, merging persisted
  user visits with recorded request history without counting background output.
- Auto-dismiss file notifications after 2.5 seconds without dismissing newer
  notifications or group Undo. Include the requested filename in preview errors.
- Hide editor Save for clean/read-only buffers and skip no-op saves. Returning
  to the saved text (including Undo) clears dirty state; refresh the saved
  baseline after loading, reloading, autosave and save-all.
- Restyle Dashboard activity as a compact four-row mint dot matrix on an ink
  background, with a restrained glow for high activity, metric pills and a single
  selected seven-day total. Keep one dated dot per day across 144 recorded days;
  never fill the pattern with sample or randomized activity.
- Separate idle/unrecorded intervals between requests from response duration.
  Compress gaps on the Trace axis while retaining their actual elapsed durations.
- Remove connection/model-loading navigation guards without weakening send
  validation. Load Explorer off-thread and route local background events to their
  owning conversation. Pause background queues at turn boundaries until their
  visible provider/model/MCP selection is validated again.
- Keep the editor gutter opaque during horizontal scrolling; remove initial
  Explain codebase/Review changes chips and usage reset-time arrows.
- Remove Follow/zoom controls from Graph/Trace and the session-storage footer.
  Keep plan steps and tools out of the collapsed agent graph, preserve disclosure
  selection by stable IDs, and group Trace children beneath their own request.
- Treat non-repository folders as a normal Source Control empty state, with
  opt-in initialization instead of a fatal Git error. Remove the blue Git
  explanation panel; never initialize, stage or commit automatically.
- Stabilize sidebar limits across session changes with account-scoped caching,
  stable Go keys and a fixed footer height. Keep explicitly labeled cached limits
  during transient request errors; clear cache after reauthorization.
- Increase Trace row spacing with explicit two-line heights at every zoom and
  round both interior panes of Settings so their backgrounds respect its corners.
- Replace the Editor settings' static font labels with persistent installed-font
  pickers. Apply interface/code families immediately, preserve them across theme
  switches, and use a read-only editor for a live font-size/wrap preview.
- Expand Explorer and search-result file icons with type-specific colors for web
  source, scripts, config, manifests, lockfiles, images, data and archives.
- Keep new conversations in a neutral Ready state instead of Waiting. Remove
  automatic demo data from empty Graph / Trace views and the title-bar preview
  badge; only show recorded session activity. Recover verified empty legacy
  drafts without changing real conversation history or pending permissions.
- Simplify model entries to one line without stars, descriptions or wire IDs.
- Preserve ACP message IDs and child-session origin, group child output separately
  and merge interleaved chunks into the correct answer. Distinguish real subagents
  from tool calls / plan items in Graph and nest tools beneath their owner; retain
  child events in Trace. Do not label every work item as an extra agent.
- Follow streaming chat at the live edge with a persistent scroll handle,
  preserve manual scrolling into history, and add Jump to latest. Prevent the
  transcript's content from shrinking to viewport height behind the composer.
- Explain when an older running OpenCode service lacks usage-limit support
  (requires 2.2+), distinguish that from transient request failures, and provide
  inline Refresh / Open Console actions rather than a generic unavailable message.
- Scope sidebar/account usage to the active environment, harness and OpenCode
  provider, with guarded asynchronous refreshes. Remove unrelated account rows,
  the fixed 19% OpenCode estimate and 62% rail value, and stop presenting ACP
  context occupancy as subscription usage. Read published Go/Console limits when
  supported; show unavailable without a percentage otherwise. Probe only the
  active local harness and keep remote usage separate from Mac accounts.
- Resolve sign-in from fresh provider integration metadata, restoring preferred
  Console browser OAuth for Go even when the composer catalog is missing. Retry
  empty startup discovery with a fixed bound and refresh after session startup.
- Hide the composer's Sign in action after successful provider connection and
  restore that state from OpenCode's public integration metadata on startup.
  Keep reconnect/add-account actions in the provider menu and retain known
  connection state during catalog refreshes.
- Simplify Chat: one conversation header, configuration in the composer, and
  connection/cost/context statistics in the status bar. Remove duplicate model,
  harness, project and branch labels, repeated ready/up-to-date messages, zero
  command counters and the non-functional Compact placeholder. Keep reasoning
  controls in the conversation header and shorten provider-prefixed model labels.
- Persist newly selected conversations even when Chat is already open. Recover
  stale startup selection only for unused, unavailable-harness stubs by opening
  an existing conversation for the available project/default harness; never
  rewrite a conversation's harness or discard its history.
- Check local ACP commands before spawning them, show an actionable missing-
  adapter notice, and do not auto-retry startup/configuration failures. Group
  repeated historical errors and avoid appending consecutive duplicate errors.
- Keep raw provider errors out of the status bar and composer footer. Preserve
  errors in the conversation/logs and expose notices only in a status tooltip.
- Fix a trace-bar rounding panic at the end of the timeline, including the
  startup crash when restoring the Trace view. Keep existing settings/history.
- Send chat messages with Enter; Shift+Enter inserts a newline. Keep `⌘Enter`
  as an alternative send shortcut.
- Ask to save unsaved edits before switching projects or branches, removing the
  active project, closing the window or quitting with the app menu / `⌘Q`.
- Resume existing local ACP sessions instead of displaying old history over a
  fresh agent context. Preserve history and report unsupported or failed loads.
- Keep session identifiers synchronized across project switches and restore the
  selected conversation before connecting on startup.
- Surface local database read/write failures in a persistent warning instead of
  silently treating them as successful saves or empty history.

### Validation and known limitations

- 311 automated tests passed; 4 optional local-service tests remain ignored.
  Workspace/all-target checks, Clippy with warnings denied and formatting passed.
  Isolated native-window smoke tests cover glass sibling order, resizing, branch
  input focus, close/reopen and guarded Quit with a panel open. These checks do
  not replace visual review or a complete fresh-account/upgrade/accessibility QA.
- macOS 15+ on Apple Silicon; bundles remain ad-hoc signed without notarization.
- Session imports are inert, local history copies, not source-session resumption,
  automatic source discovery/sync or billing scanners. Git history is local-only;
  advanced Git operations and relay history parity remain planned.
- Standalone chat/repository context, terminal, computer use, automations,
  artifacts, visual Markdown editing and a working iOS client remain planned.
  See `docs/REMAINING-WORK.md` for the reconciled backlog.

## [0.1.0-beta.1] — 2026-10-25

The first public macOS beta. Dunlin is an early, opinionated release: local
projects are usable today, the remote relay is a beta, and several planned
surfaces are intentionally absent.

### Added

- Native Rust/GPUI workspace with a real Agent Client Protocol client.
- Harness support for OpenCode (default), Claude Code, Codex and custom ACP
  commands, with per-project overrides.
- Streaming chat with thoughts, plans, tool calls, usage updates, permission
  cards and unified diff previews.
- Prompt queue with edit, remove, reorder and clear; message copy, retry and
  edit.
- File explorer, syntax-highlighted editor, save, autosave, external-change
  detection and conflict recovery.
- Project search and source control backed by the system `git` CLI: status,
  staging, discard, commit, amend, pull, push, branches and read-only diffs.
- Agent graph and trace waterfall projected from the live transcript.
- Session history, transcript snapshots, project groups and per-project
  settings.
- Self-hosted remote relay with pairing, device tokens in Keychain, project
  grants, session leasing, takeover and transcript snapshots.
- macOS notifications, Dock badge, missing-folder recovery and a debounced
  file watcher.
- First-run onboarding, empty states and a **Show keyboard shortcuts**
  setting.
- CI and a tag-driven release workflow producing an arm64 ZIP, DMG, relay
  tarball and SHA-256 checksums.
- MIT license, third-party notices and community documentation.

### Known limitations

- macOS 15+ on Apple Silicon only.
- Builds are ad-hoc signed until an Apple Developer ID is available. Gatekeeper
  may require approval in **System Settings → Privacy & Security**.
- The iOS client is a scaffold and is not part of this release.
- No terminal, computer use, automations, skills, artifacts or visual Markdown
  editor yet.
- All relay users share the OpenCode credentials installed on the relay host.
- The relay database is not encrypted at rest; use FileVault or an encrypted
  volume.

[Unreleased]: https://github.com/corapoid/dunlin/compare/v0.1.0-beta.3...HEAD
[0.1.0-beta.3]: https://github.com/corapoid/dunlin/releases/tag/v0.1.0-beta.3
[0.1.0-beta.2]: https://github.com/corapoid/dunlin/releases/tag/v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/corapoid/dunlin/releases/tag/v0.1.0-beta.1
