# Git mutations and inline blame

## UI

Open **Repository → Branch Graph & History… → Repository state**. The Git view
stays a main-panel tab. Repository state separates in-progress operations,
conflicts, stashes and worktrees into rounded sections. Inventory and preparation
forms share one vertical scroll; each worktree's path and actions stay together.
**Merge / rebase / pick…** in history copies the selected
commit into the operation form; it does not run Git. Alternatively enter a local
commit, branch or tag in the form. Every mutation first prepares a read-only plan,
shows its pinned request/HEAD/target/worktree and bounded status preview (first 80
records), then requires **Review and confirm… → Run this operation** in a native
warning. Changing form fields does not change an already prepared request.

- **Stash tracked… / Stash + untracked…** preserve native Git index/worktree
  semantics. Ignored files are not stashed. Apply/Pop restore the index too.
  Apply/Pop require a clean index/worktree and no untracked files; ignored paths
  must not collide with touched paths. Pop drops only after successful apply and
  a fresh stash-identity check. Conflicts retain the stash. Drop is irreversible
  in the app and pins the exact stash OID/selector; duplicate ambiguous OIDs fail.
- **Add worktree…** creates a detached checkout of the pinned commit under
  `<canonical common Git dir>/dunlin-worktrees/<simple-name>`. It cannot select an
  arbitrary server destination, overwrite an existing folder or force a branch
  already used elsewhere. A local worktree can then be explicitly opened as a
  project and switched to an available branch through the existing branch picker.
  Remote paths have **Copy server path**, never a local-open fallback. Registering
  a newly created remote worktree as another relay project remains a separate
  administrative action, not an implicit expansion of the existing grant.
- **Remove…** only accepts a currently registered, canonical linked worktree of
  the same repository. Current/bare/missing/locked worktrees, submodules, ongoing
  operations, dirty/untracked/ignored contents or native Git locks are refused.
  The UI also requires closing **all** file tabs to protect aliases of that tree.
  Relay removal additionally requires the target to lie within one of that user's
  granted project roots. Registration metadata alone cannot authorize deleting an
  external worktree; grant/register that worktree separately first.
  Native removal has no force flag. **Prune stale…** removes stale registration
  metadata, never recursively deletes a worktree folder; locked entries survive.
- **Merge / Rebase / Cherry-pick…** resolve the selected local object during
  preflight and re-resolve it before execution. Start requires a clean index and
  worktree, including untracked files. Ignored build outputs may remain if they
  cannot overlap incoming/replayed paths. A merge commit cherry-pick requires an
  explicit 1-based mainline parent. Rebase does not auto-stash or update other refs.
- **Continue… / Abort…** appear for merge, rebase and cherry-pick. Resolve conflicts
  in the existing editor, save and stage the resolved paths before Continue.
  Abort explicitly authorizes Git's restoration/discard of **all tracked staged/
  unstaged edits since the operation started**, including unrelated edits and conflict
  resolutions; back them up explicitly first. Untracked/ignored content overlapping
  the restored tree blocks continuation/abort instead of being overwritten.
  Abort is not application Undo. There is no automatic skip, reset,
  force, retry or rollback. Empty picks/advanced interactive sequencing may need
  the Git CLI. Stash conflicts have no native continue/abort marker: resolve them
  normally, preserving the stash until explicitly dropped.

No fetch, push or lazy promisor-object fetch is added by these operations. Mutation
confirmation explicitly warns that native Git may run configured hooks, filters,
merge drivers, signing tools and rebase todo commands. These programs are **not
sandboxed** and can have their own network/side effects. Their failure does not
mean no files changed. Generated Git messages are accepted without launching an
interactive editor. Read-only preflight disables hooks, FSMonitor and configured
content filters; repositories requiring transformed/filter-only clean semantics
can fail closed rather than execute a filter before consent.

## Guards, state and limits

- Plans pin canonical root/common-directory filesystem identity, HEAD/refs,
  configuration, semantic index entries/modes/stages/flags, dirty/untracked file
  contents and modes, operation
  metadata and the stash/worktree inventory. The affected other worktree is
  checked again before removal. Same-path directory replacement invalidates a plan.
  Benign index stat-cache refreshes do not invalidate unchanged semantic consent.
- Execution takes a common-Git-directory `dunlin-operation.lock`, redoes preflight
  and requires exact equality. A crashed/stale lock requires manual inspection;
  the app never removes another owner's lock automatically. Native Git's locks
  remain authoritative. Non-cooperating external writers are not made atomic by
  this application lock: stop external Git/editors before confirming, particularly
  when deleting stash refs. There is no claim of cross-process transactional Git.
- The UI conservatively blocks while any connected agent, Git/file mutation,
  dirty/loading/conflicted buffer is present. During prepare/execute, the mutation
  lock covers all projects to protect aliases and linked worktrees. Closing the
  Git tab does not release a running lock. Open canonical buffers are **never
  replaced by Git**: changed/deleted text files receive a reload/overwrite conflict notice,
  including alternate local project aliases. A persistent **Review…** banner remains
  after dismissing the dialog. Watcher reloads, stale asynchronous file responses,
  autosave and Save All cannot replace a protected buffer; only explicit Reload or
  confirmed Overwrite resolves it. Failed Reload preserves the dirty state and
  protection, rather than marking unsaved text clean. Verification stops starting
  additional file reads after ten seconds (an active read keeps its backend timeout);
  unverifiable buffers remain protected instead of being treated as unchanged.
  No automatic save/discard is added.
- Read commands have a two-second timeout and bounded output. The safety snapshot
  has a 64 MiB dirty-file/operation-metadata budget, 20,000 dirty/untracked-path cap,
  an 8 MiB per-command metadata cap (including semantic index listings) and
  bounded metadata depth. Ignored-collision history checks cap each range at 2,000
  commits. Assume-unchanged/skip-worktree paths and unsupported dirty submodules
  fail closed. Native mutations have a 60-second process-group limit, drained
  stdout/stderr and no retained raw hook output. Timeout means **inspect state**,
  not “nothing happened.” Native Git hooks/signing are not silently bypassed.

## Relay contract

- `POST /v1/git/operation/prepare` accepts `{project, action}` only. Typed actions
  reject unknown fields/commands/force/root overrides. The server derives the root
  from the authenticated project grant and stores the plan itself.
- The response `{id, plan}` contains a one-use token bound to user, device and
  project, expiring after five minutes. At most 64 pending plans are retained;
  a newer prepare replaces that device's previous plan for the same project.
- `POST /v1/git/operation/execute` accepts `{project, id}` only, not a client-authored
  snapshot. Consumed/expired/stale tokens are never replayed automatically. The
  four-job limiter remains held by blocking work even if HTTP disconnects. Device,
  grant and configured project path are checked again before and after execution.
  Revoking access cannot undo an already started native Git command.
- The desktop uses non-redirecting authenticated HTTP with bounded 16 MiB responses
  and a 90-second timeout. Old/denied/offline relays fail explicitly, with no local
  or legacy fallback and no echo of tokens/raw server errors. A lost response may
  mean execution happened: refresh state and prepare a **new** action only after
  review. New endpoints require deploying an updated relay separately.

## Inline blame

Use **Inline blame** in the Explorer toolbar or **Settings → Editor & Terminal** (Markdown:
**Raw** mode). Enablement is an app preference that survives file changes and
restarts; it starts no agent. Cached attribution remains scoped to each file,
backend and HEAD. An unchanged line shows subdued author and last modification
date (`YYYY-MM-DD HH:MM`, local time) after the line when space permits. Hover gives full OID, author/date, subject,
original filename/line and snapshot HEAD; click opens that commit in the existing
history tab. A compact footer shows the same author/date and hover/click target for long lines
and narrow windows. New/modified lines display no author attribution.

The backend reads immutable HEAD blobs and bounded porcelain blame; it never
blames unsaved text or follows a file symlink into the working tree. Background
diff mapping attributes **only exact equal runs**, retaining correctness across
Unicode, CRLF, insertion/deletion and Undo. Mapping is suppressed immediately on
text changes until the new result arrives. The tab/root/backend/path/HEAD cache
rechecks HEAD every three seconds while visible, invalidates on Git refresh and
file retargeting, and rejects superseded/closed/revoked results. Hover identifies
the snapshot, not an atomic promise about external Git activity.

`GET /v1/git/blame?project=…&path=…&scope=…&head=…` uses the existing versioned history
envelope, project grant, scope pin and limiter. Remote filenames never become local
reads. Limits: 2 MiB UTF-8 source, 20,000 lines, 16 MiB porcelain/HTTP output; binary,
untracked, missing and unsupported cases show an error/unknown, never invented
authors. Bare object-only blame suppresses worktree/external mailmaps and replace
refs. Configured ignore-revs files are rejected without opening them because Git
versions may eagerly read them before a CLI reset and they are not part of the
immutable HEAD/file cache key. Raw commit authors are used.

## Validation and manual acceptance

Final automated validation: **450 tests passed, 4 optional integration tests
ignored**; workspace build, Clippy `-D warnings`, formatting and `git diff --check`
passed. The isolated native Git smoke passed in dark and light/narrow windows;
the Markdown editing regression smoke also passed. Toolchain warnings about
`rust-objcopy`/missing `libLLVM.dylib` and `block v0.1.6` future compatibility are
non-fatal and remain separate from feature validation.

Automated coverage includes partial staging/untracked stash round-trips, safe pop
conflicts, stale content/refs/plans, ignored collisions, external index/common locks,
directory/symlink replacement, clean/locked/ignored worktrees and actual pruning,
merge/rebase/cherry-pick conflict continue/abort, protected untracked restoration,
merge mainline selection, hook
failure/timeout and no filter execution before confirmation. Relay route/transport
tests cover grants/devices, expiry/replay/cross-device plans, scoped blame parity,
separate grants for external worktree deletion,
no redirects/retries/local fallback and secret-safe errors. Blame mapping tests
cover Unicode, CRLF, changed lines and Undo.

The debug native-window smoke uses a fresh database/repository, no harness or user
files. It exercises the rendered inventory/form, author mapping/edited-line
suppression, dirty-buffer rejection, cancelled preflight, stash push/pop, locks
surviving view close and canonical-buffer preservation despite watcher reloads and
conflict dismissal, a failed Reload retaining unsaved text/protection, and then
a successful explicit Reload:

```sh
cargo build -p dunlin
target/debug/dunlin --git-smoke "${TMPDIR%/}/dunlin-git-smoke-$(uuidgen)"
target/debug/dunlin --git-smoke "${TMPDIR%/}/dunlin-git-smoke-$(uuidgen)" --light-narrow
```

The paired **loopback HTTP** smoke starts a separate disposable relay/database
and repository, isolates Git configuration and exercises actual authentication,
stash/index round-trip, merge/rebase/cherry-pick, worktree add/remove/prune,
conflict continue/abort, HEAD blame, denied grants, replay and stale plans:

```sh
cargo build -p dunlin-relay
python3 scripts/git-operations-smoke.py --tmp-root "${TMPDIR%/}"
```

It passed without ACP/provider calls or changes to user files. This is not a
deployment or validation against an existing remote host.

Native confirmation acceptance is simulated **only in this isolated harness**.
Manual dialog/tooltip/a11y review and a deployed remote-host acceptance run remain pending,
including both themes/narrow windows, trust/device revocation during execution,
external-process races and signing-tool UI. The running installed app/relay is not
replaced or restarted by this implementation.
