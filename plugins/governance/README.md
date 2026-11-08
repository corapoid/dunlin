# Dunlin governance WASM pack

Disabled-by-default Intent / Concept / Decision schema and pure validation logic.

The host owns files, SQLite, UI, permissions and context envelopes. This module
has no WASI, no imports, no filesystem and no network access.

## Build

From the repository root:

```sh
./scripts/build-governance-plugin.sh
```

The script builds `wasm32-unknown-unknown`, computes the SHA-256 and writes
`dist/governance/plugin.json` plus `plugin.wasm`.

## Install

```sh
./scripts/install-governance-plugin.sh
```

Then enable `dunlin-governance` in **Settings → Extensions** and use
**Validate WASM**.

## ABI

Exports:

```text
dunlin_alloc(len: u32) -> u32
dunlin_free(ptr: u32, len: u32)
dunlin_call(ptr: u32, len: u32) -> u64  // (response_ptr << 32) | response_len
```

Requests and responses are JSON. Operations: `describe`, `parse`, `validate`,
`migrate`, `context_section`, `template`.
