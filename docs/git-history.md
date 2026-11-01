# Local and relay Git history and commit comparisons

Open **Source Control → graph icon**, **Repository → Branch Graph & History…**,
or `⌘⌥G`. History is a **main workbench tab**, not a modal overlay. Chat and file
buffers/undo remain intact; opening history does not save, close or discard files.
Switching to a file or Chat hides the history view without reloading it. Escape
returns to Chat; `⌘W` or the tab's close button closes history only. Close all file
tabs keeps an active history tab. Native menus remain available while browsing.

One history snapshot is cached at a time. It belongs to a specific project,
configured folder and local/relay environment. Remote snapshots also bind the host,
server project ID and paired device. They cannot render for another scope; opening
history for another repository replaces the cached view, not any files/sessions.
Removing, relocating or revoking trust invalidates that snapshot. Refresh and
every new comparison/ref operation revalidate the pinned canonical repository root;
retargeting a folder alias never redirects an operation into another repository.

## Comparisons

The **Repository state** button opens an independent read-only
[repository-state view](git-repository-state.md) in this same tab, including
unborn repositories. It does not change the selected comparison or file buffers.

The commit inspector uses flat file rows with file icons, a filename-first label,
muted directory/rename origin and colored status letters. Full paths and rename
identities remain available on hover and in accessibility labels. Selection uses
a pale accent with a thin left marker, not a filled pill. The file list takes 28%
of the comparison width (170–260 logical pixels); the diff gets the remainder.

Commit metadata shows the subject, short ID (click to copy the full ID), author
and date without repeating the subject in the body. Only merges offer parent
controls; a single parent is a quiet comparison caption. Toolbar actions are
borderless. **Create branch…** reveals the local-only branch form on demand,
leaving more vertical room for the comparison by default.

- Select a commit to load its full message and changed-file list. With focus on
  the graph, Up/Down/Home/End select and reveal commits. Inputs/buttons retain
  their ordinary keyboard handling.
- Root commits compare against the empty tree, without writing a tree object.
- Ordinary commits compare against their parent. Merge commits default to parent
  1 and expose explicit parent buttons; this is **not** a combined merge diff.
- Select a file for its immutable commit patch. Renames include old/new literal
  paths; deletions remain readable even if the worktree path no longer exists.
- Binary, rename-only and mode-only changes show Git's metadata text when there
  are no text hunks. Symlinks/submodules compare their Git entries, not the target
  filesystem contents; no submodule checkout/update is performed.
- Empty commits/comparisons are explicit empty states. Unavailable objects,
  unsupported encodings and oversized results are errors, not fallback diffs
  against HEAD, the index or the worktree.

Selection is debounced and each history/details/patch request has an independent
generation. A superseded or closed view cannot accept an earlier result. Reads,
layout and patch parsing run off the UI thread. Already-running reads may finish
after closing, but their results are discarded; each Git child is timeout-bounded.

## Limits and safety

- 500 initial commits, load up to 2,000 or enter an older commit/branch/tag.
- Local graph output: 2 MiB. Commit metadata/message: 64 KiB. File list: 512 KiB
  and at most 2,000 files. Per-file patch: 2 MiB and 20,000 lines.
- Git subprocesses have a two-second timeout and bounded output. Rename detection
  uses Git's 50% threshold with a 200-target exhaustive-search limit; large change
  sets may therefore show delete/add instead of a rename. Non-UTF-8 paths/text are
  not silently converted into different filenames.
- No automatic fetch (including lazy promisor-object fetch), checkout, stash,
  merge, rebase, index refresh or source writes. External diff/textconv drivers,
  signature checks and repository hooks are not invoked by these reads.
- **Create branch from selected commit** remains a separate, explicit ref-only
  action. The existing project/agent/mutation checks remain in force. Closing
  history does not release its in-flight repository lock or quit guard; completion
  reports the original project even if another project is visible.
- Remote-tracking refs are cached Git refs on the selected Mac/relay, not a fresh
  observation of the repository's origin server.

## Relay transport (read-only protocol v1)

The same tab works for trusted paired relay projects. No local filesystem lookup
or Git process is used for a remote history request. Branch creation controls are
hidden and the handler rejects remote mutation explicitly.

All routes require a bearer device token and a grant for the `project` ID:

| GET endpoint | Parameters | `value` |
| --- | --- | --- |
| `/v1/git/history` | `project`, optional `revision`, `limit` (1–2,000; default 500), optional `scope` | commit array |
| `/v1/git/history/commit` | `project`, `scope`, full `oid`, `parent` (zero-based; default 0) | commit metadata/message, comparison parent and changed files |
| `/v1/git/history/patch` | `project`, `scope`, full `oid`, `parent`, `file` (zero-based index in that comparison) | `oid`, `parent`, exact file identity and patch |
| `/v1/git/overview` | `project`, optional initial `scope` | stash/worktree lists, operation markers and unmerged index paths |

Responses are `{ "version": 1, "scope": "…", "value": … }`. The opaque SHA-256
scope binds the server project/configured path/canonical repository root; it is
**not** a credential or grant. After the initial graph read, desktop sends that
scope on every refresh/comparison. Changed scopes return 409; changing projects,
host address or pairing invalidates the client view. A new tab is needed to adopt
a new repository scope. Callers cannot supply a server root, arbitrary path or
command: unknown parameters are rejected and patch indices resolve on the server.

The relay uses the same bounded Git readers as local history. Authorization and
device activity are checked again before returning results. At most four history
jobs run concurrently (429 when busy); cancelling HTTP does not release the slot
until the blocking Git work really finishes. This is a resource bound, not an
unbounded background queue.

Desktop requires protocol v1, rejects redirects, bounds decompressed JSON to
16 MiB and validates scope, commit/parent/file identity and payload limits. Requests
time out after 30 seconds; individual Git children retain their two-second bound.
Old endpoints/protocols produce an explicit update/error state: **no legacy Git
API or local fallback is attempted**. Tokens/raw error bodies are not echoed in
history transport errors. No agent is started and no prompt or mutation is sent.

## Manual QA checklist (not yet executed)

Original history/inventory baseline: 403 tests passed, 4 optional integration tests
ignored; Clippy `-D warnings` and formatting passed. The local toolchain emitted a
non-fatal `rust-objcopy`/missing `libLLVM.dylib` warning while stripping test debug
info; all test binaries ran successfully. This is not a visual GUI verification.
Current operation/blame validation is documented in
[Git operations and inline blame](git-operations-and-blame.md).

- Disposable local repository with a dirty/partially staged file: open history,
  browse commits, switch to Chat/file and back; verify buffer, undo and index stay
  unchanged. Close history and Close all file tabs separately.
- Compare a root commit, a merge against each parent, an empty commit, a rename,
  deletion, mode change, binary file, Unicode/newline/leading-dash filename.
- Rapidly change commits/parents/files and refresh/close during loading: no stale
  patch under a new commit label. Change projects, revoke trust and relocate roots.
- Use source-control icon, native menu, `⌘⌥G`, arrows/Home/End, Tab/Shift-Tab,
  Escape and `⌘W`; check focus and scrolling in light/dark at minimum window size.
- Check the redesigned inspector at narrow/wide widths: visible basenames, muted
  directories, complete hover labels, subtle selection/focus and no pill-shaped
  file/parent controls. Copy an ID, compare each merge parent and reveal/hide the
  branch form. Visual validation of this redesign has not yet been performed.
- Create a branch from an older commit, close/navigate while it runs, and verify
  the working branch/index/files remain unchanged and quit/mutation guards hold.
- Test empty/non-Git roots, missing objects and oversized patches; never present
  another project's data or silently fetch.
- Pair a real updated relay and compare graph/parents/files/patches with its local
  repository. Verify the relay read-only label and absent branch-create controls.
- Revoke a grant/device, change host/pairing or retarget a server project alias
  while loading: access/scope errors, no local fallback or stale cross-host data.
- Test an older relay (404), offline server, busy limit (429), oversized/malformed
  response and redirect; history must fail closed without exposing the token.
