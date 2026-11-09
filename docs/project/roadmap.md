# Dunlin — roadmap

Reviewed on **2026-11-09**. This is the single remaining-work backlog. Source
presence is not manual acceptance; see [product status](status.md) and the
[manual matrix](../quality/manual-acceptance.md). Items are not release-date promises.

## How to read the table

- Priority: **P0** release blocker, **P1** daily-work gap/started scope, **P2** next
  feature slice, **P3** separate subsystem.
- Size: S / M / L / XL. Type: Feature, QA, Release, Refactor or Runtime.
- IDs are retained across this reorganization; gaps do not mean missing content.
- **Removed:** former #41 (GitHub workflow extensions) and PR/Actions-specific
  acceptance from #6. That integration was withdrawn, not an active commitment.
  [Historical scope](../archive/agent-workbench-and-github.md) remains archived.

## Remaining work

| # | Priority | Area | Outcome / remaining scope | Size | Type | Reference |
| ---: | --- | --- | --- | --- | --- | --- |
| 1 | P0 | Release & QA | R11: fresh macOS account, no harness, denied notifications, database upgrade, large project and remote beta; do not mark deferred tests passed. | M | QA | [Manual matrix](../quality/manual-acceptance.md) |
| 2 | P0 | Release | Developer ID/notarization and clean-machine Gatekeeper acceptance before desktop downloads/Homebrew/OCI return. Local source bundles remain ad-hoc signed. | S | Release | [Release process](../quality/release-process.md) |
| 52 | P0 | Release | Explicit acceptance decision/policy for 0.2.0; the 0.1.0-beta.N manual deferral does not authorize it. | S | Release | [Policy gap](../quality/release-process.md#before-authorizing-a-version) |
| 3 | P1 | Runtime defaults | Real-dialog keyboard/focus, cancel/save, validation, model/env inheritance, admin/member boundaries and move/delete/Undo acceptance. | M | QA | [Runtime defaults](../reference/project-runtime-settings.md) |
| 4 | P1 | Git | Git v2 dialogs, graph details, inline blame and buffer protection on isolated data, then a deployed relay. | M | QA | [Mutations](../user-guide/git-operations-and-blame.md), [history](../user-guide/git-history.md) |
| 5 | P1 | Markdown | Visual/Raw IME/bidi, complex nesting and real paired-relay acceptance. | M | QA | [Markdown](../user-guide/markdown-editor.md) |
| 6 | P1 | Workbench | Physical dialogs/keyboard/a11y and deployed-relay checkpoint, comparison, context and check acceptance; no removed GitHub flows. | L | QA | [Workbench](../user-guide/agent-workbench.md) |
| 7 | P1 | Graph & Trace | VoiceOver, keyboard focus, scroll-to-selection, both themes and narrow windows. | M | QA | [Graph/Trace QA](../quality/graph-trace-virtualization.md) |
| 8 | P1 | Terminal & Run | Unicode/IME/bidi, paste/SIGINT, drag selection/splits, shutdown and real relay/SSH host. | M | QA | [Terminal](../user-guide/terminal-and-run.md) |
| 9 | P1 | Usage | Real Claude/Codex/OpenCode log acceptance, accounting gaps and further provider adapters. | M | QA | [Usage](../user-guide/usage.md) |
| 10 | P1 | Skills & Security | Physical skill/security dialogs and explicit real-host audit/Codex CLI acceptance. | M | QA | [Skills](../user-guide/skills.md), [audits](../user-guide/security-audits.md) |
| 11 | P1 | SSH | MFA, changed host key, disconnect/reconnect, remote service cleanup and Keychain removal. | M | QA | [SSH](../administration/ssh.md) |
| 12 | P1 | Conversation | Branch from a selected message without duplicating the entire transcript. | M | Feature | [Conversations](../user-guide/conversations.md) |
| 13 | P1 | Conversation | Undo project/session deletion without modifying repository files. | S | Feature | [Workspace](../user-guide/workspace.md) |
| 14 | P1 | Conversation | Message/tool-call context menus with existing copy/retry/edit boundaries. | M | Feature | [Conversations](../user-guide/conversations.md) |
| 15 | P1 | Conversation | Wrap long composer content without clipping controls or widening the column. | S | Feature | [Conversations](../user-guide/conversations.md) |
| 16 | P1 | Design & A11y | Replace remaining literal typography/spacing sizes with semantic tokens; preserve hit targets. | M | Refactor | [UI controls](../development/ui-controls.md) |
| 17 | P1 | Design & A11y | Physical keyboard/focus/contrast/AX/Reduce Motion audit in both themes and smallest window. | M | QA | [Manual matrix](../quality/manual-acceptance.md) |
| 18 | P1 | Onboarding | Resumable harness/auth/project/trust/relay/SSH wizard with skip/return and no data loss. | M | Feature | [First session](../getting-started/first-session.md) |
| 19 | P1 | Git | Expose safe exact-commit checkout from the graph with confirmation; no moved branch or discarded changes. | S/M | Feature | [History](../user-guide/git-history.md) |
| 20 | P1 | Artifacts | Direct capture on chat/tool cards, diffs and check reports beyond clipboard/current-file capture. | M | Feature | [Artifacts](../user-guide/artifacts.md) |
| 21 | P1 | Evidence | Full cockpit claims/evidence list: status, freshness, support/refute and artifact/task links. | M | Feature | [Artifacts](../user-guide/artifacts.md) |
| 22 | P1 | Standalone Chat | Full recent-repository picker and in-place context switching without creating another chat space. | M | Feature | [Standalone chats](../user-guide/standalone-chat-and-worktrees.md) |
| 43 | P1 | Coordination | Real-adapter MCP discovery, physical approvals/VoiceOver and deployed two-device reconnect/takeover/budget revocation. Durable peers, tasks, inboxes and deferred turns already exist. | M | QA | [Coordination](../user-guide/agent-coordination.md) |
| 53 | P1 | Skills | YAML scalar/block compatibility, validated metadata-preserving import/export and library migration without widening harness/tool permissions. | M | Feature | [Plan 20](../../todo/20-skills-frontmatter-i-odbior.md) |
| 23 | P2 | Search | Local session/transcript FTS5 first: versioned extraction, resumable backfill, privacy/purge, filters, snippets and exact-message navigation; files/relay later. | M | Feature | [Plan 08](../../todo/08-search-everywhere.md) |
| 24 | P2 | Usage | Full retry/subagent/provider accounting and account/global alerts; unknown/unpriced stays explicit. | M | Feature | [Usage](../user-guide/usage.md) |
| 25 | P2 | Projects & Groups | Revisioned runtime settings and opt-in per-user relay group sync: stable IDs, order/appearance, conflicts/tombstones; server group runtime inheritance separately. | L | Feature | [Plan 21](../../todo/21-relay-ustawienia-i-sync-grup.md) |
| 54 | P2 | Runtime defaults | Explicit Apply saved model for the active relay session, with fresh defaults, advertised/allowed model and ACK; no prompt, restart or live env mutation. | S/M | Feature | [Plan 21](../../todo/21-relay-ustawienia-i-sync-grup.md#etap-d--apply-saved-model-na-relay) |
| 26 | P2 | Runtime defaults | Additional adapter-specific settings beyond acknowledged harness/model/env options. | M | Feature | [Runtime defaults](../reference/project-runtime-settings.md) |
| 27 | P2 | Run | Remote detected recipes with the same preview, consent and bounded output as local Run. | M | Feature | [Terminal/Run](../user-guide/terminal-and-run.md) |
| 28 | P2 | Run | More static manifest/script conventions; never evaluate code during detection. | S | Feature | [Terminal/Run](../user-guide/terminal-and-run.md) |
| 29 | P2 | Knowledge | Relation-graph pan/zoom/minimap for larger knowledge graphs. | S/M | Feature | [Knowledge](../user-guide/knowledge.md) |
| 30 | P2 | Governance | Compare harnesses/models on one spec with shared ledger and human preference. | M | Feature | [Spec workspace](../user-guide/spec-workspace.md) |
| 31 | P2 | Governance | Spec-aware review: acceptance/coverage beside diff and reviewer signature bound to commit. | M | Feature | [Spec workspace](../user-guide/spec-workspace.md) |
| 32 | P2 | Governance | Remote intent/spec/contract authoring beyond current read-only indexing. | M | Feature | [Spec workspace](../user-guide/spec-workspace.md) |
| 33 | P2 | Governance | Per-requirement/task costs, preserving unknown accounting. | M | Feature | [Spec workspace](../user-guide/spec-workspace.md) |
| 34 | P2 | Governance | Spec Kit extension hook/command runtime with permissions/timeouts. | M | Feature | [Spec workspace](../user-guide/spec-workspace.md) |
| 35 | P2 | Governance | Automated behavior evals/graders per harness/model; manual eval prompts are not equivalent. | L | Feature | [Spec workspace](../user-guide/spec-workspace.md) |
| 36 | P2 | Governance | Team/relay comments, shared approvals, roles, spec publishing and read-only links. | XL | Feature | [Spec workspace](../user-guide/spec-workspace.md) |
| 37 | P2 | Plugins & MCP | Generic connector/sidecar launch with capability grants, sandbox, audit and kill switch. | XL | Feature | [Plugins](../development/plugins.md) |
| 38 | P2 | Plugins | Publisher signatures/catalog, update trust/renewed consent, private catalogs and explicit schema migration/update lifecycle. | L | Feature | [Plugins](../development/plugins.md) |
| 39 | P2 | Workbench | Compiler/LSP and non-Rust impact mapping beyond current Rust AST/lexical evidence. | L | Feature | [Workbench](../user-guide/agent-workbench.md) |
| 40 | P2 | Workbench | Further check adapters and per-user remote sandbox/quotas. | M | Feature | [Workbench](../user-guide/agent-workbench.md) |
| 42 | P2 | Foundation | Extract explicit shared services/event bus from app-wide state without visible behavior changes. | L | Refactor | [Architecture](../architecture/README.md) |
| 44 | P3 | Multi-agent | Autonomous decomposition, agent/model assignment, task/worktree spawning and unattended relay scheduling beyond approved dependencies. | XL | Feature | [Coordination](../user-guide/agent-coordination.md) |
| 45 | P3 | Multi-agent | Completion-triggered handoff/review/merge and audited conflict routing. | L | Feature | [Worktree contracts](../user-guide/standalone-chat-and-worktrees.md) |
| 46 | P3 | Multi-agent | Per-role routing and cost/concurrency limits including retries/subtasks. | M | Feature | [Coordination](../user-guide/agent-coordination.md) |
| 47 | P3 | Multi-agent | Agent-updated living spec and dependency/status task board. | M | Feature | [Spec workspace](../user-guide/spec-workspace.md) |
| 48 | P3 | Multi-agent | Human handoff/merge/expensive-decision gates and complete audit trail. | M | Feature | [Coordination](../user-guide/agent-coordination.md) |
| 49 | P3 | Automations | Cron/RRULE, precheck/worktree, run history/usage/limits, local LaunchAgent and relay scheduler. | XL | Feature | [Status](status.md) |
| 50 | P3 | Computer Use | Separate signed native helper with AX/ScreenCaptureKit/CGEvent, allowlist, approval, stop and secret-safe audit. | XL | Feature | [Security boundaries](../architecture/security.md) |
| 51 | P3 | Computer Use | Helper TCC onboarding/status/revocation without self-granting permissions. | M | Feature | [Security boundaries](../architecture/security.md) |

## Supporting proposals

[todo/](../../todo/README.md) contains Polish implementation proposals and
dependencies, including newer AI-first workflow ideas. Their own status is
planned/not accepted; they do not prove implementation or override P0 release
gates. Confirm scope against code before promoting a proposal into this backlog.
Keep completion status here rather than maintaining divergent checklists.
