# Architecture

Dunlin is a Rust workspace with a GPUI desktop app, reusable service crates and a
self-hosted relay. The marketing site is separate; no browser runtime renders the
desktop UI. The workspace/toolchain contract is in [Cargo.toml](../../Cargo.toml).

## Repository map

| Path | Responsibility |
| --- | --- |
| [apps/desktop](../../apps/desktop) | Native app lifecycle, views, feature orchestration and local/remote backend selection. |
| [crates/domain](../../crates/domain) | Shared projects, sessions, settings, events and protocol-facing domain types. |
| [crates/ui](../../crates/ui) | Design system, theme, assets and shared controls on GPUI Kit. |
| [crates/agent](../../crates/agent) | ACP client/worker and deterministic agent smoke binaries. |
| [crates/workspace](../../crates/workspace) | Filesystem, search, Git, review/check/impact and repository operations. |
| [crates/storage](../../crates/storage) | Desktop SQLite persistence, migrations and evidence/index APIs. |
| [crates/relay](../../crates/relay) | Server CLI, pairing/auth/grants, HTTP/WebSocket APIs, remote processes and vault integration. |
| [crates/terminal](../../crates/terminal) | PTY process lifecycle and bounded terminal screen. |
| [crates/usage](../../crates/usage) | Account probes, pricing/billing evidence and attribution. |
| [crates/http](../../crates/http) | Shared HTTP transport helpers. |
| [crates/vault](../../crates/vault) | Keychain/Secret Service credentials, use-only broker and MCP tools. |
| [crates/coordination](../../crates/coordination) | Peer runs/tasks, inbox, durable deferred-turn coordination and MCP. |
| [crates/artifacts](../../crates/artifacts) | Governing artifact parsing/index models and content-addressed blob helpers. |
| [crates/audit](../../crates/audit) | Supported security findings/coverage parsing and exports. |
| [crates/knowledge](../../crates/knowledge) | Generic schema, Markdown/frontmatter objects, graph validation and context envelopes. |
| [crates/plugin](../../crates/plugin) | Declarative manifests, contribution validation and bundled pack catalog. |
| [crates/plugin-host](../../crates/plugin-host) | Hash-checked, bounded `wasmi` runtime and plugin discovery. |
| [crates/knowledge-mcp](../../crates/knowledge-mcp) | Stdio knowledge tools and human-review proposal queue. |
| [crates/ssh](../../crates/ssh) | System OpenSSH lifecycle, passphrase helper, relay bootstrap and loopback tunnels. |
| [plugins/governance](../../plugins/governance) | Standalone WASM governance pack; excluded from the main workspace. |
| [assets](../../assets) | Bundled icons, fonts, skills and plugin packs. |
| [scripts](../../scripts) | Quality, packaging, verification, bootstrap and release tooling. |
| [vendor](../../vendor/README.md) | Reviewed third-party path patches, not first-party application modules. |
| [web](../../web/README.md) | Static marketing site with its own toolchain. |

This is a responsibility map, not a dependency graph. Check each crate's
`Cargo.toml` before changing dependencies; UI-free core crates must not acquire
GPUI or storage dependencies merely for convenience.

## Runtime flow

```text
user action → desktop state / guards → local service or granted relay API
prompt → client envelope / permissions / budget → ACP worker → harness
harness events → channels → transcript → native Chat / Graph / Trace
saved state → SQLite; project/knowledge files remain separate filesystem content
```

ACP protocol I/O belongs to the agent worker, not the render loop. Bounded Git,
filesystem, discovery and indexing work runs in background tasks. Results need
scope/identity checks so late work cannot replace the newly selected project's
state. Local and remote backends must not silently substitute for each other.

The relay owns remote process execution, grants, session leases and transcript
snapshots. The selected harness owns its live ACP session. A snapshot is not
provider context or OS isolation. SSH bootstrap produces an ordinary paired relay
behind an OpenSSH tunnel rather than a second agent protocol.

## State and trust

- [Persistence and lifecycle](persistence.md) — files vs indexes, saved vs transient state.
- [Security boundaries](security.md) — local permissions, relay users, credentials and plugins.
- [Plugin development](../development/plugins.md) — manifest/ABI implementation references.
- [Development](../development/README.md) — contribution and UI conventions.

[Documentation home](../README.md)
