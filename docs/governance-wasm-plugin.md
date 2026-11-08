# Governance WASM plugin — implementation plan

Status: core vertical slice implemented (2026-11-08); knowledge cockpit, artifacts and SSH fabric implemented; real-host/manual QA remains

Goal: make Intent / Concept / Decision a **downloadable, disabled-by-default
WASM knowledge pack** on top of a generic, evidence-first knowledge core. The
host owns storage, files, UI, context envelopes and permissions. The plugin owns
only the governance schema and pure transformation/validation logic.

## Non-goals in this slice

- SSH execution fabric (separate module, not WASM).
- Artifact share/revoke over relay (separate module; local export remains core).
- Computer use, automations and multi-agent orchestration.
- Arbitrary GPUI or native code from plugins.
- Automatic acceptance of agent proposals.

## Architecture

```
dunlin-domain
  └── dunlin-knowledge        generic objects, schemas, edges, graph, envelope
        ├── dunlin-plugin     manifest v2 + knowledge contributions
        │     └── dunlin-plugin-host  wasmi runtime, ABI, limits, registry
        │           └── dunlin-knowledge-mcp  read tools + pending proposals
        ├── dunlin-storage    knowledge/object/evidence/plugin tables
        └── apps/desktop      generic cockpit + plugin settings
plugins/governance           separate wasm32 cdylib (not a workspace member)
assets/plugins/governance    bundled manifest + wasm catalog (Install automatically)
assets/skills/plugin-builder.md  guided plugin-authoring skill
```

Dependency direction is acyclic. `dunlin-knowledge` is pure (no SQLite, no GPUI).
`dunlin-plugin-host` is the only crate that executes third-party code.

## Data model

### Generic knowledge objects

- `KnowledgeKind`: id, label, directory, statuses, fields, relations, default status.
- `FieldDef`: id, label, kind (`text|markdown|list|enum|bool|number`), required, enum values.
- `RelationDef`: id, label, from kinds, to kinds.
- `KnowledgeObject`: id, project, kind, title, status, path, revision, updated_ms,
  fields (JSON), body, plugin id, schema version, origin.
- `KnowledgeEdge`: from, to, relation, metadata.
- `KnowledgeGraph`: objects + edges + validation report.

### Evidence spine

- `Claim`: id, project, intent/task, statement, status, confidence, scope,
  last_verified_ms, stale_reason.
- `Evidence`: id, project, kind, artifact/run, source, revision, hash,
  observed_ms, freshness, summary.
- `Run`: id, project, session, intent, task, handoff, harness, model,
  permission, worktree, budget, envelope hash, status, timestamps, result.
- `Artifact`: id, project, run, session, intent, task, kind, title, mime,
  bytes, sha256, blob path, visibility, expiry, revocation.

Freshness is computed by the host, never trusted from the plugin:

- `fresh` — scope revision matches current project revision and TTL is valid.
- `stale` — commit/file/command scope changed or TTL expired.
- `unknown` — scope cannot be established.
- Claim status: `unverified|supported|refuted|unknown|stale`.

## File formats

New directories:

```
intent/**/*.md
concepts/**/*.md
decisions/**/*.md
```

Frontmatter:

```yaml
---
dunlin: 1
kind: concept
id: CON-001
title: Workspace
status: accepted
intent: INT-001
owners: [human:corapoid]
confidence: high
links:
  - relation: realizes
    target: INT-001
  - relation: constrains
    target: CON-002
created: 2026-11-08
updated: 2026-11-08
---
```

Rules:

- Files remain canonical. SQLite is a rebuildable index.
- Existing intent files are never rewritten. Legacy files receive a derived
  `INT-<hash>` id until the user explicitly adopts a stable id.
- New files require a stable id in frontmatter.
- `tasks.md` may declare `depends_on: T001, T002`; `[P]` remains a hint only.

## Plugin manifest v2

```json
{
  "id": "dunlin-governance",
  "name": "Intent, Concepts & Decisions",
  "version": "1.0.0",
  "api": 2,
  "min_app": "0.2.0",
  "wasm": {
    "path": "plugin.wasm",
    "sha256": "<hex>",
    "exports": ["dunlin_alloc", "dunlin_free", "dunlin_call"],
    "fuel": 10000000,
    "timeout_ms": 100,
    "max_memory_bytes": 67108864,
    "max_output_bytes": 1048576
  },
  "contributes": {
    "knowledge_kinds": [
      {
        "id": "intent",
        "label": "Intent",
        "directory": "intent",
        "statuses": ["draft", "clarifying", "accepted", "implementing", "converged", "superseded"],
        "fields": [
          {"id": "problem", "label": "Problem", "kind": "markdown", "required": true},
          {"id": "outcome", "label": "Proposed outcome", "kind": "markdown", "required": true}
        ],
        "relations": ["has_concept", "resolved_by", "supersedes"]
      }
    ],
    "context_sections": ["intent", "concepts", "decisions", "open_questions"]
  },
  "permissions": {
    "project_read": ["intent/**", "concepts/**", "decisions/**", "specs/**"],
    "project_write": ["intent/**", "concepts/**", "decisions/**"],
    "network": [],
    "process": false
  }
}
```

API v1 packs remain valid; v2 features are opt-in. Bundled ids cannot be
shadowed by external packs.

## WASM ABI v1

Core wasm, JSON request/response, no WASI, no host imports.

Exports:

```rust
dunlin_alloc(len: u32) -> u32
dunlin_free(ptr: u32, len: u32)
dunlin_call(ptr: u32, len: u32) -> u64 // (response_ptr << 32) | response_len
```

Request:

```json
{"api":1,"op":"validate","kind":"concept","schema_version":1,"input":{}}
```

Response:

```json
{"ok":true,"output":{},"warnings":[]}
```

Operations: `describe`, `parse`, `validate`, `migrate`, `context_section`,
`template`.

Runtime: `wasmi` (interpreter, fuel metering, memory limits). Calls run off the
UI thread. Output is bounded and JSON-validated. Traps are isolated; repeated
failures disable the plugin for the session.

## Lifecycle

1. Startup reads `plugin.json` only; WASM is not compiled or instantiated.
2. Disabled (default): no indexing, no UI, no context sections, no MCP tools.
   Markdown files remain visible in the explorer.
3. Enable: verify manifest signature/hash (future), verify WASM SHA-256,
   install under `~/Library/Application Support/Dunlin/plugins/<id>/<version>/`,
   compile, instantiate, index, expose UI/tools.
4. Disable: drop instances, hide UI/index, preserve all files and rows.
5. Update: side-by-side version, atomic switch, rollback, explicit migrations.
6. Uninstall: remove code; project files stay; index becomes orphaned.

## Context envelope

Host builds a transport-neutral `IntentEnvelope`:

- enabled plugin sections only,
- bounded to 32 KiB, ranked intent → concepts → decisions → evidence → files,
- redacts environment values and secrets,
- visible preview before dispatch,
- hash recorded in `runs.envelope_hash`,
- v1 renderer: explicit prompt block; later MCP resource/ACP metadata.

## MCP tools

Host-owned `dunlin-knowledge-mcp` stdio server (implemented; harness injection remains future):

- `knowledge_context` (read-only envelope),
- `object_list`, `object_get`,
- `object_propose`,
- `relation_propose`,
- `evidence_link`.

Writes are pending proposals. Only a human accepts them. The plugin validates
shape; the host validates permissions and scope.

## Implementation checklist

### M0 — knowledge core

- [x] `dunlin-knowledge` crate: schema, object, edge, graph, envelope.
- [x] Markdown/frontmatter parser and legacy derived ids.
- [x] Graph validation: cycles, unknown relations, orphan edges.
- [x] Context envelope builder with limits and hashing.
- [x] Read-only discovery for `intent/`, `concepts/`, `decisions/`.
- [x] Unit tests for parser, graph, envelope and discovery.
- [x] Storage migration v15: knowledge objects/edges.
- [x] Storage migration v16: runs, artifacts, claims, evidence, evidence links.
- [x] Storage migration v17: plugin registry.
- [x] Storage round-trip and migration tests.

### M1 — plugin manifest and WASM host

- [x] Manifest v2: `wasm`, knowledge schema, context sections; v1 compatible.
- [x] `dunlin-plugin-host`: registry, hash verification, wasmi runtime.
- [x] ABI alloc/free/call, fuel, memory, recursion and output limits.
- [x] `describe`, `validate`, `context_section`, `template`, `migrate` and `parse` ops.
- [x] Large-stack worker so a Rust-compiled plugin cannot abort the app process.
- [x] Trap isolation and repeated-failure state.
- [x] WAT fixture tests and hash mismatch test.
- [x] Governance plugin crate (`plugins/governance`) with pure logic.
- [x] Build script for `wasm32-unknown-unknown`.
- [x] Install script for `Application Support/Dunlin/plugins`.

### M2 — desktop integration

- [x] Settings → Extensions lists the bundled governance pack as available with **Install automatically**.
- [x] `plugin-builder` skill documents manifest v2, ABI, security and tests.
- [x] **Use skills and build plugins** opens a guided chat with the default/last harness.
- [x] Plugin settings show WASM module, hash and knowledge validation.
- [x] Validate WASM action with bounded synchronous smoke.
- [x] CLI `--governance-smoke` proving load → describe → validate → envelope.
- [x] Spec workspace indexes `concepts/` and `decisions/` when the pack is enabled.
- [x] `dunlin-knowledge-mcp` stdio server with read tools and pending proposals.
- [x] Pending-proposal count and folder shortcut in Settings → Extensions.
- [x] Full generic knowledge cockpit UI (list/detail/relation graph).
- [x] Pending-proposal inbox UI with Accept/Reject.
- [x] Harness MCP injection when the governance pack is enabled (local trusted projects).

### M3 — artifacts as evidence (core, not WASM)

- [x] Artifact/claim/evidence/run storage tables and APIs.
- [x] Capture from clipboard and the current editor buffer.
- [x] Native text/image preview and local blob storage.
- [x] Relay artifact upload/list/download/share/revoke and public token route.
- [ ] Direct capture buttons on chat/tool cards, diffs and check reports.
- [ ] Artifact-to-claim/evidence linking UI.

### M4 — SSH execution fabric (separate module)

- [x] Host profiles imported from `~/.ssh/config`, OpenSSH `ControlMaster`, known_hosts.
- [x] Keychain passphrase storage and `dunlin-ssh-askpass` helper.
- [x] Relay bootstrap with SHA-256 verification, launchd/systemd user service.
- [x] Loopback tunnel, auto-pair and RemoteHost transport integration.
- [x] Settings → Remote SSH host manager (import/test/bootstrap/tunnel/remove).
- [ ] Real-host QA for MFA, wrong host key, network drop and cleanup.

## User cases

### UC-01 — Install and enable

A user installs the pack with `scripts/install-governance-plugin.sh`. It appears
in Settings → Extensions as disabled. Enabling it verifies the manifest, loads
the WASM module only on demand, indexes existing `intent/`, `concepts/` and
`decisions/` files, and shows the kinds in the Spec workspace. Acceptance: no
WASM is loaded before Enable; no project file is rewritten.

### UC-02 — Disable or uninstall

The user disables the pack. The kinds disappear from the cockpit and context
envelope; files and SQLite rows remain. Re-enabling restores the index. Removing
the plugin code leaves project files untouched. Acceptance: zero deletions,
zero dangling links.

### UC-03 — Agent proposes a concept

The agent calls `object_propose` through `dunlin-knowledge-mcp`. The host
validates the object with the WASM pack and writes a pending proposal under
`Application Support/Dunlin/proposals`. The project file is not changed. A human
accepts or rejects the proposal in Dunlin. Acceptance: no auto-write, provenance
and validation errors are visible.

### UC-04 — Context envelope

Before dispatch, the host builds a bounded envelope from enabled packs only.
The user sees the preview and the SHA-256 recorded in the run. A disabled pack
contributes nothing and the UI explains why. Acceptance: no silent injection;
the hash matches the preview.

### UC-05 — Schema update

A new pack version changes a kind schema. The host checks `schema_version`,
offers a previewed migration and a rollback. A failed migration leaves the old
version active. Acceptance: no silent rewrite; backup and explicit consent.

### UC-06 — Malformed or hostile module

A module has a wrong hash, traps, exceeds fuel/memory or returns oversized
output. The host rejects the call, records the failure and disables the pack
after three failures. Acceptance: the application stays alive; no partial write.

### UC-07 — Project without the pack

A project without the governance pack opens normally. `intent/` files remain
visible as Markdown in the explorer, but no governance UI or context section is
shown. Acceptance: no errors, no implicit installation.

## Commands

```sh
# Build the WASM pack and its manifest hash.
./scripts/build-governance-plugin.sh

# Install it into Application Support/Dunlin/plugins (disabled by default).
./scripts/install-governance-plugin.sh

# End-to-end host smoke (discover → load → describe → validate → envelope).
cargo run -p dunlin -- --governance-smoke dist/governance

# MCP server smoke (stdio JSON-RPC).
cargo run -p dunlin-knowledge-mcp -- --project . --plugin dist/governance
```

## Acceptance

- `cargo test -p dunlin-knowledge -p dunlin-plugin -p dunlin-plugin-host -p dunlin-storage -p dunlin-artifacts -p dunlin-ssh -p dunlin-relay`
- `cargo test -p dunlin --bin dunlin`
- `cargo clippy -p dunlin-knowledge -p dunlin-plugin -p dunlin-plugin-host -p dunlin-storage -p dunlin-artifacts -p dunlin-knowledge-mcp --all-targets -- -D warnings`
- `cargo run -p dunlin -- --governance-smoke dist/governance`
- Disabling/removing the plugin never deletes project files or knowledge rows.
- Malformed WASM, wrong hash, out-of-fuel code and oversized output fail closed.
- No plugin code is compiled or instantiated at application startup.
