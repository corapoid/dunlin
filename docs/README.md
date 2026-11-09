# Dunlin documentation

The documents are grouped by what you need them for. Start with the full guide
and the roadmap; the rest are task-oriented references, acceptance evidence and
implementation notes.

## Start here

| Document | What it covers |
| --- | --- |
| [Full guide](guide.md) | The complete feature and operations reference. |
| [Roadmap](ROADMAP.md) | The single remaining-work backlog, ordered by priority. |
| [Screenshot gallery](screenshots.md) | Light/dark captures and feature close-ups. |
| [Performance](performance.md) | Measurement method, baseline and caveats. |
| [Changelog](../CHANGELOG.md) | Versioned release history. |

## Setup and operations

| Document | What it covers |
| --- | --- |
| [Server setup and remote workspace](relay-setup.md) | Relay bootstrap, pairing, Linux service and remote acceptance boundaries. |
| [Settings and project backup](settings-project-backup.md) | Portable JSON or repository-backed settings/project structure. |
| [Secrets and use-only credentials](secrets.md) | Keychain storage, policies, HTTP/CLI broker and relay vault. |
| [Terminal and Run](terminal-and-run.md) | Local/relay/SSH terminals, shell integration and command detection. |
| [Project import](project-import.md) | Local collections, GitHub/GitLab imports and deferred downloads. |
| [Session import](session-import.md) | Importing Claude Code, Codex and OpenCode history with preview and dedupe. |
| [Collection import](collection-import.md) | Repository-collection discovery and import rules. |
| [Standalone chat and worktrees](standalone-chat-worktree-usage.md) | Project-free chat, task worktrees, usage attribution and known boundaries. |

## Feature reference

| Document | What it covers |
| --- | --- |
| [Visual Markdown editor](markdown-editor.md) | Block editing, Raw mode, tables, selection and resource limits. |
| [Git history](git-history.md) | Graph, commit details, diffs and history safety. |
| [Git operations and blame](git-operations-and-blame.md) | Stash/worktree, merge/rebase/cherry-pick, inline blame and consent rules. |
| [Git repository state](git-repository-state.md) | Read-only snapshots of locks, worktrees, sequencer state and conflicts. |
| [Project runtime defaults](project-runtime-settings.md) | Harness/model/environment inheritance, sources and relay boundaries. |
| [Spec workspace](spec-workspace.md) | Intent, spec, tasks, evidence and remaining governance phases. |
| [Governance WASM plugin](governance-wasm-plugin.md) | Knowledge model, manifest v2, WASM ABI, MCP tools, artifacts and SSH fabric. |
| [Session list](session-list.md) | Session scopes, filters, recent activity and workspace binding. |
| [Agent coordination](agent-coordination.md) | Project peers, durable deferred turns, task reports, inboxes, scope grants and relay/device boundaries. |
| [Session tabs and UI preferences](session-tabs-and-ui-preferences.md) | Tabs, panes, zoom, density and persisted UI settings. |
| [Workspace polish and usage](workspace-polish-and-usage.md) | Dashboard, usage presentation and workspace polish decisions. |
| [Native text motion](text-motion.md) | In-place metadata transitions, accessibility, reduced motion and isolated native regression checks. |
| [Shared UI surfaces](ui-surfaces.md) | Central corner geometry, painted child edges, native backdrop alignment and regression checks. |

## Quality and acceptance

| Document | What it covers |
| --- | --- |
| [Test quality and coverage](test-quality.md) | Native UI gate, measured coverage baseline, mutation checks and remaining UI acceptance gaps. |
| [Agent workbench and GitHub](agent-workbench-and-github.md) | Review, checkpoints, checks, budgets, A/B comparison, impact map and GitHub scope. |
| [Graph and trace virtualization](graph-trace-virtualization.md) | Large-graph/trace limits, keyboard behaviour and acceptance notes. |
| [R11 manual QA](r11-qa.md) | The manual release matrix that automated checks do not replace. |
| [Release acceptance](release-acceptance.md) | Published-artifact evidence, hardening and deferred manual acceptance. |
| [CodeQL triage](codeql-triage.md) | Documented false positives and the rule that no query is disabled. |
| [UI input regressions](ui-input-regressions.md) | Historical native input, focus, glass and popover investigations. |
| [Popovers and factory reset](popovers-and-factory-reset.md) | Overlay behaviour and exact reset scope. |

## Repository

| Document | What it covers |
| --- | --- |
| [Contributing](../CONTRIBUTING.md) | Build, test and review expectations. |
| [Security policy](../SECURITY.md) | Reporting and supported-release policy. |
| [Third-party notices](../THIRD_PARTY_NOTICES.md) | Licenses for bundled dependencies, fonts and marks. |
| [License texts](../licenses/) | Full Apache-2.0 text used by bundled dependencies. |
