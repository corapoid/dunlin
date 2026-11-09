# Repository state inventory

The inventory reads remain non-mutating. This view now also hosts separately
confirmed [Git operations and inline blame](../user-guide/git-operations-and-blame.md); merely
opening or refreshing it never authorizes a mutation.

Open **Repository → Branch Graph & History…** (`⌘⌥G`), then
**Repository state**. **Commit history** returns to the existing
graph without discarding its selection, comparison or file buffers. The inventory
loads separately, so an unborn repository or invalid history revision does not
prevent reading state. **Refresh state** explicitly replaces the old observation;
an unsuccessful read shows an error, not an empty/clean repository.

Available for trusted local and paired relay projects:

- Stash selector, full object ID in the response (short ID in the UI), date and
  subject, newest first. This does not read stash patches or apply anything.
- Registered worktree paths, HEAD, branch, current-project marker, detached/bare
  flags, lock and prune reasons. Missing/prunable registrations stay visible.
  The reader does not visit sibling worktree directories. Local **Open as project…**
  is an explicit separate action; remote paths never open locally.
- Merge, rebase (merge/apply backends), `git am`, cherry-pick, revert and sequencer
  markers from the selected worktree's Git directory, including linked worktrees.
  Resolved/staged conflicts do not erase an ongoing operation marker. Sequencer
  indicates a sequence, not necessarily a currently conflicted cherry-pick.
- Unmerged paths and present index stages: 1/base, 2/ours, 3/theirs. Conflict
  labels distinguish modify/modify, add/add, modify/delete and other stage sets.
  A stash-apply conflict may exist with **no** operation marker; both lists are
  independent. “No unmerged index entries” never claims a clean working tree.

## Bounds and transport

`GET /v1/git/overview?project=…&scope=…` returns protocol-v1
`{ "version": 1, "scope": "…", "value": { "stashes": […], "worktrees": […],
"operations": […], "conflicts": […] } }`. Scope is optional on an initial read;
subsequent desktop reads pin the scope shared with commit history. Graph and
inventory responses cannot replace each other's scope when they finish out of
order. Closing/reopening the tab is necessary to adopt a changed repository.

The endpoint reuses history's project grant, device recheck, repository-root
validation and four-job semaphore (including HTTP cancellation). Unknown query
parameters, including arbitrary paths/commands, are rejected. The desktop shares
the non-redirecting, size-limited transport and 30-second request timeout. Older
relays require an update; there is no local or legacy fallback.

Each list permits at most 2,000 entries, each Git listing at most 512 KiB and the
inventory at most 2 MiB of serialized JSON. Individual Git children time out after
two seconds. Oversized/invalid/non-UTF-8 metadata fails explicitly rather than
silently truncating or inventing an empty state. NUL-delimited paths preserve
spaces, tabs and newlines; control characters are escaped in the display.

Worktree paths/lock reasons are **repository registration metadata**, including
registrations outside the project's folder. A relay grant exposes that metadata,
not permission to read those directories; the desktop never interprets server
paths as local paths. Operation marker contents are not opened; unexpected marker
types (including symlinks) cause an error. FSMonitor and signature verification
are disabled for relevant reads, and no fetch, hooks, filters or Git writes run.

This is a non-atomic observation, **not mutation preflight**: external Git tools
may change state between commands. Stash/worktree/merge/rebase/cherry-pick actions
now prepare a new content-backed plan and require themed in-app confirmation. They do not
reuse this inventory as authority; [details and limits](../user-guide/git-operations-and-blame.md).

## Validation and pending manual QA

Original inventory baseline: 403 tests passed, 4 optional integration tests ignored;
Clippy `-D warnings`, formatting and the expanded relay smoke test passed.

Automated fixtures cover unborn repos, dirty/partially staged files, stash order
and reflog/index preservation, linked/detached/locked/prunable worktrees,
per-worktree operation markers, actual merge/rebase/cherry-pick and stash-apply
conflicts, literal newline paths, bounds, symlinks and non-execution of FSMonitor.
Relay/client tests cover grants, unknown queries, scopes, parity, malformed
payloads, old endpoints and no local fallback. The relay smoke test reads a
disposable stash/worktree inventory over authenticated HTTP.

Manual GUI QA is **not yet performed**:

- Open both views with dirty editor buffers; return to files/Chat, close the tab
  and verify unchanged text/undo/index. Test keyboard focus and scrolling in both
  themes at minimum window size.
- Compare stash/worktree/conflict lists with Git, including a linked worktree,
  deleted registration, resolved but unfinished operation and stash-apply conflict.
- Read an unborn repository and an invalid history revision; inventory still works.
- Refresh/close/switch projects while loading; revoke trust, relocate roots or
  change relay host/pairing. No cross-project/stale-scope result may appear.
- Test an updated and old relay, offline server and oversized inventory: explicit
  snapshot/error labels, no automatic mutation or fallback, no token in errors.
