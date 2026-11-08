# Remaining work — overview 2026-11-03

Sources: `ROADMAP.md`, `PLAN.md`, both `TODO-*` lists and the current code.
This is a list of missing scopes, not a copy of stale larger checkboxes.

## Answer to the question about started work

**No — not all of it is finished.** After beta.3, all eight workbench and GitHub
areas have limited v1 code with tests, but manual acceptance and parts of the
original scope are still missing. Git/Markdown/settings are implemented within
the described boundaries, not fully accepted; usage, skills, Graph/Trace,
ergonomics, design/onboarding and the iOS client also have functional gaps.
The detailed "works / remains" matrix is at the top of the [roadmap](ROADMAP.md).

Closed described baseline scopes: queue, watcher, missing folder, basic
notifications/badge, session import and bulk repository import. Their completion
does not close all chat, project or whole-milestone extensions.

Beta.3 source validation: 487 tests OK / 4 optional skipped, build/fmt/Clippy,
native regressions and loopback HTTP OK. Beta.3 CI/Release finished successfully,
the prerelease was published; downloaded artifacts were verified without
installation. Current hardening is not part of beta.3: migration of five obsolete
packages without exceptions and the approved solo rules audit pass. 491 Rust
tests and 25 Python passed; green PR/main CI/Security, signed merge `f61251f`
without bypass and verified attestations for both binaries.
**Current working tree validation after PR #9, the beta.6 scope and the new v1:
592 tests OK / 0 skipped**, build, fmt and Clippy `-D warnings` pass.
The README is a short landing page, and the full description was moved to
[`guide.md`](guide.md); the local performance baseline is in
[`performance.md`](performance.md).
Settings have a single flat surface, two-step onboarding and an ACP catalog;
their manual QA remains open. Four `#[ignore]` tests were removed; `app.rs` is
now split into `app/overlays.rs`, `app/helpers.rs` and `app/tests.rs`. Remote
usage for `member` is explicitly `Admin-only` without a request and without an
error; admin still fetches limits. CI rejects `todo!()`/
`unimplemented!()`. Manual QA is explicitly deferred for the beta series, not
accepted. [Evidence and boundaries](release-acceptance.md).

## Partially implemented — what is still missing

1. **Conversations and ergonomics**: branching from a message (not the same as a
   Git branch), Undo for deleting projects/sessions, context menus for messages
   and tool calls, refining composer wrapping in a narrow window.
2. **Design and accessibility**: semantic tokens, Compact/Comfortable and Reduce
   Motion are implemented (first in Settings and new surfaces). Migration of
   literal sizes in older views and a comprehensive manual audit of keyboard,
   focus, contrast and AX labels remain.
3. **Project/group settings**: local model/env per project and group settings with
   a global → ancestors → project resolver, value sources and inheritance reset
   are implemented. Remote project settings are stored on relay and
   administratively editable; validated env reaches the next harness process,
   and a saved model is applied once ACP models appear. A member sees values but
   cannot save them. Group agents now support explicit multi-root grants: local
   directories through ACP `additionalDirectories` and relay projects on a single
   host through the `roots` list in the ACP connection. Relay verifies every grant
   on connection and on every request, and the sandbox permits only approved
   roots. Manual QA of the new dialogs is still pending.
4. **Groups v2**: manual order, color selection and relay/iOS sync. Hierarchy,
   List/Tree, sorting, drag & drop assignment/group moving and safe
   delete/promote/Undo already work.
5. **Usage**: accurate Claude/Codex/OpenCode log scanners with checkpoints and dedupe,
   billing pricing for cache/long-context/`unpriced`, cache reuse rate, full
   global/project/model/harness breakdowns, budgets and alerts. There are already
   the recorded-data Dashboard, ACP updates and scope-aware account limits; they
   do not replace full billing usage. Durable request/time/reported-USD limits per
   task are implemented; they are not a guaranteed bill cap or full accounting of
   internal retries/subagents. The Claude OAuth usage reader is enabled
   (reading 5h/weekly limits through `api.anthropic.com/api/oauth/usage`).
   The billing foundation (`dunlin_usage::billing`) scans Claude Code JSONL logs,
   Codex rollout JSONL and OpenCode SQLite, deduplicates by message id, checkpoints
   complete JSONL lines, counts cache read/write and prices known models (including
   long-context); unknown models remain `unpriced`. The Usage Dashboard shows
   global/provider/harness/model/project/day breakdowns, and alert budgets
   ($50/$100/$250) warn at 80% and 100% of known cost.
   Remote usage is admin-only on the relay side: the client shows `Admin-only`
   for the `member` role without sending a request and without an error toast,
   while the administrator still fetches limits. This is an intentional permission
   state, not a connection error.
   Attribution v1 (`Main/Subagent/Internal/Unknown`), sources for manual QA,
   a copyable report and optional aggregate sync through relay are implemented;
   manual QA on real logs, full provider accounting and possible further adapters
   remain. [Scope](standalone-chat-worktree-usage.md).
6. **Onboarding and permissions**: the two-step harness → project wizard now has
   an optional notifications card (Allow/Open Settings), a status center for
   Notifications/Accessibility/Screen Recording/Automation/FDA, links to System
   Settings, rechecking and an explicit **Not granted** state. Chat shows a
   project trust banner. Manual TCC QA and the full relay/SSH wizard remain.
7. **Git v2**: commit checkout from the graph and remote branch create are implemented;
     manual QA of mutations/hovers and a real relay remain. Stash/worktree mutations, merge/rebase/cherry-pick,
     continue/abort and inline blame are implemented locally/through relay with preflight,
     one-shot plans and buffer protection — [scope](git-operations-and-blame.md).
     Safe switch, local DAG/refs history, branch creation from the selected commit
     and a graph tab in the main panel with a file list/commit diff are ready in
     code. Root/merge-parent comparisons, rename/delete/binary have tests; manual
     QA of the new panel is still pending.
     Read-only history through relay (graph/details/patches) is implemented with
     grants and scope pinning; remote checks run through `POST /v1/checks/run`
     (explicit argv, 120 s, 512 KiB, no sandbox, for anyone with a grant), and impact
     through `/v1/impact/build` and `/v1/impact/verify`. Remote checkpoints/restore,
     compare with explicit worktree projects, GitHub (admin-only, host `gh` account)
     and context inspector have `/v1/review/*`, `/v1/compare/*`,
     `/v1/github/*` and `/v1/context` endpoints. Mutations/blame outside the existing
     scope and manual QA on a real relay remain.
    Read-only stash/worktree previews (including locks/prunable), operation state
    and index conflicts are implemented locally and through relay; their GUI still
    requires QA.
8. **Global search**: FTS5 transcripts and sessions, shared filters and snippets.
   The current in-project file search does not cover the whole scope.
9. **Skills**: discovery of existing libraries, folder import, per-project
   installation and hash/provenance audit are implemented. The global library,
   editing/validation and safe installation/deletion of own copies into harness
   roots also work. Multi-file packs were added (shared files, per-file hash,
   updates and deletion only of own unchanged copies)
   plus the Cloudflare Security Audit catalog in Settings → Skills and the Security
   section; Codex Security remains an external plugin requiring the Codex harness.
   `dunlin-audit` reads and validates Cloudflare/Codex/SARIF artifacts, shows
   findings, coverage, a report and SARIF export in Security, while Settings → Security
   lets you globally hide a workspace without deleting files. Local triage,
   fingerprint occurrence counter, SARIF/JSON/CSV/Markdown export, fresh
   verifier, run phases, optional Codex CLI scan/patch/verify-fix, local
   seatbelt sandbox for explicit checks and remote start/read of audit artifacts
   through relay are implemented. Security handles only Cloudflare and Codex Security
   artifacts; reports from other scanners are explicitly unsupported.
   Manual UI QA and remote Codex CLI execution on the relay host remain.
10. **Agent graph / Trace**: pan/zoom, minimap, search preserving
    ancestors, `⌘+`/`⌘−`/`⌘0` shortcuts, sticky axis, row-density zoom,
    depth-unlimited tidy-tree, graph/minimap viewport virtualization,
    virtual trace rows, ARIA tree and focus with scroll-to-selection are
    implemented. Automated 10k+ fixtures and the manual matrix are described in
    [graph-trace-virtualization.md](graph-trace-virtualization.md); manual
    VoiceOver/keyboard QA in both themes and a narrow window remains.

## Further backlog — new subsystems and already started extensions

The list below does **not** mean every module is unimplemented. Markdown,
workbench, iOS sources and local terminal/Run v1 already exist; FTS5, the scheduler
and the coordinator do not follow automatically from existing checks/file search/A/B.

11. **Terminal**: local PTY (`portable-pty`) + VT (`vt100`), tabs, bounded
    scrollback, resize, copy/paste, basic keys, search of the visible
    buffer with highlighting, clickable HTTP(S)/mailto/OSC 8 links, mouse
    selection (word/line/Shift), splits with a draggable divider and shell
    integration OSC 133/633/7/1337 are implemented. Relay transport through an
    authenticated WebSocket and SSH v1 through OpenSSH also work. Manual
    GUI/IME/bidi/split QA and the full SSH bootstrap/tunnels and Keychain remain.
    Details: [terminal and Run](terminal-and-run.md).
12. **Run**: static detection of package.json/Cargo/Make/just/pyproject/go/
    deno/composer/Rakefile/mix/gradle/maven and scripts/*; preview argv/cwd/env,
    confirmation, PTY, exit code/signal, history and rerun are implemented. Manual
    QA, more manifest formats and remote Run on relay remain.
13. **Artifacts**: model/storage/panel, explicit "save as artifact", preview/source
    for Markdown/HTML/CSV/images/diffs; later share, expiration and revoke.
14. **Markdown**: default Visual, stable edit fields without typography changes on focus,
    editable table/code/prose cells and syntax fields of other blocks + Raw are
    implemented, as are table row/column operations, alignment and Tab/Shift+Tab;
    basic Enter/Backspace/Delete for prose/lists and compact typography are also ready.
    Selection/editing across blocks, arrows/graphemes, nested lists/quotes,
    subtree Tab/Shift+Tab, link/image properties and explicit local/relay resources
    are also implemented. Smoke passed in dark and light/narrow windows.
    Manual GUI/IME/bidi QA, a real paired relay and complex
    edge cases remain; [exact scope and limitations](markdown-editor.md).
15. **Standalone chat**: ChatSpace/scratch workspace, isolated histories,
    an explicit Chat/repo picker and confirmed repository context are implemented as
    v1. Manual GUI QA, context switching without creating a new space
    and a possible shared recent-repositories picker remain.
    [Scope](standalone-chat-worktree-usage.md).
16. **Automations**: cron/RRULE, precheck, worktree, runs/history/usage/limits,
    LaunchAgent and relay scheduler, work while the app is closed.
17. **SSH**: SSH terminal v1 (OpenSSH PTY, `ControlMaster`, ssh-agent,
    known_hosts, explicit target/path) is implemented. Keychain,
    relay bootstrap, loopback tunnel, reconnect and process cleanup remain; the
    current relay does not mean full SSH.
18. **Computer use**: a separate Swift/AppKit helper, TCC, AX/ScreenCaptureKit/CGEvent,
    allowlist, human approvals, global stop and audit trail.
19. **iOS — started, unaccepted**: SwiftUI/ACP sources and project configuration
    exist; the Xcode build and on-device/relay behavior are not confirmed.
    Group/usage/artifact sync and notifications also remain.
20. **Multi-agent orchestration**: durable message bus/inbox, coordinator and task DAG,
    handoff/review/merge, model routing, per-role budgets, living spec/task board
    and human gates. Worktree per subtask has an implemented v1: explicit branch and worktree from
    a commit plus a durable registry, without auto-starting the agent/merge/push.
    [Scope](standalone-chat-worktree-usage.md).
     The separate A/B comparison already creates two worktrees from one commit, runs
     an identical approved prompt with a shared budget and records the preference
     without merge. It does not replace the coordinator/handoffs.
21. **Agent workbench/GitHub**: eight areas have limited implementations;
    manual QA of dialogs/keyboard/a11y, real GitHub and a deployed relay,
    full billing, compiler/LSP/non-Rust semantics and manual QA of remote
    checkpoints/comparison/GitHub/context remain; checks, impact and the mentioned
    endpoints are already available.
    The impact map works as Rust AST/lexical references and test candidates, not
    as coverage proof. [Exact scope and validation](agent-workbench-and-github.md).

## Foundation and release — separately from features

- `app.rs` is split into `app/overlays.rs`, `app/helpers.rs` and
  `app/tests.rs`; shared Services, models/migrations and the event bus for
  new subsystems from `PLAN.md` remain. `app.rs` still contains app state and logic.
- No `todo!()`/`unimplemented!()` in the sources; unfinished relay paths
  end with an explicit error. CI enforces this rule for `apps`/`crates`.
- **R11 QA**: fresh macOS account, no harness, notification denial, database upgrade,
  large project and remote beta; full GUI verification remains to be done.
  The user accepted deferring GUI/TCC/real host only for
  `0.1.0-beta.N`. [Manual R11 matrix](r11-qa.md); Settings → Advanced →
  Release QA generates a copyable environment report, and `--onboarding-smoke`
  covers a fresh directory without a project/harness and the permissions center
  without replacing TCC acceptance.
- **R12 security/release — closed solo/beta scope**: dependency review/Dependabot, secret
  scanning/push protection and PR/status/no-force rules for `main` already exist.
  The CodeQL/cargo-deny code and metadata/artifact gates and rule audit are already
  prepared locally. Migration of all five obsolete packages without exceptions
  and the integrity of nine forks pass. With user approval, solo rules
  require a PR, eight strict app-bound checks and signed squash merges without
  bypasses; the rules audit passed, and zero independent approvals does not fake review.
  Fresh workflows, signed merge and mainline provenance confirmed;
  zero open security alerts, six documented false positives.
  Developer ID/notarization depend on Apple configuration.

Session file import is ready, but automatic source discovery/sync, native resuming
of imported sessions and relay/SSH import are outside the current scope.
`TODO-project-ui.md` no longer has pending functional items.
