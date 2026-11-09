# Knowledge, governance and proposals

The Knowledge workspace presents file-first objects as a list, details and a
relation graph. Files are canonical; SQLite stores derived indexes and evidence.
The host owns file operations, UI, permissions and bounded context. The optional
WASM pack supplies governance schema and pure validation/transformation logic.

## Install the governance pack

In **Settings → Extensions**, find `dunlin-governance`, choose **Install
automatically**, validate its WASM and explicitly enable it. Installation does
not imply enablement. Startup discovery reads manifests; it does not eagerly
compile/instantiate every WASM module.

The pack contributes Intent, Concept and Decision kinds for Markdown under
`intent/`, `concepts/` and `decisions/`. Frontmatter records kind, stable id, title,
status and relation metadata. Legacy intent files can have derived IDs; indexing
does not silently rewrite them. The bundled example schema is in
[plugin.json](../../assets/plugins/governance/plugin.json).

Disable a pack to stop its contributions without deleting project files or
knowledge rows. Project files remain readable as Markdown. Do not infer publisher
signatures, automatic upgrades or migration rollback from the old design plan;
see [current plugin boundaries](../development/plugins.md).

## Agent access and human review

For enabled governance on trusted local projects, the desktop can inject the
host-owned knowledge MCP server into ACP. It exposes bounded context/list/get
tools and proposal tools. Object/relation/evidence writes enqueue pending
proposals; they do not directly mutate project files. A human accepts/rejects
them in the inbox, with host-side path/scope validation.

Review the exact proposed content and destination before acceptance. The plugin
is not authority to widen permissions or approve its own proposals. Context
envelopes are bounded and derived from enabled kinds; they do not establish that
the harness saw or obeyed every instruction.

## Related workspaces

- [Spec workspace](spec-workspace.md) indexes Spec Kit/governing documents and
  exposes advisory phases, hash-bound decisions and task evidence.
- [Artifacts](artifacts.md) capture content-addressed evidence, independently of
  the WASM pack.
- **Build your own plugin** opens a guided agent chat using the bundled
  `plugin-builder` skill; it does not automatically enable generated code.

Sources: [knowledge UI/proposals](../../apps/desktop/src/knowledge.rs),
[core](../../crates/knowledge/src), [MCP](../../crates/knowledge-mcp/src/main.rs).
