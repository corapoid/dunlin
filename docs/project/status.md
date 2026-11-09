# Product status and boundaries

Reviewed against the source tree on **2026-11-09**. This describes implemented
scope, not a fresh GUI/provider/deployed-host acceptance or a publication claim.
The workspace version is defined in [Cargo.toml](../../Cargo.toml); published
versions are on [GitHub Releases](https://github.com/corapoid/dunlin/releases).

## Supported scope

| Area | Current source supports | Important limitation |
| --- | --- | --- |
| Desktop | Native GPUI macOS 15+ Apple Silicon app; source installation. | No Intel/Linux desktop or restored prebuilt distribution. |
| Agents | ACP catalog/custom stdio, models/modes, permissions, persisted history and imports. | Provider/adapter capabilities vary; saved history is not guaranteed context resume. |
| Workspace | Projects/groups, standalone chats, two panes, editing/previews/search and tabs. | Draft/pane state differs from durable history; no full group/settings cross-device sync. |
| Git | History, mutations, blame, explicit subtask worktrees and public hosting links. | No PR/Actions client; worktrees are not sandboxes. |
| Workbench | Context, decision memory, checkpoints/review, checks, budgets, A/B and Rust impact. | No compiler/LSP-complete impact or guaranteed provider bill cap. |
| Coordination | Project peers, inboxes, reports and human-approved durable deferred turns. | Not autonomous decomposition/model routing/task-DAG spawning or automatic review/merge. |
| Knowledge | Generic file-first objects, optional WASM governance, relation/proposal UI and artifacts. | No arbitrary native plugin UI, trusted publisher catalog or automatic proposal acceptance. |
| Remote | macOS/Linux relay, managed harnesses, grants/leases, project clone/copy, terminal and OpenSSH bootstrap. | Host accounts are not untrusted tenant isolation; real-host acceptance remains separate. |
| Security | Stored/use-only vault, audit artifact readers and explicit audit/check workflows. | Local ACP harnesses are not universally sandboxed; no automatic proof of security. |

## Not current functionality

- Pull Request/GitHub Actions integration and `--github-smoke` were removed.
- Autonomous coordinator/DAG execution, automatic handoff/review/merge, scheduled
  automations and native Computer Use remain future subsystems.
- Complete provider/internal-child billing attribution and global transcript/file
  FTS search are not implied by current usage/project search.
- Plugin signatures, pack catalog/update trust and generic MCP sidecar hosting
  are not implied by installed bundled packs.

## Acceptance and release

Native mocks/fixtures and deterministic tests provide scoped evidence, not a
completed physical-input, VoiceOver, IME/bidi, fresh-account TCC, real-provider or
deployed-host matrix. See [manual acceptance](../quality/manual-acceptance.md) and
feature-specific validation sections. The `0.2.0` policy gap is documented in the
[release process](../quality/release-process.md).

The [roadmap](roadmap.md) is the single remaining-work backlog. Archived proposals
are historical context, not a promise to restore removed features.
