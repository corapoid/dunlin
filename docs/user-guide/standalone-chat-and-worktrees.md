# Standalone chat, subtask worktrees and billing contracts

This reference describes four related contracts, originally delivered in the
2026-11-04 implementation slice:

1. standalone chat / scratch workspace,
2. worktree per subtask,
3. attribution of internal/subagent work in Usage,
4. manual log QA and billing evidence sync through relay.

Acceptance bullets below are test requirements, not a claim of a fresh manual
run. This does not mean autonomous coordination or full provider accounting is
finished. For task-oriented instructions, see [workspace](workspace.md),
[usage](usage.md) and [agent coordination](agent-coordination.md).

## Shared principles

- Domain model and migration first, then backend, UI and regression tests.
- No operation starts an agent, sends a prompt, performs a merge,
  push, fetch or reset without separate, explicit confirmation.
- Unknown data (model, price, attribution, child cost) is never zero.
- Every new storage/sync path has a size limit and an explicit error instead of a silent
  fallback.
- Old projects, sessions and transcripts remain readable.

## 1. Standalone chat / scratch workspace

### Model

- `ProjectKind::{Repo, ChatSpace}` with `#[serde(default)] = Repo`.
- SQLite migration v8 adds `projects.kind` with the default `repo`; existing
  projects remain repositories.
- A chat space has its own `project.id`, its own sessions and transcripts. It is not an
  artificial project in the Projects tree.
- Scratch workspace: `DUNLIN_DATA_DIR/chat-spaces/<chat-id>` (or the default
  app data directory). Files are not deleted when the conversation is deleted.
- Optional repo context: a separate chat space with explicit user consent and
  the `path` of the selected folder. This is a new space with isolated history; existing
  project sessions never silently change `cwd`.

### UI and behavior

- Command palette: **New Standalone Chat**, **New Chat in Repository…** and
  **New Chat in &lt;repo&gt;** for each configured local project
  (search by name/path in the palette).
- Sidebar: a `CHATS · SCRATCH WORKSPACES` section above projects; a chat space is not
  counted as a project and does not appear in `Switch to <project>`.
- Selecting a chat space uses the same composer, harness, model and permission mode
  as a regular chat.
- A scratch chat without `.git` does not open Git history; it shows an explicit message and
  points to `New Chat in Repository…`.
- History remains separated by the session's `project_id`; project sessions are not
  re-pinned. The composer draft remains session behavior, not a shared project
  buffer.

### v1 boundaries

- One chat space = one context (scratch or the indicated folder). Changing
  the context creates a new space instead of mutating existing sessions.
- No automatic repository discovery or disk scanning.
- A chat space is not a sandbox; the normal permission mode applies.

### Acceptance

- Migration v8 on a v5/v6/v7 database preserves projects, sessions and transcripts.
- A scratch chat creates a directory and a separate session; the repo project does not get
  this session.
- `New Chat in Repository…` requires confirmation and does not change the `cwd` of any
  existing project session.
- Restarting the app preserves the chat space selection and its history.
- Manual QA: keyboard/focus, empty state, missing data dir, deletion of the context
  folder, long transcript.

## 2. Worktree per subtask

### Model and Git

- New action `Action::SubtaskAdd { name, revision }`.
- Preflight: name validation, commit resolution, safe directory
  `dunlin-worktrees/subtask-<name>`, no collision of the branch
  `dunlin/subtask/<name>` or the directory.
- Execute: `git worktree add -b dunlin/subtask/<name> -- <path> <commit>`.
  No agent, merge, push or deletion.
- `dunlin_workspace::subtask::Registry` stores task, branch, path, base and
  project id; it validates duplicates and the 256-entry limit.
- The registry is saved in SQLite settings (`subtask-worktrees-v1`) and
  survives restart.

### UI

- Command palette: **Subtask Worktrees**.
- Form: task label, worktree name, commit/branch/tag.
- Confirmation shows the branch, path, base commit and explicit boundaries.
- Entry list: directory presence, branch, base, **Open as project** and
  **Remove worktree…**.
- Removing a worktree requires a clean state and separate confirmation; the branch
  remains. Remote removal in this v1 is explicitly rejected.

### Acceptance

- Repository test: exactly one branch and one worktree from HEAD are created.
- Duplicate name/branch and unsafe name are rejected in preflight.
- An old plan after changing the repository does not perform mutations.
- Manual QA: hook failure, dirty worktree, locked worktree, missing `git`,
  opening as a project and restarting the app.

## 3. Usage: attribution of internal/subagent work

### Model

- `WorkKind::{Main, Subagent, Internal, Unknown}` and `Attribution` with
  `session`, `parent`, `agent`, `task`.
- `BillingEvent.attribution` is serialized; old evidence without the field
  gets `Unknown`, not `Main`.
- Heuristics:
  - Claude Code: `isSidechain`, `agentId` → subagent; `<synthetic>`,
    `isCompactSummary`, `isApiErrorMessage` → internal; the rest with `sessionId`
    → main.
  - Codex: `session_meta.parent_thread_id`/`parent_id`/`source.subagent` →
    subagent; `source`/`originator` internal → internal; no session → unknown.
  - OpenCode: `session.parentID` → subagent; no `sessionID` → unknown.
- `Totals` counts events and known cost per category. `Summary` has
  `by_attribution`, and alerts explicitly warn about unknown.

### UI

- Usage shows **Subagents** and **Unknown attribution** metrics.
- **By work attribution** breakdown.
- Unknown is not counted as main work or as zero known cost.

### Boundaries

- This is not full provider accounting. A provider may not disclose
  child/retry usage; then it remains `Unknown`.
- Heuristics rely only on documented log fields; a new
  format is not guessed.

### Acceptance

- Claude sidechain/synthetic, Codex subagent `session_meta` and OpenCode
  `parentID` fixtures produce the expected categories.
- Dedupe and checkpoints do not change attribution.
- Manual QA: real Claude/Codex/OpenCode logs, mixed sessions, a log without
  session id, an `unpriced` model.

## 4. Manual log QA and billing evidence sync through relay

### Local QA

- `ScanReport.sources` contains provider, path, size, mtime, offset,
  event count and state (`scanned`, `cached`, `skipped`, `error`).
- `Summary.qa_report()` creates a copyable report: totals, attribution, sources,
  unpriced models and warnings. It contains no prompts, responses, env or secrets.
- Settings → Usage & Limits → Cost report: **Copy QA report** and the
  **Log evidence · manual QA** list.

### Relay sync

- `POST /v1/billing/evidence` (member): validates `Evidence` v1, 2 MiB limit,
  save per `(user_id, device_id)`. Accepts aggregates only.
- `GET /v1/billing/evidence` (admin): returns per-user/device evidence.
  A member gets an explicit `403`.
- Client: **Sync evidence** sends the aggregate to every paired relay;
  **Relay evidence** (admin) fetches aggregates. Errors are shown per host.
- No automatic upload: sync is always an explicit user action.

### Acceptance

- Relay test: member writes, member cannot read the aggregate, admin reads,
  invalid evidence is rejected.
- Manual QA: real relay, two devices, repeated sync, no network,
  member vs admin, large report > 2 MiB.

## Implementation history (2026-11-04)

1. `dunlin-usage`: attribution, QA sources, `Evidence`, fixture tests.
2. `dunlin-relay`: evidence table and endpoints + auth/limit tests.
3. `dunlin-workspace`: `SubtaskAdd`, subtask registry, Git tests.
4. `dunlin-storage`/`dunlin-domain`: `ProjectKind`, migration v8, tests.
5. Desktop: chat space, subtask UI, Usage UI/sync, sidebar filters.
6. Documentation, changelog and full validation.

## What this scope does not close

- the full M6 coordinator/DAG/handoffs,
- automatic agent start per subtask and auto-merge,
- a guaranteed provider bill cap,
- automatic discovery and background billing evidence sync,
- remote removal of subtask worktrees and cross-device group/usage synchronization.
