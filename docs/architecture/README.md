# Architecture

Dunlin is a Rust workspace with a GPUI desktop app, reusable service crates and a
self-hosted relay. The marketing site is separate; no browser runtime renders the
desktop UI. The workspace/toolchain contract is in [Cargo.toml](../../Cargo.toml).

A separate browser-client preview is being built in `apps/web`, not in the Astro
marketing site. Its [implementation plan](../../implement/README.md) tracks the
Tailscale-only live web application. The current read-only catalog bootstrap is
not AHP or a completed workflow runtime; see [preview scope](../../apps/web/README.md).

## Repository map

| Path | Responsibility |
| --- | --- |
| [apps/desktop](../../apps/desktop) | Native app lifecycle, views, feature orchestration and local/remote backend selection. |
| [crates/domain](../../crates/domain) | Shared projects, sessions, settings, events and protocol-facing domain types. |
| [crates/ui](../../crates/ui) | Design system, theme, assets and shared controls on GPUI Kit. |
| [crates/agent](../../crates/agent) | ACP client/worker and deterministic agent smoke binaries. |
| [crates/workspace](../../crates/workspace) | Filesystem, search, Git, review/check/impact and repository operations. |
| [crates/storage](../../crates/storage) | Desktop SQLite persistence, migrations and evidence/index APIs. |
| [apps/cli](../../apps/cli) | Headless local/remote client; catalog reads and fresh one-shot ACP sessions. |
| [apps/web](../../apps/web) | Browser preview: same-origin pairing and read-only WebSocket catalog. Separate React/Vite toolchain; not the marketing site. |
| [apps/server](../../apps/server) | Relay server CLI, pairing/auth/grants, HTTP/WebSocket APIs, remote processes and vault integration. |
| [crates/runtime](../../crates/runtime) | UI-independent transcripts, pending prompts, durable budget dispatch policy and one-shot session execution. |
| [crates/relay-protocol](../../crates/relay-protocol) | Shared v1 pairing, project and session wire contracts. |
| [crates/relay-client](../../crates/relay-client) | Relay HTTP client and local/remote workspace backend; no GPUI or credential persistence. |
| [crates/terminal](../../crates/terminal) | PTY process lifecycle and bounded terminal screen. |
| [crates/usage](../../crates/usage) | Account probes, pricing/billing evidence and attribution. |
| [crates/http](../../crates/http) | Shared HTTP transport helpers. |
| [crates/mcp-client](../../crates/mcp-client) | Host-owned Streamable HTTP MCP client and deterministic Linear adapter; no LLM or credential storage. |
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

## Application and service boundaries

The desktop, CLI and server are independent binaries in one Cargo workspace.
"Core" means the existing service crates plus `dunlin-runtime`, not another
process or a catch-all library. All packages keep one workspace version and lock
file. The desktop remains `dunlin`; the server remains `dunlin-relay` after moving
from `crates/relay` to `apps/server`. Existing installer and MCP commands retain
their names. No daemon is required for local desktop or CLI operation.

Clients import shared services and `dunlin-relay-client`, never server internals.
The server and client share pairing/project/session v1 contracts through
`dunlin-relay-protocol`; adding this crate does not change URLs or wire formats.
Other endpoint contracts are still owned by their existing feature modules.
The protocol crate itself depends only on serde, not SQLite, HTTP or GPUI.

Desktop credential persistence and native Trash integration remain application
adapters. The client receives credentials as arguments and recoverable deletion
as an explicit callback. Its optional private CA must be configured before the
first HTTP request; no client library reaches into desktop acceptance fixtures.

`dunlin-runtime` owns persisted conversation items and stream/child transcript
reducers used by the desktop, the per-session prompt queue and client-dispatch
budget state, plus fresh one-shot ACP execution for the CLI. Queue identities,
timestamps and reviewed skill snapshots survive reordering and failed pre-send
preparation. Clearing a queue does not reset IDs. The queue never authorizes a
send itself; the caller still checks the current model, grants and budget.

Model-default inheritance, validation of an explicitly applied model, and guarded
refresh of relay-owned project defaults also live in the runtime crate. They
operate on wire model IDs and caller-supplied visible choices, never on display
labels or implicit provider fallbacks. Local UI/permission choices and newer
runtime edits are preserved when an earlier remote read completes.

Budget state receives persistence callbacks, not GPUI or a database connection.
Reservations update memory only after a successful durable write; failed/corrupt
storage blocks sending, and restart invalidates stale cost observations. The
desktop retains its existing SQLite key and owns actual storage, notices, vault
revocation, deferred-turn scheduling, provider discovery, imports and UI-specific
settings. These integrations are not claimed as fully extracted yet. The CLI
deliberately does not load desktop budget/settings policies implicitly; see the
[CLI contract and limitations](../reference/cli.md#standalone-cli).

`scripts/check_architecture.py` rejects transitive GPUI or desktop dependencies
in every workspace member except the desktop and its UI crate. Linux headless
tests and native macOS checks remain separate required CI work.

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
