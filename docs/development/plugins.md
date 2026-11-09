# Plugin packs and knowledge MCP

## Current layers

- API v1 packs contribute metadata, workflows, commands, skills and views.
- API v2 adds a top-level `knowledge` schema and optional `wasm` module.
- The host owns files, SQLite, context, permissions and native UI. Packs cannot
  supply arbitrary GPUI/native code.
- MCP declarations in a generic manifest are data, not a general-purpose server
  launcher. Built-in knowledge MCP injection is a separately implemented path.

Authoritative definitions and examples:

| Interface | Source |
| --- | --- |
| Manifest fields, validation and defaults | [crates/plugin/src/lib.rs](../../crates/plugin/src/lib.rs) |
| Current governance manifest | [assets/plugins/governance/plugin.json](../../assets/plugins/governance/plugin.json) |
| Schema/object/edge types | [knowledge model](../../crates/knowledge/src/model.rs) |
| Frontmatter parsing and legacy IDs | [parser](../../crates/knowledge/src/parse.rs) |
| Bounded context envelope | [context](../../crates/knowledge/src/context.rs) |
| Discovery and WASM execution | [plugin host](../../crates/plugin-host/src/lib.rs) |
| Pure governance operations | [plugins/governance](../../plugins/governance) |
| Desktop install/enable/injection | [plugins.rs](../../apps/desktop/src/plugins.rs) |

Use those structures, not the archived plan's illustrative JSON: contributed
knowledge kinds belong in `knowledge`, not an invented `contributes.knowledge_kinds`
field. Bundled ids cannot be shadowed by external packs. A manifest SHA-256 is
local integrity/pinning evidence, **not a publisher signature**.

## WASM ABI

Core WASM, JSON request/response, no WASI or host imports:

```text
dunlin_alloc(len: u32) -> u32
dunlin_free(ptr: u32, len: u32)
dunlin_call(ptr: u32, len: u32) -> u64
response_ptr = result >> 32; response_len = result & 0xffffffff
```

```json
{"api":1,"op":"validate","kind":"concept","schema_version":1,"input":{}}
```

```json
{"ok":true,"output":{},"warnings":[]}
```

The governance module handles `describe`, `parse`, `validate`, `migrate`,
`context_section` and `template`. Implemented operations do not imply an automatic
user-facing migration/update workflow.

`wasmi` checks exact module SHA-256, fuel, memory, recursion, request/output sizes
and JSON shape. Calls use a dedicated large-stack thread and a fresh bounded
instance. Repeated failures disable the loaded plugin. Defaults are 10,000,000
fuel, 64 MiB memory and 1 MiB output; consult manifest validation for admissible
values. `timeout_ms` exists in the manifest, but do not describe it as a proven
preemptive wall-clock timeout: execution safety relies on the implemented limits.

Startup discovery reads manifests, not eagerly instantiated WASM. Installation
and enablement are separate. Disabling preserves project files and stored rows.
Publisher signatures/catalog, atomic upgrades/rollback and interactive schema
migrations are not guaranteed by the current host. Track missing work in the
[roadmap](../project/roadmap.md), not a checked box in the archived plan.

## Build and inspect

The standalone plugin is excluded from the main Rust workspace and targets
`wasm32-unknown-unknown`:

```sh
./scripts/build-governance-plugin.sh
cargo run -p dunlin -- --governance-smoke dist/governance
cargo run -p dunlin-knowledge-mcp -- --project . --plugin dist/governance
```

The first command writes build output. The final command starts a stdio JSON-RPC
server, not a finished test. `./scripts/install-governance-plugin.sh` writes to
Application Support; Settings → Extensions also supports explicit bundled-pack
installation and validation. Do not install into someone's active app data merely
to validate documentation.

## Knowledge MCP contract

[dunlin-knowledge-mcp](../../crates/knowledge-mcp/src/main.rs) accepts `--project`
and `--plugin` (or `DUNLIN_PROJECT_ROOT` / `DUNLIN_GOVERNANCE_PLUGIN`) and serves
newline-delimited JSON-RPC over stdin/stdout. Read operations:
`knowledge_context`, `object_list`, `object_get`. Proposal operations:
`object_propose`, `relation_propose`, `evidence_link`.

Write tools enqueue pending proposals; they do not modify project files. Desktop
accept/reject performs host-side validation. Local trusted-project injection
requires an enabled governance pack and available helper binary; do not infer
automatic relay or arbitrary third-party MCP hosting from it.

## Validation

```sh
cargo test -p dunlin-plugin -p dunlin-plugin-host -p dunlin-knowledge -p dunlin-knowledge-mcp --locked
```

Add tests for malformed manifests, hashes, imports/traps, fuel/memory/output
exhaustion, malformed JSON and disabled-pack behavior when changing this contract.
User-facing proposals and real-harness discovery still need their own acceptance.
See [knowledge](../user-guide/knowledge.md) and [quality](../quality/README.md).
