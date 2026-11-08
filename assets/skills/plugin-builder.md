---
name: plugin-builder
description: Design, implement, test, build and install a Dunlin WASM plugin (manifest v2, sandboxed ABI, knowledge schema, MCP tools, permissions and lifecycle).
---

# Dunlin WASM plugin builder

Use this skill when the user wants a new Dunlin plugin, wants to extend the
governance pack, or asks how plugin manifests, WASM ABI, knowledge kinds or
MCP tools work.

## Interview first

Do not write files before these answers are explicit:

1. What should the plugin do, in one sentence?
2. Plugin id (lowercase letters, digits and hyphens), name, version and author.
3. Which knowledge kinds does it contribute? For each kind: id, label,
   directory, statuses, fields, relations and default status.
4. Which context sections should it add to the intent envelope?
5. Which MCP tools should it expose? Mark read-only vs proposal-only.
6. Which project paths may it read or write? It must not request network or
   process execution unless the host explicitly supports that.
7. How will the user verify it? Give concrete sample files and expected output.
8. Should it be bundled with the app or installed as an external pack?

Summarise the answers before creating the manifest. Ask one question at a time
when the user is undecided.

## Hard rules

- The plugin is a **sandboxed WASM module**. It gets no WASI, no filesystem, no
  network, no Keychain and no GPUI.
- The host owns file IO, SQLite, UI, permissions, context envelopes and MCP
  routing. The plugin only receives JSON and returns JSON.
- A plugin is disabled by default. Enabling is an explicit user action.
- Never write into `Dunlin.app/Contents`. Install under
  `~/Library/Application Support/Dunlin/plugins/<id>/<version>/` on macOS or
  `~/.local/share/dunlin/plugins/<id>/<version>/` on Linux.
- No native dylib, no dynamic library loading, no process execution from the
  plugin.
- Output must be bounded and valid JSON. Never return secrets, absolute host
  paths or executable HTML.
- Migrations must be previewable and must not silently rewrite project files.

## Plugin directory

```text
my-plugin/
  plugin.json
  plugin.wasm
  README.md
  LICENSE
  templates/          # optional Markdown templates
```

The app discovers only directories containing `plugin.json`. Bundled ids win
over external ids with the same name.

## Manifest v2

```json
{
  "id": "my-plugin",
  "name": "My Plugin",
  "version": "1.0.0",
  "api": 2,
  "min_app": "0.2.0",
  "description": "What it does and what it does not do.",
  "author": "Your name",
  "license": "MIT",
  "wasm": {
    "path": "plugin.wasm",
    "sha256": "<64 lowercase hex characters>",
    "exports": ["dunlin_alloc", "dunlin_free", "dunlin_call"],
    "fuel": 10000000,
    "timeout_ms": 100,
    "max_memory_bytes": 67108864,
    "max_output_bytes": 1048576
  },
  "knowledge": {
    "api": 1,
    "kinds": [
      {
        "id": "note",
        "label": "Note",
        "directory": "notes",
        "statuses": ["draft", "accepted", "archived"],
        "default_status": "draft",
        "fields": [
          {"id": "body", "label": "Body", "kind": "markdown", "required": true}
        ],
        "relations": ["references"]
      }
    ],
    "relations": [
      {
        "id": "references",
        "label": "references",
        "from_kinds": ["note"],
        "to_kinds": ["note"]
      }
    ]
  },
  "contributes": {
    "knowledge_kinds": [],
    "context_sections": ["note"]
  },
  "permissions": {
    "project_read": ["notes/**"],
    "project_write": ["notes/**"],
    "network": [],
    "process": false
  }
}
```

Rules:

- `api` must be 1 or 2. WASM requires API 2.
- `wasm.path` is relative, must end in `.wasm`, and must not contain `..`.
- `sha256` is the SHA-256 of the exact WASM bytes.
- `knowledge` is optional; if omitted, the host calls the `describe` op.
- `context_sections` are section ids the host may include in an envelope.
- `permissions.process` must be false for a WASM pack.
- `network` must stay empty for a WASM pack in the current host.

## ABI v1

Core wasm, no imports, no WASI.

```rust
#[unsafe(no_mangle)]
pub extern "C" fn dunlin_alloc(len: u32) -> u32;

#[unsafe(no_mangle)]
pub extern "C" fn dunlin_free(ptr: u32, len: u32);

#[unsafe(no_mangle)]
pub extern "C" fn dunlin_call(ptr: u32, len: u32) -> u64;
// returns (response_ptr << 32) | response_len
```

Request:

```json
{
  "api": 1,
  "op": "validate",
  "kind": "note",
  "schema_version": 1,
  "input": {}
}
```

Response:

```json
{"ok": true, "output": {}, "warnings": []}
```

Errors:

```json
{"ok": false, "output": null, "warnings": [], "error": "why it failed"}
```

Supported ops:

- `describe` — return `{"schema_text": "<JSON schema string>"}` or a schema
  object. Keep it small.
- `parse` — `input` is `{"path": "...", "source": "..."}`; return a parsed
  object.
- `validate` — `input` is the object; return `{"errors": ["..."]}`.
- `migrate` — `input` is `{"from_version": n, "to_version": m, "object": {}}`.
- `context_section` — return `{"text": "Markdown section"}`.
- `template` — return `{"text": "Markdown template"}`.

The host enforces fuel, memory, recursion and output limits. A trap disables
the plugin after three failures.

## Minimal Rust plugin

```toml
[package]
name = "dunlin-my-plugin"
version = "1.0.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[workspace]
```

```rust
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::alloc::{alloc, dealloc, Layout};

#[derive(Deserialize)]
struct Request {
    #[serde(default = "api")]
    api: u32,
    op: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    input: Value,
}
fn api() -> u32 { 1 }

#[derive(Serialize)]
struct Response {
    ok: bool,
    output: Value,
    warnings: Vec<String>,
    error: Option<String>,
}

#[unsafe(no_mangle)]
pub extern "C" fn dunlin_alloc(len: u32) -> u32 {
    let layout = match Layout::from_size_align(len.max(1) as usize, 1) {
        Ok(layout) => layout,
        Err(_) => return 0,
    };
    unsafe { alloc(layout) as u32 }
}

#[unsafe(no_mangle)]
pub extern "C" fn dunlin_free(ptr: u32, len: u32) {
    if ptr == 0 { return; }
    if let Ok(layout) = Layout::from_size_align(len.max(1) as usize, 1) {
        unsafe { dealloc(ptr as *mut u8, layout) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn dunlin_call(ptr: u32, len: u32) -> u64 {
    let input = if ptr == 0 || len == 0 { &[] } else {
        unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }
    };
    let response = match serde_json::from_slice::<Request>(input) {
        Ok(request) => handle(request),
        Err(error) => fail(format!("invalid request: {error}")),
    };
    let bytes = serde_json::to_vec(&response).unwrap_or_default();
    let out = dunlin_alloc(bytes.len() as u32);
    if out == 0 { return 0; }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, bytes.len()) };
    ((out as u64) << 32) | bytes.len() as u64
}

fn handle(request: Request) -> Response {
    match request.op.as_str() {
        "describe" => ok(json!({"schema_text": SCHEMA})),
        "validate" => ok(json!({"errors": validate(&request)})),
        "context_section" => ok(json!({"text": render_context(&request.input)})),
        "template" => ok(json!({"text": TEMPLATE})),
        _ => fail("unsupported operation".into()),
    }
}

const SCHEMA: &str = r#"{"api":1,"kinds":[{"id":"note","label":"Note","directory":"notes","statuses":["draft","accepted","archived"],"default_status":"draft","fields":[{"id":"body","label":"Body","kind":"markdown","required":true}],"relations":[]}],"relations":[]}"#;
const TEMPLATE: &str = "---\ndunlin: 1\nkind: note\nid: NOTE-001\ntitle: \nstatus: draft\n---\n\n## Body\n";

fn validate(_request: &Request) -> Vec<String> { Vec::new() }
fn render_context(_input: &Value) -> String { String::new() }
fn ok(output: Value) -> Response {
    Response { ok: true, output, warnings: Vec::new(), error: None }
}
fn fail(error: String) -> Response {
    Response { ok: false, output: Value::Null, warnings: Vec::new(), error: Some(error) }
}
```

## Build and hash

```sh
rustup target add wasm32-unknown-unknown
cargo build --manifest-path my-plugin/Cargo.toml --target wasm32-unknown-unknown --release
shasum -a 256 my-plugin/target/wasm32-unknown-unknown/release/dunlin_my_plugin.wasm
```

Put the hash in `plugin.json`, then install:

```sh
mkdir -p "$HOME/Library/Application Support/Dunlin/plugins/my-plugin"
cp plugin.json plugin.wasm "$HOME/Library/Application Support/Dunlin/plugins/my-plugin/"
```

The app also supports **Settings → Extensions → Install automatically** for
bundled catalog packs. External packs can be dropped into that folder and
enabled explicitly.

## Knowledge files

- Files are canonical. SQLite is a rebuildable index.
- Frontmatter uses `dunlin: 1`, `kind`, `id`, `title`, `status`.
- Stable ids belong in frontmatter; legacy files get a derived id and are never
  rewritten automatically.
- Links:

```yaml
links:
  - relation: references
    target: NOTE-002
```

- Required fields are declared in `knowledge.kinds[].fields`.
- Relations are validated against `knowledge.relations` from/to kinds.
- `tasks.md` may use `depends_on: T001, T002`; `[P]` is only a parallelism hint.

## MCP tools

The host owns `dunlin-knowledge-mcp`. A plugin can declare which knowledge
kinds and context sections it contributes. Write tools are proposal-only:

- `object_propose` writes a pending proposal; it never writes project files.
- `relation_propose` writes a pending proposal.
- `evidence_link` writes a pending link.
- A human accepts or rejects proposals in Dunlin.

Never make an MCP tool that self-approves a proposal.

## Security checklist

- [ ] No WASI, imports, filesystem, network, Keychain or process execution.
- [ ] Manifest `permissions.process` is false and `network` is empty.
- [ ] WASM hash matches the exact bytes.
- [ ] Output is bounded; no secrets or absolute host paths.
- [ ] Invalid input returns `ok: false`, not a trap.
- [ ] Plugin is disabled by default and installed outside the app bundle.
- [ ] README states what the plugin does, does not do, and how to uninstall it.
- [ ] Tests cover `describe`, `validate`, `context_section` and one failure case.

## Test plan

1. Native unit tests for validation and rendering.
2. Build the WASM and verify its SHA-256.
3. Load it through `dunlin --governance-smoke` style host or a small harness.
4. Validate sample files with missing fields, invalid statuses and unknown
   relations.
5. Disable the pack, confirm files remain and UI hides the kinds.
6. Re-enable and confirm the index rebuilds.
7. Test a wrong hash and a malformed response; the host must fail closed.

## Delivering to the user

Report:

- plugin id and version,
- manifest path and WASM hash,
- install path,
- exact build and test commands,
- what the plugin reads/writes,
- what it deliberately does not do,
- how to disable or uninstall it.

Ask before writing any file into the user's project.
