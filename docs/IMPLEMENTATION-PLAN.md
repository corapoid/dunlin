# Development plan — updated 2026-11-01 / beta.3

Scope: settings, Git, search, skills, Graph/Trace, design/a11y, onboarding,
terminal/Run, Markdown, standalone chat, SSH, computer use and orchestration.
Starting point: [REMAINING-WORK.md](REMAINING-WORK.md). This document sets the
implementation order; it does not mean the entire backlog is delivered.

New approved scope: [Agent workbench and GitHub](agent-workbench-and-github.md).
It covers all eight proposals and the full CI/PR cycle. GitHub, the inspector,
decision memory, review/checkpoints, checks, durable budgets, A/B comparison and
the Rust AST map have v1 implementations with tests. Full billing, compiler/LSP/non-Rust
impact, manual acceptance and full graph virtualization remain open; do not mark
the entire extension as finished. The current matrix is in [ROADMAP.md](ROADMAP.md).

Source status: `0.1.0-beta.6` + further scope, **592 tests OK / 0 skipped**; build, Clippy
`-D warnings`, fmt and isolated regressions passed. Four optional `#[ignore]`
integration tests were removed. `app.rs` is split into
`app/overlays.rs` (UI and overlays), `app/helpers.rs` (helper functions) and
`app/tests.rs`; shared Services and the event bus remain. Remote usage for the
`member` role shows an explicit `Admin-only` state without a relay request and
without an error toast; the administrator still fetches limits. CI rejects
`todo!()` and `unimplemented!()` in `apps`/`crates`. The older numbers below
describe historical validation stages, not the current set. Beta.3 CI/Release
finished successfully; published artifacts were downloaded and verified without
installation. Beta.4 hardening adds fail-closed gates but does not close red
dependency/policy audits or manual QA: [acceptance status](release-acceptance.md).

## Shared principles

- Small vertical slices: domain model → storage/migration → backend → UI → tests.
- No hidden script execution, paid prompts, fetches, restarts or Git mutations.
  Destructive operations and access expansion require consent.
- Local and relay are explicit capabilities, not pretend parity or a fallback to the Mac.
- I/O work off the UI thread; size/time limits and cancellation for new tasks.
- Every stage: old-data compatibility, regression tests, fmt/clippy,
  a manual keyboard/focus scenario and clear limitations in the documentation.

## Stages and acceptance criteria

### 1. Settings (M) — started

- **1a, implemented; automated tests OK, GUI QA pending:** a local default model
  assigned to a harness, per-project env, validation, saving and editing. Env is
  for the local child process only, without changing the app env and without
  sending it through relay. The model is selected through ACP before a new session
  is offered; resumed sessions keep their model.
  UI: Project settings → Model & environment…; env values are unencrypted in the
  database, not for secrets. Relay model/env remain a separate capability scope.
- **1b, implemented; automated tests OK, GUI QA pending:** separate group settings; resolver global → ancestors from the root →
  group → project. In the UI, the source of each value, reset to inheritance,
  cycle detection and tests for reparenting/deletion/Undo. The icon is not inherited.
  Group agent context still uses the strictest policy of its members.
- **1c, local matrix implemented; GUI QA pending:** local client permissions from the next request;
  model/mode through ACP if the capability allows it; harness/env on the next
  connection. Never an automatic restart of an ongoing turn. Explicit action to apply
  the model to the current idle session and handling denial/busy/parked runtime.
  The **Apply saved model to current session** action does not change other/parked
  conversations, respects provider/model filters and does not send a prompt. Relay settings
  and further adapter-specific settings remain a separate scope.
- 1a acceptance: old projects unchanged, env does not leak between projects,
  an invalid/unavailable model does not send a prompt with another model.
- **1d, implemented; QA pending:** group agents can explicitly add ACP roots
  (`additionalDirectories`) locally or on a single relay. Relay verifies
  every grant on connection and on every request; MCP servers and paths outside the
  grant are rejected, and the sandbox permits only approved roots.

### 2. Git (L) — after establishing the shared operation contract

- **2a, implemented; automated tests OK, GUI QA pending:** history as a main-panel tab, file list and commit diff
  (root/merge/rename/binary/large files); preserving the current safe branch create.
  Scope and manual QA: [Git history](git-history.md). The snapshot is bounded,
  independent of Chat/files; read-only relay transport is delivered by stage 2e.
- **2b, read-only implemented; GUI QA pending:** stash/worktree list, locks/prunable,
  merge/rebase/cherry-pick/revert/sequencer state and index conflicts, locally and through
  relay. Reading alone does not mutate the repository; apply/remove/continue/abort
  actions are delivered by stage 2c. [Scope and QA](git-repository-state.md).
- **2c, implemented; manual QA pending:** stash push/apply/pop/drop, worktree
  add/remove/prune, merge/rebase/cherry-pick, continue/abort; preflight, confirmations,
  global UI lock and shared Git dir lock, buffer preservation. Local and relay
  (one-shot server plans). No force/reset/auto-stash. Details of safety boundaries,
  limited cases and the contract: [Git operations](git-operations-and-blame.md).
- **2d, implemented; manual QA pending:** inline blame for the cursor line in the source
  editor, HEAD/file/backend cache, hover + jump to commit, without attributing
  authorship to changed/unsaved lines. Local/relay, bounded reads.
- **2e, read-only implemented; GUI QA pending:** versioned,
  authenticated endpoints with project grant, timeouts and limits. Tests for
  authorization/path traversal and parity with the local backend; no fallback.
  Graph, metadata and patch are remote; branch create is still local. The
  concurrency limit of 4 reads holds the slot until the actual Git work finishes.
- Acceptance: mutating operations only in isolated test repositories;
  dirty buffers, hook failure, conflict and a worktree from another process covered by tests.
   Historical Git stage validation: 450 tests OK, 4 optional skipped; build/fmt/Clippy OK,
  native Git dark/light-narrow smoke, Markdown regression and paired loopback HTTP
  OK. Manual dialogs/hover/a11y and the target remote host are still pending.

### 3. Global search (M) — functionally parallel with Git, after the storage spike

- FTS5 sessions/transcripts: versioned index, backfill outside the UI, incremental
  update/delete/import in a transaction. Do not index keys/env or hidden reasoning.
- One query and result model: text, project, group, harness, dates, type;
  snippets, ranking, pagination, cancellation and opening the right message.
- Acceptance: old databases, Unicode/FTS characters, combined filters, deletion and large histories;
  an index error does not destroy the source transcript. Relay as a separate capability.

### 4. Skills (M/L) — after the settings resolver

- Read-only discovery of known library roots; folder/repo import as a preview
  with a file list and limits, without executing code. Git ref pinned to a SHA.
- Provenance, hash, version, ownership and diff before update; rejection of
  symlinks/path traversal; external files are never overwritten/deleted.
- Enable per project, explicit installation into a compatible harness root and rollback.
- Acceptance: name conflicts, external change, missing trust and offline import.

### 5. Graph/Trace (M/L)

- Shared pan/zoom viewport and stable IDs, then deterministic layout,
  minimap, search with reveal and preserving selection after updates.
- Keyboard: select/enter/leave hierarchy, collapse, fit, reset;
  Trace: sticky axis and row/span virtualization. Export independent of the viewport.
- Acceptance: 10k-element fixture, bounded rendering work, no focus loss
  and stable layout during streaming, transformation and hit-testing tests.
  Done: depth-unlimited tidy-tree, graph viewport and minimap virtualization,
  virtual trace rows, ARIA tree/active descendant and focus with
  scrolling; automated fixtures and the manual matrix are described in
  [graph-trace-virtualization.md](graph-trace-virtualization.md).

### 6. Design and accessibility (M, continuous through all stages)

- View inventory → semantic typography/spacing tokens → migration.
  Compact/Comfortable as a setting, without shrinking hit targets.
- Audit Tab/Shift-Tab/Esc/Enter, dialog focus trap/restore, contrast,
  AX labels and reduce motion; checklists for both themes and the smallest window.

### 7. Onboarding and permissions (M)

- Resumable wizard: harness → model/auth → project/trust → optional relay/SSH.
  Skipping and returning without data loss; no prompt during configuration.
- macOS center: Notifications/Accessibility/Screen Recording/Automation/FDA,
  distinguishing denied/not determined/unknown, links and recheck after activation.
  Do not promise automatic TCC grants; do not read protected data
  solely to check a permission.

### 8. Terminal and Run (L/XL) — local, relay and SSH v1 implemented

- Chose `portable-pty` + `vt100` (no web libghostty). Local PTY, tabs,
  5,000-line scrollback, resize/SIGWINCH, copy/paste, ANSI/RGB and basic
  keys are implemented. Also implemented: search with highlighting, mouse selection
  (word/line/Shift), splits with a draggable divider, shell integration
  (OSC 133/633/7/1337 through a private zsh/bash wrapper), safe OSC 8 and
  relay transport (authenticated WebSocket + PTY on the host) and SSH v1
  (OpenSSH PTY, ControlMaster, explicit target/path).
- Run: static manifest detection without evaluation; preview argv/cwd/env,
  confirmation, PTY, exit/signal, 50-entry history and rerun are implemented.
- Details and boundaries: [terminal-and-run.md](terminal-and-run.md).
- Manual acceptance: Unicode/IME/bidi, multi-line paste, SIGINT, large output,
  mouse drag in splits, process shutdown, a real relay/SSH host,
  unusual prompts and a11y.

### 9. Markdown and standalone chat (M → XL)

- **Visual Markdown — first scope implemented; GUI QA pending:** by default
  a rendered document with block text/formatting editing and a Raw toggle.
  Shared source/undo/save, without lossy round-trip. Stable fields without size
  change on click; editable tables/code, remaining syntax in fields inside Visual.
  [Scope and limitations](markdown-editor.md). Table row/column operations,
  alignment and Tab/Shift+Tab are implemented; manual QA pending.
  Basic Enter/Backspace/Delete for paragraphs/simple lists
  and compact typography are also ready. Also implemented: selection/editing across blocks, arrows
  with grapheme support, nested lists/quotes and subtree Tab/Shift+Tab,
  link/image properties and explicit, bounded local/relay project resources.
  Automated smoke passed in dark and light/narrow windows; manual QA,
  IME/bidi, complex edge cases and a real paired relay are pending.
- **ChatSpace v1, implemented; manual QA pending:** independent ID/storage
  and scratch cwd (`ProjectKind::ChatSpace`, migration v8), separate history without
  an artificial project in the Projects tree, explicit `New Chat in Repository…`
  selection with confirmation. Repo context creates a new space; existing project
  sessions never silently change cwd. [Scope and boundaries](standalone-chat-worktree-usage.md).
- Acceptance: migrations/history isolation, dirty source when changing views,
  revoking access and resuming a chat without a repo.

### 10. SSH (XL) — terminal transport v1 implemented; relay bootstrap remains

- SSH terminal v1 uses OpenSSH: hosts/aliases from config, `ControlMaster`,
  keys through ssh-agent, interactive known_hosts acceptance, `-tt`,
  `ConnectTimeout` and a private `ControlPath`. Target and path are validated,
  project env/tokens are not sent.
- Remaining: full host and key management in Keychain, confirmed
  relay bootstrap, a loopback-only tunnel, reconnect and process cleanup
  on the host side. Acceptance: offline, MFA, wrong key and interrupted bootstrap.

### 11. Computer use (XL) — after the permissions center

- A separate signed AppKit/Swift helper (AX/ScreenCaptureKit/CGEvent),
  versioned IPC and least privilege; app allowlist, preview and consent
  for sensitive actions, time/action limits, audit without secrets.
- Independent global emergency stop, consent revocation during operation,
  no self-granting of permissions and masking of sensitive fields.

### 12. Multi-agent orchestration (XL, final stage)

- First a durable message bus/inbox with authorization, dedupe and idempotency.
  Then coordinator, DAG and retry/cancel, worktree per task (dependency: Git 2c).
- **Worktree per subtask v1, implemented; manual QA pending:** explicit
  `SubtaskAdd` action creates a `dunlin/subtask/<name>` branch and worktree from the selected
  commit; the durable registry survives restart; no auto-starting the agent, merge or push.
  [Scope](standalone-chat-worktree-usage.md).
- Handoff → review → human gate → merge; conflicts routed to a human,
  full audit, recovery after restart and task board in the graph.
- Model per role and budgets require reliable usage/pricing: unknown/unpriced
  does not mean zero. Cost/concurrency limits also apply to retries and subtasks.
- **Usage attribution v1, implemented; manual log QA pending:** explicit
  `Main/Subagent/Internal/Unknown`, Usage breakdown, copyable source QA report
  and optional aggregate sync through relay (member writes, admin reads).
  [Scope](standalone-chat-worktree-usage.md).
- Acceptance: acyclicity, duplicate messages, crash recovery, trust revocation,
  merge conflict and no automatic exceeding of the approved budget.

## Immediate tasks — recommendation to close started scopes

1. Beta.3 CI/Release and artifacts plus automatic hardening are already confirmed:
   migration without exceptions, solo rules, scans, signed merge and provenance.
   Do not retag beta.3 or retroactively append new validation results to it;
   any new release requires a separate decision.
2. Perform manual acceptance of [settings/groups/Apply model](project-runtime-settings.md),
   [Git local/relay](git-operations-and-blame.md) and [Markdown GUI/IME/bidi](markdown-editor.md).
   Tests/smoke do not replace a real host, dialogs and an accessibility review.
   The user explicitly deferred this matrix only for `0.1.0-beta.N`, without acceptance.
3. Accept [workbench/GitHub](agent-workbench-and-github.md) on isolated data
   and a real account: all eight v1 areas are in code, there is no longer a separate task
   to "implement budgets/comparison/impact from scratch". Explicit extensions remain.
4. Finish the small M1 gaps; the R12 solo/beta scope and remote evidence are closed.
   With user approval, PR, eight app-bound checks and signed squash without bypasses
   were set up in solo mode. Do not pretend to have independent code-owner review
   in a one-person repository.
5. Choose the next vertical scope (FTS5, billing usage or skills), and before
   the terminal perform a VT spike. Standalone chat, scheduler and M6 are separate stages;
   existing checks and A/B comparison do not mean they are finished.
6. New v1 scope — standalone chat/scratch, worktree per subtask, usage
   attribution and billing evidence sync — is in code and automated tests.
   Perform the manual QA described in
   [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md);
   do not mark full M6 or full provider accounting as finished.
