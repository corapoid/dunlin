# Editor and previews

## Edit safely

Explorer opens native file tabs with dirty tracking, Undo, **⌘S**, optional
autosave and external-change/conflict handling. **⌘⇧S** saves all tabs. Closing
dirty tabs or quitting requires an explicit save/discard/cancel decision. A
checkpoint restore or Git operation is not a replacement for buffer protection.

Two panes share canonical buffers; opening the same file does not create two
independent saved versions. Tabs and undo are launch-local, unlike saved files
and persisted conversation history. See [workspace](workspace.md).

Markdown opens in Visual mode with a Raw switch. Tables, code blocks and prose
share source Undo/save; resource loading and navigation have additional safety
limits. Read the [Markdown editor contract](markdown-editor.md) before relying on
round-trip behavior for complex documents.

## Highlighting and minimap

Language selection uses filenames and bounded syntax grammars, not execution of
project code. The catalog includes common source/web/data languages and Zig,
Dart, C#, PowerShell, HCL/Terraform, Jinja, GraphQL, Protobuf, Nix and CMake.
YAML-based Ansible/CI/container formats use their corresponding grammars.
Highlighting is not language-server completion or semantic validation; arbitrary
extensionless inventories are not content-sniffed.

The minimap shows the viewport, local changes against HEAD and unsaved edits.
Remote files show unsaved edits rather than an invented local Git baseline.
Font, wrap and minimap controls are in **Settings → Editor & Terminal**. Reading
zoom is shared with chat and terminals; it does not edit shell configuration.

## Images, PDF and remote files

Bounded PNG/JPEG/SVG images and paged PDFs use native previews. Unsupported or
binary file types fail explicitly. SVG scripts/external resources are not loaded;
unknown extensions are not admitted merely because another grammar can color them.
Preview caches and source reads are bounded.

Relay files use the relay's granted file APIs. Updating only the Mac may not
enable a newly supported remote extension; update the relay as well. No local
filesystem fallback is used for a remote project.

Sources: [syntax catalog](../../crates/workspace/src/syntax.rs),
[preview implementation](../../crates/workspace/src/preview.rs),
[file preview UI](../../apps/desktop/src/file_preview.rs),
[Explorer](../../apps/desktop/src/explorer.rs).
