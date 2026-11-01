# Visual Markdown editor

`.md` and `.markdown` files now open **rendered by default**, in the existing main
file tab. The two modes are **Visual** and **Raw**. This replaces the earlier
Source/Preview/Split UI: Visual is editable, not a read-only preview next to code.
One button in the file header switches between Visual and Raw; there are no
per-block Raw buttons. The compact formatting row groups undo/redo and inline
formatting icons, with headings, lists, quotes, code blocks, tables and images in **Insert**.
Tooltips and accessibility labels describe icon-only controls. In narrow panes,
the formatting row scrolls horizontally instead of wrapping or squeezing icons.
Insert uses standard, non-modal menu rows: opening it does not dim/blur the
document or register an oversized rectangular backdrop cutout around the menu.

## Visual editing

The document uses **persistent native text editors**, not rendered labels that
are replaced with inputs on click. A first click positions the native caret.
Focus never changes font, line height, weight, padding or the editor entity.
Typography is compact: body/table text **13 px**, code/syntax **12 px**, headings
**20 / 17 / 15 / 14 / 13 px**, with 1.4 line height and 8 px block spacing.
Fields grow only when text or wrapping width changes. Empty minimum editor rows
are clipped so they cannot paint or intercept clicks over the next block.

- Paragraphs, headings, lists/quotes (including multiline continuations), task
  text and inline styles are editable without their Markdown delimiters.
- Tables are native grids of editable cells, including header weights and column
  alignment. Cell edits preserve the rest of the table and its delimiter row;
  literal pipes are escaped and cell line breaks become `<br>`.
  Selecting a cell exposes **Table** on the top toolbar: insert/delete body rows,
  insert/delete columns, and left/center/right/default column alignment. The
  header row and last remaining column cannot be deleted. There are no per-row
  buttons. **Tab / Shift+Tab** select adjacent cells; Tab on the final cell appends
  a blank body row and selects its first cell. Shift+Tab at the first cell leaves
   the table. Outside tables, Tab/Shift+Tab indent/outdent a list item or quote
   subtree; ordinary paragraphs and code retain their native indentation behavior.
- Fenced code is edited without its fences. Language/info strings are preserved;
  typing fence characters lengthens the enclosing fence to keep code contained.
- Character entities, hard breaks, reference-link labels and image descriptions
  have editable source maps. Image URLs/titles and link destinations survive text
  edits unchanged; images are never fetched automatically.
- HTML, metadata/frontmatter, definitions, separators and unusual syntax have
  editable literal syntax fields **inside Visual**, never inert read-only blocks.
  HTML is not executed and these fields do not claim HTML WYSIWYG editing.
- Task checkboxes toggle directly. **+ Paragraph** appends a paragraph. **Enter**
  splits prose at the selection into source-backed blocks, continues nested lists
  (incrementing numbers and clearing the new task checkbox), or exits an empty
  list/heading/quote. Splitting a heading starts ordinary body text on the right.
- **Backspace** at the beginning / **Delete** at the end joins compatible adjacent
  prose blocks and puts the caret at the join. The touched inline fragments retain
  balanced marks/link syntax and Unicode; leading/trailing literal spaces can be
  entity-escaped to keep Markdown from dropping/reinterpreting them.
- **Shift+Enter** keeps a newline within the field. Code and table fields retain
  their native newline behavior; code edits preserve trailing blank code lines.

### Document selection, navigation and nesting

- Left/Right move by grapheme, including combining marks and joined emoji, and
  cross field boundaries. Up/Down use native caret geometry and a retained X
  position. Table Up/Down keeps the column; entering/leaving a table crosses rows,
  not horizontal cells. The caret is revealed in the document scroll area.
- Shift+arrows, Shift-click and pointer drag extend selections across fields;
  Cmd+A selects the document. Copy exports visible prose, tabs between table
  cells and newlines between rows. Cut, paste, Delete/Backspace and typing replace
  the selected content in one canonical-source transaction. Read-only still
  permits navigation, selection and copying, never mutation.
- Compatible prose endpoints merge and preserve balanced inline marks. Partial
  mixed code/table/syntax selections clear their selected **contents**, preserving
  structural scaffolding; selecting the whole document removes scaffolding too.
  Cross-block formatting is guarded rather than applying only to the head field.
- Nested list/task/ordered/quote prefixes are token-aware: only the active ordered
  marker increments, ancestor markers become continuation indentation, and only
  the new task checkbox resets. Empty nested items lift one level. Tab/Shift+Tab
  moves one AST item/quote and its descendants, validating all visible text before
  mutation. Indenting a list item requires a preceding sibling. Bulk subtree
  indentation, lazy quote lifting and exotic/tab indentation remain guarded.

This is not an arbitrary rich-text serializer. Edits that invalidate the surrounding
projection fail closed with a notice; they never silently discard source.
Exact caret restoration after canonical undo, IME composition over multi-block
selections, bidi and pathological nesting still need further QA.
Structural merging never crosses a table/code/HTML/separator or mismatched
container boundary. Splitting around zero-width image descriptions and some
complex nested inline constructs remains guarded instead of losing syntax.

Table structure edits preserve untouched cell spellings (including escapes and
inline code), indentation/quote/list prefixes, surrounding document bytes and EOF
newline policy. Rows with fewer cells are padded when a column operation requires
it; rows with extra hidden GFM cells fail closed instead of exposing/discarding
their data. Each operation validates the entire resulting projection before one
canonical-buffer replacement. Open menu actions are pinned to the tab, table,
selected cell and exact source snapshot, and recheck project identity and locks.

## Lossless source and history

The workbench's existing source editor remains the authoritative buffer, including
dirty tracking, autosave and conflict-aware save. Visual controls are projections.
Each visual edit replaces only the affected source-backed block; the rest of the
file, including unrelated blocks and blank separators, remains byte-identical.
Untouched inline nodes retain their original delimiters and link destinations.
New literal punctuation is escaped instead of being unexpectedly interpreted as
Markdown. No whole-document AST serialization is used when saving or switching.

Visual **Undo/Redo** and **⌘Z / ⇧⌘Z** route to the canonical source editor's history,
not to independent block histories. Its action node remains clipped/invisible
while the rendered document is shown. Raw uses the same editor entity directly.
Switching to Raw flushes an outstanding visual edit before removing the projection.
Visual edits recheck read-only/loading state, Explorer/AI-commit locks, project
identity and the exact source snapshot before modifying anything.

Mode/scroll are kept per open tab, not persisted across app restarts. Search
results with a requested line reveal Raw; ordinary opens default to Visual.
Local and relay files share the UI and existing scoped file read/write backend.
No agent is started and no mutation is performed by merely opening the file.

## Link/image properties and safety

Place the caret in a link or image description and choose **Link / image…** on the
top toolbar. The contextual panel edits address and optional title; labels/alt text
remain editable in the document. Inserting a link/image opens its properties.
Reference uses update their shared definition (including definitions preceding the
use), not an unrelated inline label. Apply is a canonical undoable source edit.

**Load project image** explicitly reads and decodes only relative project PNG/JPEG/SVG
resources through the tab's local or authenticated relay backend. It never treats a
server path as a local path or falls back to local reads. Document-relative paths
are decoded and normalized within the project, then backend canonical confinement
rejects escaping symlinks. Reads are capped at 32 MiB; raster dimensions/allocation
are bounded by the existing preview decoder. SVG resource resolvers are disabled.
Changed source/address/project, closed/reopened panels and superseded request IDs
cannot attach an old preview. Closing/Raw switching discards panel state; in-flight
bounded I/O may finish, but cannot restore the panel.

**Open link…** opens relative files inside the workbench. HTTP(S) links require a
native confirmation before opening a browser. Credentials, unsafe schemes, rooted
paths, traversal outside the project, encoded separators and double encoding are
rejected. External images, `file:`, `data:` and executable document content never load.

### Limits

- Native GPUI rendering, no browser/webview and no document JavaScript.
- Merely rendering/focusing Markdown performs no resource reads/network requests.
  Opening the resource panel also performs no reads; activation is explicit.
- Visual parsing is bounded to 128 KiB, 2,000 lines, 1,000 blocks and a traversal
  budget/depth limit. Oversized/unsupported input remains available in Raw.
  Source formatting remains bounded to 2 MiB.
- Raw updates are debounced; closed/superseded projections cannot overwrite a
  newer source or restore a closed tab. Scroll synchronization is unnecessary
  for the two exclusive modes; each keeps its own editor/document scroll state.

## Validation

Workspace tests: **418 passed, 4 optional integration tests ignored**. Desktop
build and the native Markdown smoke passed for the Visual/Raw implementation.
`cargo fmt --all --check`, workspace Clippy with `-D warnings`, and
`git diff --check` also passed. The smoke was rerun after making the invisible
canonical editor presentational for accessibility; history dispatch still passed.
The toolbar refresh also passed these checks and the native smoke, including the
single toggle's Visual → Raw → Visual path. A toolbar catalog test verifies that
all 13 formatting actions remain reachable and their icons are bundled.
After the Insert backdrop fix, these checks and the native smoke passed again;
a regression test requires standard menu rows rather than custom modal probes.
The smoke exercises editor behavior, not opening or visually inspecting Insert.

Unit tests cover source maps, styled text without delimiters, Unicode/escapes,
heading/list edits, nested/multiline continuation maps, table cell edits, entities,
images/reference links, safe code fences, paragraph joining, task offsets and
preservation of unrelated source, alongside the toolbar and bounds tests.
Table tests additionally cover row/column operations, sparse rows, omitted outer
pipes, alignment, quoted/list tables, CRLF, EOF policy and protected/invalid inputs.
Structure tests cover styled/Unicode splits, headings, selected-text replacement,
list numbering/task identity, quote/CRLF preservation, compatible joins and
protected boundaries; a typography regression test bounds the heading/body ratio.

The debug-only native-window smoke uses an isolated database and fixture, no
harness and no user files:

```sh
cargo build -p dunlin
target/debug/dunlin --markdown-smoke "${TMPDIR%/}/dunlin-markdown-smoke-$(uuidgen)"
target/debug/dunlin --markdown-smoke "${TMPDIR%/}/dunlin-markdown-smoke-$(uuidgen)" --light-narrow
```

It dispatches a real pointer click into a mounted visual editor, verifies first-click
caret placement and unchanged editor identities/bounds/line heights on focus, then
tests prose formatting, table/code/image edits, canonical undo/redo, Visual/Raw
round-trip, dirty/save, stale cancellation and blocked Markdown/HTML image requests
against a loopback listener. It explicitly draws its own window before dispatching
events; it is **not manual visual GUI QA**.
The table-structure extension also dispatches native Tab/Shift+Tab, tests append at
the final cell, column insertion/deletion, alignment, stale-menu/read-only guards,
and undo/redo/save of structural edits.
The prose extension dispatches native Enter/Backspace/Delete/Shift+Enter, checks
caret placement and canonical undo/redo, compact font metrics, code trailing
newlines, read-only split rejection and final save.

The document-interaction extension passed at dark 1200×800 and light 760×800:
cross-field arrows, Unicode Shift-selection starting inside a field, pointer
drag/Shift-click, Copy/Cut/Paste/typing, read-only selection, nested Tab/Shift+Tab,
link URL/title Apply + undo, explicit project PNG preview and cancelled preview
completion after reopening its panel. The harness forces a fresh dispatch-tree
render before input; background-window cached dispatch was intermittent in earlier
runs. Clipboard contents are restored after successful completion.
Unit tests additionally cover whole/mixed document selections, nested empty-item
lifting, mixed task/quote/ordered prefixes, reference definitions before/after uses,
Unicode/entity escaping, traversal/percent encoding/schemes/credentials, escaping
local symlinks, oversized reads, malformed decode and authenticated mock-relay
preview transport. Existing relay route tests cover preview grants and confinement.

Manual visual QA remains pending: both themes, narrow windows, keyboard/IME/focus,
read-only and lock transitions, long/nested documents, local/relay parity and
complex syntax. The user's running application is not replaced/restarted without
permission. GUI automation was unavailable (`orca skills get computer-use --json`
returned `zsh:1: command not found: orca`); the native smoke is not a visual review.
