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

History has two resizable columns: the full-height commit graph on the left,
changed files above the commit inspector on the right. The inspector also has a
resizable horizontal divider. **Tree** groups files into collapsible directories;
**Flat** shows basenames with muted directories/rename origins. Both lists are
virtualized. Status names are colored, and full literal paths/rename identities
remain available in tooltips and accessibility labels. Presentation sorting
never changes the backend file indices used to request an immutable patch.

The graph uses identity-colored first-parent paths, curved branch/merge edges,
ring markers for merges and a selected-commit halo. Hover emphasizes the matching
path. Clicking refs filters to history ending at that exact commit; **All refs**
restores the unfiltered view. Short selection fades respect both the app and OS
reduced-motion settings; scrolling does not animate the whole graph.

**Commit details**, **Diff** and **Operations** switch the lower-right pane without
closing history. Metadata shows the subject, copyable short ID, author, commit
date, refs, parents and message body without duplicating the subject. Only merges
offer comparison-parent controls. Selecting a file opens **Diff**; patches load
on demand rather than eagerly during commit navigation. **Create branch…** in
Operations reveals the ref-only branch form for local or relay repositories.

Right-click a commit for **Create branch here…**, **Merge into current branch…**,
**Rebase current branch onto here…**, **Cherry-pick…**, **Create worktree here…**
or the general operation form. Actions pin a commit OID and view identity, not a
virtual row index. Merge cherry-picks require an explicitly entered mainline.
Mutations retain the existing fresh preflight and native confirmation. The
history toolbar also exposes **Repository state**, including stash/worktree
inventory, and an operation banner shows observed in-progress operations and
conflicts with **Continue… / Abort…** where supported. These are snapshots, not
inferred historical rebase/cherry-pick events.

- Select a commit to load its full message and changed-file list. With focus on
  the graph, Up/Down/Home/End select and reveal commits. Inputs/buttons retain
  their ordinary keyboard handling. Enter opens Operations. In the file tree,
  Up/Down/Home/End navigate, Left/Right collapse/expand, and Enter opens a diff.
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
Refresh and **Load more commits** keep the displayed snapshot until its replacement
is ready, preserve the selected OID when still present, and anchor scrolling to
the first visible commit. A failed refresh explicitly labels retained data as the
previous cached snapshot. A removed selection clears its comparison and invalidates
pending detail/patch reads before selecting the first replacement row.

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
  action for local and relay repositories. **Checkout commit** detaches HEAD at
  one exact local commit only after a fresh clean-worktree check; it never
  stashes, resets or forces. The existing project/agent/mutation checks remain in
  force. Closing history does not release its in-flight repository lock or quit
  guard; completion reports the original project even if another project is visible.
- Remote-tracking refs are cached Git refs on the selected Mac/relay, not a fresh
  observation of the repository's origin server.

## Relay transport (read-only protocol v1)

The same tab works for trusted paired relay projects. No local filesystem lookup
or Git process is used for a remote history request. Remote branch creation uses
an explicit, scoped mutation endpoint; commit checkout remains local-only.

All routes require a bearer device token and a grant for the `project` ID:

| GET endpoint | Parameters | `value` |
| --- | --- | --- |
| `/v1/git/history` | `project`, optional `revision`, `limit` (1–2,000; default 500), optional `scope` | commit array |
| `/v1/git/history/commit` | `project`, `scope`, full `oid`, `parent` (zero-based; default 0) | commit metadata/message, comparison parent and changed files |
| `/v1/git/history/patch` | `project`, `scope`, full `oid`, `parent`, `file` (zero-based index in that comparison) | `oid`, `parent`, exact file identity and patch |
| `/v1/git/overview` | `project`, optional initial `scope` | stash/worktree lists, operation markers and unmerged index paths |
| `POST /v1/git/history/branch` | JSON `project`, `scope`, full `oid`, `name` | creates one ref; no checkout/HEAD/index change |

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

## Automated redesign checks

The desktop suite passed 440 tests, including 20 history tests. Graph-specific
coverage checks distinct fork colors, color continuity across virtual rows,
column compaction, stable older-page extension and fractional scroll anchoring.
File-tree coverage checks literal paths, sorted patch indices and collapsed
directories without hiding similarly named siblings.

The isolated native-window smoke passed in dark (1,200 × 800) and light/narrow
(760 × 800) windows, including the virtual 500-commit list and existing buffer,
stash and mutation-lock protections. This is automated controller/painting
acceptance, not manual visual or native-confirmation-dialog acceptance.

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
- Check both resizable dividers at narrow/wide widths, Tree/Flat switching,
  nested collapse/expand, keyboard focus, literal/renamed paths and status names.
  Copy an ID, compare each merge parent, open a file diff and reveal/hide the branch
  form. Verify hover paths, merge curves and reduced-motion behavior in both themes.
  Refresh while scrolled, prepend commits, and load older history without losing
  the visible anchor, selection or immutable comparison. Right-click before a
  refresh and ensure menu actions never target a newly occupying row. Visual
  validation of this redesign has not yet been performed.
- Create a branch from an older commit, close/navigate while it runs, and verify
  the working branch/index/files remain unchanged and quit/mutation guards hold.
- Test empty/non-Git roots, missing objects and oversized patches; never present
  another project's data or silently fetch.
- Pair a real updated relay and compare graph/parents/files/patches with its local
  repository. Verify the relay label, explicit remote branch/operation requests
  and no local interpretation of server paths.
- Revoke a grant/device, change host/pairing or retarget a server project alias
  while loading: access/scope errors, no local fallback or stale cross-host data.
- Test an older relay (404), offline server, busy limit (429), oversized/malformed
  response and redirect; history must fail closed without exposing the token.
