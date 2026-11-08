# Dunlin — roadmap

Legend: `S` small, `M` medium, `L` large, `XL` a separate subsystem.

Statuses: `[x]` = the **described scope** is implemented and automatically
verified; it does not mean the entire parent module is finished or manually
accepted. `[ ]` = code, integration, or acceptance remains. "v1" means an
explicitly limited implementation.

Current review of missing scopes (2026-11-01):
[`REMAINING-WORK.md`](REMAINING-WORK.md). The large unchecked milestones below
may have finished parts — they should not be treated as completely unimplemented.

Implementation order, dependencies, and acceptance criteria:
[`IMPLEMENTATION-PLAN.md`](IMPLEMENTATION-PLAN.md).

## Implementation status

### Working tree overview after `0.1.0-beta.6` — 2026-11-03

**Not all started items are finished.** All eight agent workbench and GitHub
areas have v1 code and tests, but the full accepted scope and the manual
acceptance matrix are not closed. There is no basis for marking all of M1–M6 as
ready or for calculating a completion percentage from checkboxes alone.

| Started area | What works | What blocks closing the full area |
| --- | --- | --- |
| Conversations / M1 ergonomics | Queue, copy/retry/edit, notifications, watcher, missing folder, basic menus | Branch from message, project/session Undo, message/tool-call menus, composer wrapping |
| Settings / M2 groups | Local models/env, inheritance and value sources, explicit Apply model, hierarchy/drag & drop, explicit multi-root local/relay grants | Relay model/env for members, further adapter capabilities, group order/colors/sync and manual QA |
| Usage / budgets | Claude/Codex/OpenCode scanners with checkpoints and dedupe, cache/long-context/`unpriced` pricing, breakdowns and alerts in Usage, account limits, Claude OAuth reader, durable request/time/reported-USD ledger; remote account usage shows `Admin-only` for `member`; `Main/Subagent/Internal/Unknown` attribution, QA sources and aggregate sync through relay (member writes, admin reads) | Manual QA on real logs, full provider accounting and further adapters |
| Git v2 | Local/remote history, commit checkout, remote branch create, stash/worktree, operations and blame with buffer protection | Manual acceptance and the target relay |
| Markdown | Visual/Raw, editing/structure/selection/resources and native regressions | Manual GUI/IME/bidi, complex edge cases and a real relay |
| Agent workbench / GitHub | Eight v1 implementations, local CI/PR and remote checkpoints/compare/context/GitHub admin-only | Acceptance on real GitHub and relay, further capabilities, full billing and compiler/LSP impact; see M3c |
| Skills | Own library, discovery/import, per-project installation, hash/provenance, safe export/deletion, multi-file packs, Cloudflare Security Audit catalog, reading/validating Cloudflare artifacts and Codex Security results, phases, triage, fresh verifier, exports, local checks sandbox and remote audits through relay | Manual UI QA and remote Codex CLI execution on the relay host |
| Graph / Trace | Hierarchies, disclosure, filters/timing, graph export, search with ancestors, minimap, zoom and shortcuts, sticky axis, depth-unlimited tidy-tree, graph/minimap viewport virtualization and trace row virtualization, ARIA tree with active descendant and focus with scroll-to-selection | Manual focus/a11y QA (VoiceOver, both themes, narrow window) |
| Design / onboarding / permissions | Design settings, scale tokens, Compact/Comfortable, Reduce motion, uniform flat Settings, two-step onboarding, ACP catalog, repository languages, resilient Explorer and TCC center | Full migration of remaining literal sizes, SSH/trust after opening a project, manual keyboard/a11y/TCC QA |
| iOS | SwiftUI/ACP client sources and project configuration | Confirmed Xcode build, device/relay test, missing sync and notifications; not a finished client |
| Foundation / release / security | Feature modules, migrations, verified beta.3, migration without exceptions, solo rules, green scans/main CI and attestations; `app.rs` split into `app/overlays.rs`, `app/helpers.rs` and `app/tests.rs` | Shared Services/event bus, deferred R11; a stable release and notarization require separate acceptance |

Local beta.3 validation: **487 tests passed, 4 optional skipped**; desktop/relay
build, Clippy `-D warnings`, fmt, native dark/light-narrow smoke (Git, Markdown,
GitHub, workbench) and paired loopback HTTP Git passed. This does not replace
manual QA or testing a deployed host.

Current working tree validation after PR #9 (beta.4), the beta.5 scope and the
new v1: **592 tests passed, 0 skipped**; build, fmt and Clippy `-D warnings`
pass. Four `#[ignore]` tests
were removed, and remote usage for the `member` role shows an explicit
`Admin-only` state without a relay request and without an error. CI rejects
`todo!()`/`unimplemented!()` in `apps`/`crates`.

### Closed described baseline scopes

The prompt queue, file watching, missing-folder handling, basic macOS
notifications/badge, session file import, bulk repository import, and the current
editor tabs/previews have implementations and tests. Their completion does not
close all of conversations, onboarding, usage, Git, or the full workbench.

Detailed features delivered:

- prompt queue with editing, deletion, reordering and clearing,
- copying and retrying user messages,
- macOS notifications through `UNUserNotificationCenter` and Dock badge,
- debounced file watching for local projects,
- missing-folder detection, badge and `Locate folder…`,
- toast with `Undo` after deleting a group,
- compact sidebar width, `⌘B` collapse and hiding hints in a narrow window,
- prompt editing and retry, unified diff preview on tool calls,
- right-click on projects and sessions,
- inspector as an overlay in a narrow window,
- per-project harness and permission mode override,
- setting to show/hide keyboard hints,
- per-harness session file import with preview, path matching and dedupe,
- local Git/refs graph and a branch from the selected commit without checkout,
- custom project icon, local-time greeting and native macOS menu,
- groups/List/Tree hierarchy, bulk repository import and monorepo badge,
- Design settings, editor with tabs/minimap, image/PDF preview and scoped diffs,
- global skills catalog and installation of own copies into harnesses,
- limited [agent workbench and GitHub](agent-workbench-and-github.md): CI/PR,
  context/decision memory, review/checkpoints/checks, durable budgets, A/B
  comparison in worktrees and an incremental Rust AST map. Full billing, LSP
  semantics, remote new tools and manual acceptance remain open,
- first scale tokens in `crates/ui/src/scale.rs`.
- a short README and the full guide in `docs/guide.md`; the competitive
  comparison and differentiators section were removed, and the verified
  performance baseline is in [`performance.md`](performance.md) (M1: ~0.30–0.36 s to
  first window, 99–106 MiB idle, 0.4% CPU, 42.4 MiB binary).
- a VS Code-inspired chat composer: centered conversation start, expandable
  environment selection (Mac / paired relays), harness, ACP/permission mode and
  model; changing the harness creates a new session, while the previous one
  keeps its harness.

Next in M1: context menus for messages/tool calls, branch from message, composer
wrapping, `Undo` for projects and sessions, notification permission status with
a link to System Settings, and typography tokens in all views.

## Release 0.1 — public beta gate

Source status: `0.1.0-beta.3`, commit `36398df`, tag pushed on November 1, 2026.
[CI](https://github.com/corapoid/dunlin/actions/runs/41291827294) and
[Release](https://github.com/corapoid/dunlin/actions/runs/41291827251) finished
with **success**. The prerelease was published at `2026-11-01T09:19:50Z`; the
downloaded ZIP/DMG/relay have matching SHA-256, version/identity/arm64, valid
ad-hoc signature and DMG verification. The artifacts were not installed.
Current hardening changes are part of **0.1.0-beta.6**: they were not part of
those runs and do not change the existing tag.
[Evidence and closeout blockers](release-acceptance.md).

0.1 scope: macOS 15+, Apple Silicon, local projects and remote relay in beta.
Beta.3 also includes visual Markdown, its own skills library and a limited agent
workbench/GitHub. iOS, terminal, computer use, automation scheduler, artifacts
and full orchestration remain outside the finished scope.

- [x] **R1. Name and identity** — Dunlin, bundle ID `dev.dunlin.ide`, Keychain
  service `dev.dunlin.remote`, data directory `Dunlin`, icon, copyright.
  Old data and tokens migrate on first launch.
- [x] **R2. Onboarding and real data** — demo projects/sessions and the
  compile-time `workspace_root` removed; first-launch screen.
- [x] **R3. Empty states** — no project, no session, no search results;
  an offline harness has a status and a retry action.
- [x] **R4. Window and app** — clamp the saved position to a visible screen,
  single instance, warning when closing during a turn, default macOS app menu.
- [x] **R5. Notifications** — `UNUserNotificationCenter`, authorization on the
  first notification, behavior on denial. Status and the System Settings link
  remain as M1 polish.
- [x] **R6. License** — MIT, `LICENSE`, `THIRD_PARTY_NOTICES`, Geist license,
  `license` change in Cargo and `Info.plist`.
- [x] **R7. Release engineering** — version from tag, arm64 build, DMG/ZIP,
  checksums, optional signing/notarization through secrets.
- [x] **R8. CI** — fmt, clippy, tests, relay smoke on a macOS runner.
- [x] **R9. GitHub Release — mechanism** — `v*` workflow, sources + binaries;
  the current script publishes a release (prerelease for beta versions), not a
  draft. The mechanism succeeding does not confirm the current beta.3 run.
- [x] **R10. Documentation** — README with screenshots and a differentiators
  section, CHANGELOG, CONTRIBUTING, SECURITY, issue/PR templates.
- [ ] **R11. QA** — fresh macOS account, no harness, notification denial,
  database upgrade, large project, remote beta. Manual GUI/TCC/real host are
  explicitly deferred by the user's decision only for `0.1.0-beta.N`, not
  accepted. [Manual matrix](r11-qa.md); an automated report does not replace
  acceptance.
- [x] **R12. Public repo security — approved solo/beta scope**
  - [x] dependency review workflow and Dependabot configuration,
  - [x] GitHub secret scanning and push protection enabled (API read 2026-11-01),
  - [x] `main` rules: PR, eight strict checks bound to the Actions app,
    signed squash merge, no bypasses, no deletion/non-fast-forward;
    explicitly approved solo mode without a fictitious second-person review,
  - [x] CodeQL workflow code (Rust/web/Python/Actions), cargo-deny, artifact
    validator, QA reports and rule audit; confirmed PR/main runs and workflow
    token validation,
  - [x] migration of five obsolete packages across the lockfile, nine maintained
    forks and integrity/provenance; cargo-deny without exceptions, 491 Rust tests,
    25 Python, solo rules audit and nine native smokes passed locally,
  - [x] fresh scans of migrated/fixed sources, signed squash `f61251f`,
    main CI/Security and attestations for both binaries; zero open security
    alerts. Six false positives were documented individually, without disabling
    rules or excluding Rust sources. Merge rules: pass, no bypass.
    [Full matrix and deferrals](release-acceptance.md).

Without an Apple Developer account, the first build will be ad-hoc signed and
marked as an unsigned beta. After an account is added, the workflow will sign
and notarize the artifact.

## Remaining directions after beta.3

- accurate usage scanners/pricing and full breakdowns (the basic Dashboard already works),
- terminal + command autodetection,
- missing Git v2 operations/QA + global FTS5 search,
- acceptance and edge cases for the existing Visual/Raw Markdown,
- standalone chat,
- artifacts,
- discovery/import/per-project skills (the own catalog and export already work),
- automations,
- SSH hosts,
- computer use + TCC helper,
- advanced graph/trace,
- iOS build and sync.

## M1 — Daily work and trust

- [x] **2. Prompt queue** (M)
  - per-session queue, editing, deletion, reordering, clearing,
  - queue panel in the composer, `N queued` counter,
  - dispatch only after `PromptFinished`, without losing prompts.
- [ ] **3. Message actions** (M)
  - [x] copy and retry for user messages,
  - [x] copy agent response,
  - [x] edit and send,
  - [x] unified diff preview on tool calls,
  - [ ] branch from message.
- [x] **4. macOS notifications + badge** (M)
  - permission request and end of turn,
  - only when the window is inactive, on/off setting,
  - Dock badge for pending sessions.
- [x] **5. File watching** (M)
  - FSEvents/`notify`, debounce, ignoring `.git`, `node_modules`, `target`,
  - refreshing the tree, Git and marking an open file as stale.
- [x] **6. Missing project folder** (S)
  - sidebar badge, `Locate folder…`, auto-connect blocked, readable error.
- [ ] **13. Design tokens and UI scale** (M)
  - [x] first tokens in `crates/ui/src/scale.rs`,
  - [ ] typography/spacing tokens in existing views,
  - [ ] Compact/Comfortable density.
- [ ] **15. Toasts with action** (S)
  - [x] toast queue and `Undo` for group deletion,
  - [ ] `Undo` for projects/sessions.
- [ ] **16. Context menus** (M)
  - [x] right-click on project and session,
  - [x] file/folder (create, rename/delete, Reveal in Finder),
  - [ ] message, tool call.
- [ ] **20. Narrow window** (M)
  - [x] compact sidebar and hiding hints in the titlebar,
  - [x] collapsible sidebar (`⌘B`, titlebar button),
  - [x] inspector as an overlay below 1100 px,
  - [ ] composer wrapping.

## M2 — Projects, statistics, onboarding

- [ ] **10. Per-project settings** (M)
  - [x] harness and permission mode override, modal in the project menu,
  - [x] precedence global → project,
  - [x] custom project icon (embedded image, return to automatic),
  - [x] local default models per project/harness and local process env,
  - [x] per-group settings, inheritance global → ancestors → project and value sources,
  - [ ] default model/env through relay,
  - [ ] remaining settings applied automatically without restarting the harness;
    harness selection in the composer already creates a new conversation, and
    global/project defaults apply to new sessions.
- [ ] **11. Groups v2** (M)
  - [x] drag & drop assignments/reparenting, nesting, List/Tree and sorting,
  - [ ] manual order, color selection, relay/iOS sync.
- [ ] **12/28/31. Usage and statistics** (XL)
  - [x] recorded-data Dashboard and scope-aware account limits (not full billing),
  - Claude/Codex/OpenCode scanners, incremental checkpoints, dedupe,
  - pricing with cache and long-context, `unpriced`, cache reuse rate,
  - global/project/model/harness dashboard, sessions, tokens, cost, time,
  - separate "context estimate" from "billing usage".
- [ ] **19. Onboarding** (M)
   - [x] basic first screen, adapter detection, project and trust,
   - [x] optional notifications card, TCC request and **Not granted** state,
   - [x] project trust banner in Chat and QA report in Advanced,
   - [ ] full resumable model/auth/relay/SSH wizard and acceptance.
- [ ] **30. macOS permissions center** (M)
  - Notifications, Accessibility, Screen Recording, Automation, FDA;
  - statuses, opening System Settings, rechecking.

## M3a — Workbench

- [ ] **7. Git v2** (L)
  - [x] safe branch switch and local log/DAG/refs and branch create from a commit,
  - [x] read-only stash/worktree and operation/conflict state, locally/through relay (GUI QA pending),
  - [x] stash/worktree mutations, merge/rebase/cherry-pick and continue/abort with preflight/confirmations, local/relay (manual QA pending),
  - [x] **git graph in the main panel**, details/file list and commit diff
    (root, explicit merge parent selection, rename/delete/binary; GUI QA pending),
  - [x] read-only history through relay: graph, details, files and patches with project grants,
  - [x] selecting a commit in the graph → merge/rebase/cherry-pick form and confirmation,
  - [x] inline git blame on the cursor line with hover and jumping to the commit, HEAD/file/backend cache, and no authorship for unsaved lines,
  - [ ] commit checkout directly from the graph and remote branch create,
  - [ ] manual acceptance of mutations, hovers and a real relay — [scope and limitations](git-operations-and-blame.md).
- [ ] **8. Global search** (M)
  - FTS5 transcripts/sessions + files, filters, snippets.
- [x] **25. Terminal — local, relay and SSH v1** (L)
  - PTY (`portable-pty`) + VT (`vt100`), tabs, bounded scrollback, resize,
    copy/paste, basic keys and ANSI/RGB are implemented;
  - mouse selection (word/line/Shift), splits with a draggable divider,
    shell integration OSC 133/633/7/1337, OSC 8 and relay and SSH v1 transport
    are implemented;
  - manual GUI/IME/bidi/split QA and a real host remain; details:
    [terminal-and-run.md](terminal-and-run.md).
- [x] **26. Command autodetection — v1** (S/M)
  - package.json, Cargo.toml, Makefile, justfile, pyproject, go, deno, composer,
    Rakefile, mix, gradle/maven and scripts/*;
  - "Run" panel, preview/confirmation, PTY, status/exit, history and rerun.

## M3b — Content and extensions

- [ ] **21. Artifacts** (L) — v1 implemented
  - [x] model, blob store, panel, Markdown/image/PDF/diff/JSON preview,
  - [x] capture from clipboard and the current buffer,
  - [x] relay upload/list/download/share/revoke and public token,
  - [ ] direct "Save as artifact" buttons on chat/tool-call cards,
    diffs and check reports,
  - [ ] full claims/evidence list in the cockpit (the link from an artifact already works).
- [ ] **24. Skills catalog** (L)
  - [x] global library, add/edit/validate, safe installation and deletion
    of own copies into harness roots,
  - [ ] discovery of external libraries, enable per project, folder/repo import,
    full hash/provenance audit.
- [ ] **27a. Markdown** (XL)
  - [x] default Visual + Raw, stable typography on click, editable table/code/prose cells and syntax fields of other blocks (GUI QA pending),
  - [x] table row/column operations, alignment and Tab/Shift+Tab between cells (GUI QA pending),
  - [x] compact typography, Enter for paragraphs/simple lists and Backspace/Delete at compatible boundaries (GUI QA pending),
  - [x] selection/editing across blocks, arrows and graphemes, nested lists/quotes and subtree indentation (with validation and limits),
  - [x] link/image properties, explicit local/relay project image preview and confirmed HTTP(S) links,
  - [x] automated QA: unit/transport/security and native dark + light/narrow smoke,
  - [ ] manual GUI/IME/bidi acceptance and a real paired relay; further complex structural cases — [scope](markdown-editor.md).
- [ ] **27b. Standalone chat** (M) — v1 implemented, GUI QA open
  - [x] `ChatSpace`, scratch workspace and `ProjectKind` with migration v8; no
    artificial project in the Projects tree,
  - [x] main composer with environment, harness, model and work mode selection,
  - [x] **New Standalone Chat** and **New Chat in Repository…** with explicit
    context confirmation; no full-disk scanning,
  - [x] isolated history through a separate `project_id`; project sessions
    never silently change `cwd`,
  - [ ] manual GUI QA, a full recent-repositories picker and context switching
    without creating a new space — [scope](standalone-chat-worktree-usage.md).

## M3c — Agent workbench and GitHub — v1 in code, full acceptance open

All items below have an implementation and automated tests. They are not
mockups, but they also do not mean the original proposals are fully realized.
Common missing acceptance: manual dialogs/focus/keyboard/a11y; for GitHub also
a real account/CI/PR. Exact limits and confirmed scenarios:
[agent workbench and GitHub](agent-workbench-and-github.md).

| Area | Delivered v1 scope | v1 boundaries / unfinished extensions |
| --- | --- | --- |
| Task review | Baseline, observed diffs, comment/evidence → draft | No automatic per-turn authorship or test-coverage evidence |
| Context inspector | Real client envelope, provenance, MCP/skills | Harness-owned system/history/context remain explicitly unknown |
| Checkpoints | Named snapshots of saved files, selective restore with confirmation | No RAM/index/refs snapshot; remote checkpoints/restore not implemented |
| Checks / problems | Confirmed JSON argv, result/exit/logs, Cargo diagnostics; remote execution for anyone with a grant | Further test adapters and per-user sandbox/limits |
| Task budgets | Durable reservation of requests, time, reported USD, stop/approvals/retention | Not a provider bill cap; internal retries/subagents may have unknown cost |
| A/B comparison | Two detached worktrees from one HEAD, identical prompt, shared budget, diff/checks/preference | Local-only, no sandbox or auto-merge; does not replace the M6 coordinator |
| Impact map | Incremental Rust AST, lexical dependencies, test candidates and a remote index through relay | Not compiler/LSP/type/cfg-complete; non-Rust remains out of scope |
| Decision memory | Explicit local versions, stale state, evidence and selected entry → draft | No sync; no automatic instruction injection or secret store |
| GitHub CI/PR | GitHub.com + `gh`, job/step polling, logs of finished jobs, confirmed PR/CI operations | No relay/Enterprise, fork-head creation, reviewer/assignee/labels, merge queues or full live log stream |

"Unknown" costs/context/coverage are intentional safety boundaries, not a
result equal to zero or a promise of full functionality. Disabling auto-merge,
auto-send and silent restart is intentional behavior, not a pending task.

## M4 — Remote and autonomous

- [ ] **22. Automations** (XL)
  - cron/RRULE, precheck, worktree, runs, history, usage, notifications,
  - local scheduler + LaunchAgent + relay scheduler.
- [ ] **29. SSH hosts** (XL) — SSH terminal v1 implemented
  - [x] OpenSSH PTY, ControlMaster, ssh-agent/known_hosts, explicit target and path,
  - [ ] Keychain, relay bootstrap, loopback tunnel, reconnect and process cleanup;
  - [ ] terminal and remote projects across the full SSH scope.
- [ ] **23. Computer use** (XL)
  - Swift helper, TCC, AX/ScreenCaptureKit/CGEvent, allowlist, HITL, audit.
- [ ] **30. Full permissions** (M)
  - integration with the helper and onboarding.

## M5 — Visualization and polish

- [ ] **17. Graph** (M)
  - [x] agent hierarchy, collapse/expand and SVG export,
  - [x] pan/zoom, depth-unlimited tidy-tree auto-layout, minimap,
        search and viewport virtualization.
  - [ ] manual focus/a11y QA.
- [ ] **18. Trace** (M)
  - [x] hierarchy, collapse/expand and type filters,
  - [x] search, extended keyboard, sticky axis, row virtualization.
  - [ ] manual focus/a11y QA.
- [ ] **12. Budgets and alerts** (M)
  - [x] durable request/time/reported-USD limits per task (also in the background),
  - [ ] full provider/subagent accounting and account/global alerts.
- [ ] **21. Artifact sharing** (M)
- [ ] **20/13. Accessibility and UI density** (M)
- [ ] **iOS** — group/usage/artifact sync, notifications, Xcode build.
  SwiftUI/ACP sources already exist, but the build and on-device behavior are
  not confirmed; this is a started, unaccepted area, not a finished app.

## M6 — Multi-agent orchestration

Goal: agents not only coexist in sessions but hand work to each other through a
visible, controlled graph. This is an extension of graph/trace and an answer to
the orchestration need known from mature agent-first IDEs, but based on ACP and
our worktrees.

- [ ] **M6.1. Message bus** (L) — structured messages between sessions,
  human inbox, SQLite durability, sender/recipient permissions.
- [ ] **M6.2. Coordinator** (XL) — spec/task → decomposition → dependencies →
  assignment to harnesses/models; each subtask in a separate worktree.
  Worktree per subtask v1 is implemented (`SubtaskAdd`, branch
  `dunlin/subtask/<name>`, durable registry, explicit UI); automatic
  decomposition, agent start and merge are missing.
  [Scope](standalone-chat-worktree-usage.md).
- [ ] **M6.3. Handoff and review** (L) — task completion automatically
  triggers review/merge; conflicts: auto → agent → human.
- [ ] **M6.4. Model routing and budget** (M) — model per role, cost limits,
   usage attribution per task.
  The separate A/B comparison already has model selection, a shared ledger and
  confirmations; it does not mean role routing or a guaranteed bill cap is
  implemented.
- [ ] **M6.5. Living spec / task board** (M) — spec updated by agents,
  dependency and status view in the graph.
- [ ] **M6.6. Human gates** (M) — approval of handoffs, merges and expensive
  decisions; full audit trail.

Dependencies: terminal, Git worktree, skills, automations, usage and graph.


## M7 — Spec workspace, intent and agent governance

Goal: governance artifacts (intent, spec, plan, tasks, agent contract) are
native, versioned workspace objects, not text in chat.
Files remain the source of truth; SQLite is the index and evidence store.

Implemented:

- [x] **M7.1. Intent + spec cockpit** — intent capture/interview, 10 SDD phases,
  Clarify/Checklist/Analyze/Converge, implementation gates, drift and coverage.
- [x] **M7.2. Approval and evidence** — hash-bound decisions, HEAD commit, notes,
  log export, artifact index, evidence per task.
- [x] **M7.3. Relay read-only indexing** — artifacts from relay/SSH projects through
  the existing backend, loaded in the background.
- [x] **M7.4. Task worktree + merge gate** — automatic worktree creation after
  confirmation, `task_id` in the registry, task completion gate and
  commit/branch/GitHub CI/PR evidence.
- [x] **M7.5. Traceability** — interactive requirement → task →
  worktree/evidence/session tree.
- [x] **M7.6. Contract runtime audit** — heuristic tool-call audit,
  permission ceiling, policy lint from `AGENTS.md`, manual eval prompt.
- [x] **M7.7. Global switch** — Settings → General disables intent/spec/contract
  and related pack commands without deleting files.
- [x] **M7.17. Generic knowledge cockpit** — list/detail/relationship graph, filters,
  Accept/Reject suggestions, artifact panel and SSH hosts.
- [x] **M7.18. Governance WASM plugin** — manifest v2, `wasmi` sandbox,
  automatic installation from a directory, `plugin-builder` skill, MCP injection.

Remaining:

- [ ] **M7.8. Compare harnesses on one spec** (M) — the same intent/spec,
  two harnesses/models, shared ledger and human preference.
- [ ] **M7.9. Spec Kit extension runtime** (M) — executing Spec Kit extension
  hooks/commands with explicit permissions and timeouts.
- [ ] **M7.10. Spec-aware review** (M) — acceptance criteria and coverage next to
  the diff, reviewer signature bound to the commit.
- [ ] **M7.11. MCP connector host + sidecar** (XL) — launch MCP from a manifest,
  capability grants, sandbox/seatbelt, audit and kill switch.
- [ ] **M7.12. Signatures and pack catalog** (L) — publisher keys, pinning,
  updates requiring renewed consent, private catalogs.
- [ ] **M7.13. Team/relay governance** (XL) — comments, shared
  approvals, roles, spec publishing and read-only links.
- [ ] **M7.14. Remote authoring** (M) — creating/editing intent and agent
  contract on relay, not just read-only indexing.
- [ ] **M7.15. Cost attribution** (M) — cost per requirement/task, not only
  per session/project.
- [ ] **M7.16. Automated behavior evals** (L) — grader, scenarios and results
  per harness/model; the manual eval prompt already exists.
- [ ] **M7.19. Capture from chat/tool-call cards, diffs and check reports** (M) —
  direct "Save as artifact" next to clipboard and current file.
- [ ] **M7.20. Full claims/evidence list in the cockpit** (M) — status, freshness,
  support/refute, linkage to artifacts and tasks.
- [ ] **M7.21. Knowledge graph zoom/minimap** (S/M) — pan/zoom, minimap and
  larger relationship graphs.

Dependencies: storage, Git worktree, relay grants, skills, usage, M6 handoff.

## Recommended closeout order

1. Beta.3 and automatic hardening are confirmed: migration without exceptions,
   solo rules, scans, signed merge and mainline provenance. A new release requires
   a separate decision; do not retag beta.3 or pretend the deferred R11 acceptance
   happened.
2. Accept the started scopes: settings/groups, Git, Markdown and workbench/GitHub
   on isolated data; separately the real relay, GitHub and GUI/IME/a11y.
3. Close the small M1 gaps (conversation branch, Undo, message/tool-call menus,
   composer) and the indicated R12 gaps, without automatically changing repo rules.
4. Then choose the next functional scope: FTS5, billing usage or skills
   discovery/per-project. Do not start all large modules at once.
5. Terminal/Run, standalone chat, artifacts, scheduler, SSH, computer use and M6
   remain separate stages; the existing checks, relay and A/B do not deliver them for us.
