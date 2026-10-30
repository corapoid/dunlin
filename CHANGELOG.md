# Changelog

All notable changes to Dunlin are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/corapoid/dunlin/compare/v0.1.0-beta.2...HEAD
[0.1.0-beta.2]: https://github.com/corapoid/dunlin/releases/tag/v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/corapoid/dunlin/releases/tag/v0.1.0-beta.1
