# Dunlin — roadmap

Last reviewed against the `0.1.0-beta.11` source tree on 2026-11-08. This is the
single backlog for Dunlin: completed scopes have been removed after checking the
current code, tests and acceptance notes.

## How to read the table

| Column | Meaning |
| --- | --- |
| **Priority** | `P0` release/acceptance blocker, `P1` close a started scope or a daily-work gap, `P2` next feature slice, `P3` separate subsystem. |
| **Section** | Product area or subsystem the item belongs to. |
| **Item** | A short, independently trackable outcome. |
| **Size** | `S` small, `M` medium, `L` large, `XL` a separate subsystem. |
| **Type** | `Feature`, `QA` (manual/real-host acceptance), `Release`, `Refactor` or `Runtime`. |
| **Remaining scope** | Only what is still missing; already delivered parts are not repeated. |
| **Reference** | Detailed scope, safety boundary or acceptance matrix. |

Rows are ordered by priority and then by product importance. A row is not a
promise of a release date, and manual acceptance is never implied by automated
tests or CI.

## Remaining work

| # | Priority | Section | Item | Size | Type | Remaining scope | Reference |
| ---: | --- | --- | --- | --- | --- | --- | --- |
| 1 | P0 | Release & QA | Release acceptance (R11) | M | QA | Fresh macOS account, no harness, notification denial, database upgrade, large project and remote beta. GUI/TCC/real-host acceptance is deferred for the beta series, not passed. | [r11-qa.md](r11-qa.md) |
| 2 | P0 | Release & QA | Developer ID signing and notarization | S | Release | Apple Developer certificate, notarized DMG/PKG and clean-machine Gatekeeper acceptance. Ad-hoc signing remains the fallback until the account exists. | [release-acceptance.md](release-acceptance.md) |
| 3 | P1 | Settings & Groups | Real-dialog acceptance of runtime defaults | M | QA | Keyboard/focus, validation and cancel/save in project and group editors; admin/member relay behaviour; reserved environment keys; inheritance after move/delete/Undo. | [project-runtime-settings.md](project-runtime-settings.md) |
| 4 | P1 | Git | Manual acceptance of Git v2 | M | QA | Mutation dialogs, graph details, inline blame hover and buffer protection on isolated data, then a deployed relay. | [git-operations-and-blame.md](git-operations-and-blame.md) · [git-history.md](git-history.md) |
| 5 | P1 | Markdown | Manual GUI/IME/bidi acceptance | M | QA | Visual/Raw editing with IME and bidi text, complex nested structures and a real paired relay. | [markdown-editor.md](markdown-editor.md) |
| 6 | P1 | Workbench & GitHub | Acceptance on real GitHub and relay | L | QA | Dialogs, keyboard and accessibility; a real GitHub account; a deployed relay for checkpoints, compare, context and GitHub admin flows. | [agent-workbench-and-github.md](agent-workbench-and-github.md) |
| 7 | P1 | Graph & Trace | Manual accessibility acceptance | M | QA | VoiceOver, keyboard focus and scroll-to-selection in both themes and a narrow window. | [graph-trace-virtualization.md](graph-trace-virtualization.md) |
| 8 | P1 | Terminal & Run | Manual terminal/Run acceptance | M | QA | Unicode/IME/bidi, multi-line paste, SIGINT, drag selection in splits, process shutdown and a real relay/SSH host. | [terminal-and-run.md](terminal-and-run.md) |
| 9 | P1 | Usage | Real-log acceptance and further adapters | M | QA | Real Claude/Codex/OpenCode logs, full provider accounting and additional provider adapters. | [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md) |
| 10 | P1 | Skills & Security | Manual UI and remote audit acceptance | M | QA | Skills/Security dialogs and a remote Codex CLI scan on the relay host. | [secrets.md](secrets.md) |
| 11 | P1 | SSH | Real-host acceptance | M | QA | MFA, changed host keys, network drops, reconnect, remote service cleanup and Keychain removal. | [guide.md](guide.md#ssh-execution-fabric) |
| 12 | P1 | Conversation | Branch from a message | M | Feature | Create a branch that starts from a chosen message without duplicating the whole transcript. | [guide.md](guide.md) |
| 13 | P1 | Conversation | Undo for project/session deletion | S | Feature | Restore a deleted project or session from a toast without touching repository files. | [guide.md](guide.md) |
| 14 | P1 | Conversation | Message and tool-call context menus | M | Feature | Right-click actions for messages and tool calls, with the same copy/retry/edit boundaries as the visible controls. | [guide.md](guide.md) |
| 15 | P1 | Conversation | Composer wrapping in narrow windows | S | Feature | Wrap long composer content without clipping controls or widening the conversation column. | [guide.md](guide.md) |
| 16 | P1 | Design & A11y | Migrate remaining literal sizes to tokens | M | Refactor | Replace hard-coded typography and spacing in older views with the semantic scale; keep hit targets unchanged. | — |
| 17 | P1 | Design & A11y | Manual keyboard/focus/contrast/AX audit | M | QA | Tab/Shift-Tab/Esc/Enter, dialog focus trap/restore, contrast, AX labels and Reduce Motion in both themes and the smallest window. | [r11-qa.md](r11-qa.md) |
| 18 | P1 | Onboarding & Permissions | Resumable harness/auth/project/relay/SSH wizard | M | Feature | A resumable onboarding path covering model/auth, project/trust and optional relay/SSH, with skipping and returning without data loss. | [guide.md](guide.md) |
| 19 | P1 | Git | Safe commit checkout from the graph | S/M | Feature | Expose the existing clean-worktree, exact-commit checkout behind an explicit confirmation; never move branches or discard changes. | [git-history.md](git-history.md) |
| 20 | P1 | Artifacts & Evidence | Direct capture on chat/tool cards, diffs and check reports | M | Feature | "Save as artifact" next to the existing clipboard and current-file capture. | [governance-wasm-plugin.md](governance-wasm-plugin.md) |
| 21 | P1 | Artifacts & Evidence | Full claims/evidence list in the cockpit | M | Feature | Status, freshness, support/refute and links to artifacts/tasks, not only the link from an artifact. | [governance-wasm-plugin.md](governance-wasm-plugin.md) |
| 22 | P1 | Standalone Chat | Recent-repositories picker and in-place context switching | M | Feature | A full recent-repositories picker and changing context without creating a new chat space. | [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md) |
| 23 | P2 | Search | Global FTS5 search | M | Feature | Versioned transcript/session/file index, backfill off the UI thread, filters, snippets, ranking, pagination and cancellation. | — |
| 24 | P2 | Usage | Full provider/subagent accounting and alerts | M | Feature | Account/global alerts and complete retry/subagent accounting; keep unknown/unpriced values explicit instead of zero. | [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md) |
| 25 | P2 | Projects & Groups | Group/settings sync across paired relay devices | L | Feature | Synchronize group definitions and appearance settings without overwriting local-only state or silently widening access. | [project-runtime-settings.md](project-runtime-settings.md) |
| 26 | P2 | Settings & Groups | Further adapter-specific runtime settings | M | Feature | Settings beyond harness/model/env where an ACP adapter exposes a safe, acknowledged option. | [project-runtime-settings.md](project-runtime-settings.md) |
| 27 | P2 | Terminal & Run | Remote Run on a relay | M | Feature | Run detected recipes on a relay project with the same preview, confirmation and bounded output as local Run. | [terminal-and-run.md](terminal-and-run.md) |
| 28 | P2 | Terminal & Run | More manifest formats for Run detection | S | Feature | Additional static manifests and script conventions; never evaluate project code during detection. | [terminal-and-run.md](terminal-and-run.md) |
| 29 | P2 | Knowledge & Governance | Knowledge graph pan/zoom and minimap | S/M | Feature | Navigate larger relation graphs without losing selection or focus. | [governance-wasm-plugin.md](governance-wasm-plugin.md) |
| 30 | P2 | Knowledge & Governance | Compare harnesses on one spec | M | Feature | Run the same intent/spec with two harnesses/models, one shared ledger and a human preference. | [spec-workspace.md](spec-workspace.md) |
| 31 | P2 | Knowledge & Governance | Spec-aware review | M | Feature | Acceptance criteria and coverage beside the diff, with reviewer signature bound to the commit. | [spec-workspace.md](spec-workspace.md) |
| 32 | P2 | Knowledge & Governance | Remote authoring of intent/spec/contract | M | Feature | Create and edit governance objects on a relay, not only read-only indexing. | [spec-workspace.md](spec-workspace.md) |
| 33 | P2 | Knowledge & Governance | Per-requirement/task cost attribution | M | Feature | Cost per requirement/task rather than only per session/project, preserving unknown values. | [spec-workspace.md](spec-workspace.md) |
| 34 | P2 | Knowledge & Governance | Spec Kit extension runtime | M | Feature | Execute Spec Kit extension hooks/commands with explicit permissions and timeouts. | [spec-workspace.md](spec-workspace.md) |
| 35 | P2 | Knowledge & Governance | Automated behaviour evals | L | Feature | Grader, scenarios and results per harness/model; the manual eval prompt is not a substitute. | [spec-workspace.md](spec-workspace.md) |
| 36 | P2 | Knowledge & Governance | Team/relay governance | XL | Feature | Comments, shared approvals, roles, spec publishing and read-only links. | [spec-workspace.md](spec-workspace.md) |
| 37 | P2 | Plugins & MCP | MCP connector host and sidecar | XL | Feature | Launch MCP servers from a manifest with capability grants, sandbox/seatbelt, audit and a kill switch. | [spec-workspace.md](spec-workspace.md) |
| 38 | P2 | Plugins & MCP | Plugin signatures and pack catalog | L | Feature | Publisher keys, pinning, updates requiring renewed consent and private catalogs. | [governance-wasm-plugin.md](governance-wasm-plugin.md) |
| 39 | P2 | Workbench & GitHub | Compiler/LSP and non-Rust impact map | L | Feature | Go beyond the current Rust AST/lexical map; coverage must remain evidence, not a guarantee. | [agent-workbench-and-github.md](agent-workbench-and-github.md) |
| 40 | P2 | Workbench & GitHub | Further check adapters and per-user limits | M | Feature | More test adapters and per-user sandbox/quotas for remote checks. | [agent-workbench-and-github.md](agent-workbench-and-github.md) |
| 41 | P2 | Workbench & GitHub | GitHub workflow extensions | L | Feature | Enterprise hosts, fork-head creation, reviewers/labels, merge queues and a live log stream. | [agent-workbench-and-github.md](agent-workbench-and-github.md) |
| 42 | P2 | Foundation | Extract shared Services and event bus | L | Refactor | Move app-wide state and events out of `app.rs` into explicit services without changing visible behaviour. | — |
| 43 | P3 | Multi-agent | Durable message bus and inbox | L | Feature | Structured messages between sessions with authorization, dedupe, idempotency and SQLite durability. | [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md) |
| 44 | P3 | Multi-agent | Coordinator and task DAG | XL | Feature | Spec/task decomposition, dependencies, assignment to harnesses/models and one worktree per task. | [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md) |
| 45 | P3 | Multi-agent | Handoff, review and merge | L | Feature | Task completion triggers review/merge; conflicts route to an agent or human with a full audit. | [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md) |
| 46 | P3 | Multi-agent | Model routing and per-role budgets | M | Feature | Model per role and cost/concurrency limits that also cover retries and subtasks. | [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md) |
| 47 | P3 | Multi-agent | Living spec and task board | M | Feature | Spec updated by agents with a dependency/status view in the graph. | [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md) |
| 48 | P3 | Multi-agent | Human gates and audit trail | M | Feature | Approval of handoffs, merges and expensive decisions with a complete audit trail. | [standalone-chat-worktree-usage.md](standalone-chat-worktree-usage.md) |
| 49 | P3 | Automations | Scheduler, runs and notifications | XL | Feature | Cron/RRULE, precheck, optional worktree, run history/usage/limits, local LaunchAgent and a relay scheduler. | — |
| 50 | P3 | Computer Use | Native helper and TCC permissions | XL | Feature | Separate signed Swift/AppKit helper with AX/ScreenCaptureKit/CGEvent, allowlist, human approval, global stop and audit without secrets. | — |
| 51 | P3 | Computer Use | Permission helper integration and onboarding | M | Feature | Integrate the helper's TCC status, revocation and onboarding flow without self-granting permissions. | — |
