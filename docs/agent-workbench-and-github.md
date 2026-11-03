# Agent workbench and GitHub — accepted implementation scope

The user approved **all eight features**, plus GitHub CI and a full PR lifecycle.
This is a staged implementation, not a claim that every stage is shipped.
Implementation tests never replace the installed application/relay or perform
real GitHub mutations. Source commits/version tags are published separately
after explicit user approval; that does not install or restart local services.

## Current delivery (implementation present; manual acceptance pending)

- **GitHub CI/PR core:** Repository history → **GitHub · CI / PR**. Pick an explicit
  Git remote (default `origin`) and Load. The installed `gh` uses its existing
  GitHub.com authentication; no login/token extraction is performed. A missing,
  denied or old CLI fails explicitly. Remote projects have no local fallback.
- Exact-commit Actions runs, attempt-pinned job/step hierarchy, status symbols and
  colors, opt-in 15-second refresh while visible, browser links, completed-job logs
  and selected evidence → existing composer draft. Logs are NOT advertised as a
  live stream. Failures pause polling; empty results never mean CI passed.
- PRs: recent all-state list, description/head/base, files and bounded patches,
  checks/statuses, discussion/review/line comments; create draft/ready PR on already
  published **same-repository** branches, edit title/body, comment, approve/request
  changes/review, RIGHT-side line comments, draft/ready transitions, close/reopen,
  merge/squash/rebase; cancel/rerun CI. Every write has a typed immutable preview,
  account/repository/HEAD/target rechecks, a five-minute approval and native
  confirmation. Merge/reviews pin the reviewed commit; no auto-push, checkout,
  admin override, branch deletion, automatic merge or automatic mutation retry.
  A lost response may mean success: inspect GitHub before submitting again.
- **Context inspector:** composer → **Context…**. Shares the actual client wire
  envelope builder with dispatch, shows effective runtime/default provenance,
  environment names (not values), observed MCP state and skill installation
  records. Harness-owned system/history/instructions/loaded skills are explicitly
  unknown, not falsely presented as absent or fully inspectable. Queued requests
  may have a different envelope when they eventually dispatch.
- **Decision memory:** in Context, explicitly approve local project-scoped notes,
  save new revisions, mark stale and append a selected current decision to the
  draft. Old revisions remain stored; stale decisions cannot be inserted. Plain
  source links are recorded, never opened automatically. Up to 100 revisions,
  256-byte title / 8-KiB body / 4-KiB sources. Stored locally, unencrypted; not a
  secret vault and never silently injected into future prompts.
- **Review/checkpoints v1:** composer → **Review…**. Explicitly capture a named
  saved-file baseline before a task, inspect observed changes and append a file/
  line comment plus bounded diff to the draft. Preexisting dirty/untracked saved
  contents are the baseline, not falsely attributed to the agent. Provenance and
  test coverage remain unknown unless independently established.
  Selective one-file restore has a fresh byte/mode preview, native confirmation,
  content/identity/age guards and a shared Git-dir lock. Close all file tabs and
  stop agents before restoration; no automatic discard of buffers/undo occurs.
- **Checks/problems v1:** composer → **Checks…**. Explicit JSON argv preview and
  confirmation before running native project code (NOT sandboxed). Captures real
  exit code, bounded stdout/stderr, timeouts, cancellation and output failures;
  navigable primary diagnostics from Cargo's structured JSON, report → draft.
  No command is extracted/executed from log text; exit zero does not establish
  coverage, all tests passing, or correctness after later source edits. Stop check
  remains available in Chat after closing its view. Up to five reports are stored
  locally, unencrypted; inspect logs for secrets before sharing.
  Stored reports and decision history have separate confirmed purge actions
  (logical removal from app history, not forensic secure erasure of SQLite pages).

- **Task budgets v1:** composer → **Budget…**. Explicit request-attempt/time limits
  and an optional reported-USD ceiling, persisted outside tabs/runtimes. Reserve
  durably before every client wire dispatch, including queued sends/retries; block
  the next request without cancelling the final allowed request merely because it
  consumed the request cap. Expiry, stop, storage errors and unverifiable cost
  accounting deny tool permissions, including automatic grants, and request
  cancellation in foreground/parked sessions. Pending queue text is retained on
  budget stops; the queue itself is not a durable restart queue.
  Only finite nonnegative ACP **USD** observations count; missing/non-USD/stale
  observations and opaque child-work cost coverage remain unknown. USD-limited
  sends require matching observations within 30 seconds. This is **not** a
  guaranteed provider billing cap: late/in-flight charges can exceed it, and
  harness-internal retries/child turns are not client request attempts.
  Up to 64 task records; explicit stop/release/forget controls. Release requires
  idle sessions and empty queues. Forget only released records, oldest overlapping
  memberships first: deleting a release must never reactivate an older budget.
- **Compare solutions v1:** composer → **Compare…**. Prepare two detached worktrees
  at one clean committed HEAD, then separately confirm creation and both adapter
  starts. Imports two local trusted projects with Ask permissions, no group context
  and the approved effective environment. Identical approved prompt, independently
  selected harness/model IDs, one shared durable request/time ledger; no USD cap
  is implied. Records observed model IDs and claims each dispatch durably before
  submission. Changes to project/settings/budget invalidate pending dispatch.
  Restart loads evidence but never automatically resumes/submits comparison work;
  a failed/uncertain dispatch is not retried. Diffs, observed costs (or unknown),
  identical separately confirmed checks and a human preference are displayed.
  Preference never merges/copies files. Worktrees are **not security sandboxes**;
  hooks and external writers are rechecked, not made transactional. Partial trees
  are retained and reported. Up to 16 comparison records, with confirmed metadata/
  log removal; files, worktrees, projects, sessions and budgets remain separate.
- **Impact map v1:** composer → **Impact…**. Bounded background Rust AST/lexical
  indexing with module/import aliases, direct references, transitive dependent
  symbols and candidate tests. Reuses unchanged parse units and invalidates on
  watcher/Git changes. Source links open the existing editor; sharing re-verifies
  saved source/inventory/HEAD and appends evidence to the existing draft, never
  sends it. No build scripts, macros, LSP or project code are executed. This is
  **not compiler/type/cfg-complete semantic indexing**, and candidate tests are not
  proven coverage. Dynamic receiver methods, macro expansion, ambiguous paths,
  external crates and non-Rust code remain explicitly unresolved/outside scope.

**Remote workbench (new):** granted relay projects can capture/read/restore
checkpoints through `/v1/review/*`, create real A/B worktrees with explicit
relay projects through `/v1/compare/*`, inspect relay-reported context through
`/v1/context`, and use `/v1/github/*` with the same typed local previews. Remote
GitHub is administrator-only because it uses the relay host's `gh` account, not a
per-member GitHub token. Remote comparison checks run through the existing
bounded `/v1/checks/run`; no local path or `gh` fallback is used. Group agents may
request multiple roots only from explicit same-host relay grants; the relay
revalidates every grant on each request and the sandbox permits only those roots.

**Still pending:** full compiler/LSP/multi-language impact mapping, complete
provider/internal-child billing attribution, automatic per-turn provenance/test
explanations, further test adapters, manual acceptance of the remote surfaces,
real GitHub account/relay transport, and full virtualization/auto-layout for the
largest graphs. All eight feature areas have bounded implementations, not full
completion of every capability in the approved proposals.

### Bounds and acceptance notes

- GitHub read child limit: 20 seconds / 4 MiB response; writes: 30 seconds with
  bounded stdin/output and process-group cleanup. Raw CLI/server errors and tokens
  are not echoed. Native preview is capped at 65,000 encoded bytes; oversize
  requests fail rather than hiding part of the publication behind truncation.
- API page caps: 20 exact-SHA runs, 50 updated PRs, 100 jobs/files/comments/checks.
  UI caps are labeled: 20 jobs, 100 steps/job, 20 files and 20 discussion entries;
  patches/bodies/logs have explicit truncation markers. Binary/unavailable patches
  are not presented as empty diffs. Log/PR text is plain, stripped of ANSI/OSC and
  directional overrides before display and marked untrusted when shared.
- Checkpoints store dirty/untracked non-ignored overlays (16 MiB encoded cap,
  2 MiB/file, 2,000 changed paths) plus immutable HEAD identity; three retained
  baselines maximum. A bounded 20,000-file permission inventory preserves even
  private permissions on otherwise clean tracked files; restores never create a
  broadly readable temporary file before applying the saved mode. Baseline
  deletion is explicit. No ignored files, unsaved text, index,
  refs, submodules, symlinks or directory restoration. Clean baseline files depend
  on their Git objects remaining available; missing objects fail without fetching.
  Capture/review has a ten-second work budget checked between bounded children;
  external writers are not made transactional. Restore never retries/rolls back.
- Checks: 120-second process-group limit, 512 KiB per stream, 200 Cargo diagnostics;
  launch/capture/timeout/cancel failures are never inferred as successful testing.
- Remote review/compare payloads are bounded by the same checkpoint/restore limits;
  restore previews are one-time server-side plans with a five-minute expiry.
  Remote GitHub reads/writes require relay administrator role; project grants are
  still checked. Multi-root ACP connections require explicit same-host project
  grants and reject MCP servers or paths outside those grants.
- Comparison checks keep at most two latest reports (40 KiB stored per stream);
  diffs show the first 20 changed files per side / 80 KiB text. Checks may predate
  later edits. Impact indexing: 30,000 inventory paths, 500 module scopes, 512 KiB
  per source, 20 MiB source total, 10,000 symbols, 100,000 raw paths / 50,000 edges,
  ten-second work budget between bounded reads/parses. Unsupported/ambiguous
  results are omitted with warnings, never advertised as an exhaustive graph.
- Automated fixtures use a fake `gh` executable (no real account/writes), disposable
  repositories, binary/stale/symlink restore cases and real isolated check processes.
  `--github-smoke` renders CI/PR, context/decisions, real checkpoint review and a
  check-report fixture in a fresh native window/database. It does not accept real
  publication/restore dialogs or call a provider/GitHub.
- `--workbench-smoke` uses only the bundled `mock_acp_agent` in a disposable
  database/repository: two actual ACP connections, distinct observed models, one
  prompt each, shared cap, last-request tool approval, parked permission denial/
  cancellation, restart-registry non-resumption/no replay and impact/budget drawing.
- Manual native dialog/tooltip/keyboard/a11y acceptance, real GitHub account/CI/PR
  acceptance and deployed relay transport remain pending. Enterprise/custom hosts,
  fork-head creation, reviewer/assignee/label administration, merge queues and
  full live log streaming are not claimed by the current slice.

### Current validation

**527 tests passed, 0 ignored**; workspace desktop/relay build, Clippy
`-D warnings`, formatting and `git diff --check` passed. The new
native GitHub/workbench smokes passed dark and light/narrow, as did the Git controller and
Markdown regressions. The paired loopback HTTP Git smoke also passed. These do
not replace real GitHub account/deployed-host/manual dialog acceptance.
Known non-fatal toolchain warnings (`rust-objcopy`/`libLLVM.dylib` and `block`
future compatibility) remain separate from functional results.

```sh
cargo build -p dunlin
cargo build -p dunlin-agent --bin mock_acp_agent
target/debug/dunlin --github-smoke "${TMPDIR%/}/dunlin-git-smoke-$(uuidgen)"
target/debug/dunlin --github-smoke "${TMPDIR%/}/dunlin-git-smoke-$(uuidgen)" --light-narrow
target/debug/dunlin --workbench-smoke "${TMPDIR%/}/dunlin-git-smoke-$(uuidgen)"
target/debug/dunlin --workbench-smoke "${TMPDIR%/}/dunlin-git-smoke-$(uuidgen)" --light-narrow
```

## Delivery sequence and acceptance

1. **GitHub CI/PR** (core implemented above; further capabilities/QA pending): installed `gh`, existing authentication, explicit
   repository ownership; workflow → job → step statuses, attempt/SHA pinning,
   bounded logs and CI evidence added to a composer draft without sending it.
   PR list/details/files/diffs/checks/discussion/reviews; create/edit/comment/
   review/close/reopen/merge with fresh preflight and native confirmation.
   No implicit push/checkout, admin override or automatic retries. Live step
   status is polled; downloadable logs are not advertised as a live stream.
   Local `gh` must never act on a relay project as a fallback.
2. **Task review**: task baseline, actual diff and changed-file provenance,
   recorded test/command evidence and explicit unknown/unverified state;
   line comments can be added to an agent draft. Closing review preserves buffers.
3. **Context inspector**: exact application-controlled context before dispatch,
   scope/instruction/skill provenance, effective model and permissions;
   distinguish harness-managed context that the client cannot inspect.
4. **Tests/problems center**: structured test/diagnostic/command outcomes,
   bounded log, source navigation and selected evidence → agent draft. Running
   project commands requires explicit consent; unknown result is not success.
5. **Checkpoints**: explicit immutable baseline; preview selective restoration,
   editor safeguards and stale-baseline checks. No hidden reset/auto-rollback;
   snapshots have retention/size limits and do not publish secrets.
6. **Task budgets**: cost/time/turn accounting across child/retried work, pre-send
   limits and stop semantics; unavailable pricing/usage is unknown, never zero.
7. **Compare solutions**: isolated worktrees and explicit budget/agent starts,
   same baseline/input, diff/test/cost comparison and a human selection gate.
8. **Impact map**: real references/dependencies and affected tests, distinguish
   indexed evidence from heuristics, incremental background updates.
9. **Decision memory**: user-approved editable/versioned decisions, links to
   evidence, stale markers and explicit inclusion in future context.

Every stage requires local/remote capability boundaries, fixtures, stale-result
guards, formatting/Clippy and documentation of manual QA still pending. The
GitHub initial slice targets GitHub.com only; Enterprise/custom hosts, fork-head
creation and relay transport must be explicitly implemented, not inferred.
