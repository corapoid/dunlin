# Agent workbench

The composer exposes **Context…**, **Review…**, **Checks…**, **Budget…**,
**Compare…** and **Impact…**. These are explicit evidence/review tools, not an
autonomous coordinator. PR/Actions functionality is not part of the current app.

## Context and decision memory

Context previews the application-controlled envelope and effective runtime
sources. Environment names are visible, not values. Harness-owned system prompts,
history and loaded instructions can be unknown; the inspector does not claim to
see a provider's entire context. A queued turn can have a different envelope at
dispatch time.

Approve project-scoped decision notes explicitly. Revisions remain stored; stale
notes cannot be inserted. Adding a current decision appends it to a draft, not to
every future request. These local records are unencrypted, not a secret vault.

## Review and checkpoints

Capture a saved-file baseline **before** a task, then inspect changes and append
selected evidence/comments to the draft. Preexisting dirty/untracked saved files
are part of the baseline, not attributed to the agent.

Selective one-file restore requires a fresh preview and confirmation, current
file/HEAD checks and a Git-dir lock. Save/close file tabs and stop agents first.
Checkpoints exclude unsaved buffers, ignored files, index/refs and symlink or
directory restoration. Current bounds include three baselines, 2 MiB per file,
2,000 changed paths and 16 MiB encoded payload. No automatic retry/rollback,
fetch, merge or buffer discard is implied.

## Checks

Review exact JSON argv/cwd before running project code. Ordinary workbench checks
are **not sandboxed**. Reports contain actual exit status, bounded output and
available Cargo diagnostics; failure/timeout/cancellation is not success. Current
limits include 120 seconds and 512 KiB per output stream. A passing old report
does not establish correctness after later edits or prove coverage.

## Budgets

Durable request-attempt/time limits reserve before dispatch, including retries
and queued sends. Optional USD limits use observed ACP USD usage and fail closed
when required accounting is unavailable. Harness-internal retries and child work
are not necessarily visible client attempts. Late charges can exceed a reported
threshold: this is **not a guaranteed provider billing cap**.

Stop/release/forget have distinct behavior; forgetting released records must not
reactivate older budgets. Restarts load evidence, not an automatic paid-work replay.

## Compare and impact

Compare prepares two detached worktrees from one clean committed HEAD. Creation
and adapter starts need separate authorization. Both sides share approved input
and request/time limits; unknown costs stay unknown. A human preference does not
merge/copy files. Partial worktrees are retained for inspection. Restart does not
automatically resubmit a comparison.

Impact performs bounded incremental Rust AST/lexical indexing, showing references,
dependents and candidate tests. It runs no build scripts/macros and is **not
compiler/LSP/type/cfg-complete analysis**. Candidate tests are not proven coverage.
Sharing rechecks saved source/HEAD and appends evidence without sending it.

## Remote boundary and validation

Granted relay projects use explicit server-side review, compare, context and check
endpoints, not this Mac's paths or tools. Old relays can require an update. Native
fixtures exercise isolated data/mock ACP; real-provider, physical dialog/keyboard,
accessibility and deployed-relay acceptance remain separate. See
[test quality](../quality/test-quality.md) and [manual acceptance](../quality/manual-acceptance.md).

Sources: [context](../../apps/desktop/src/workbench.rs),
[review/check UI](../../apps/desktop/src/git_history),
[budgets](../../apps/desktop/src/task_budgets.rs),
[comparisons](../../apps/desktop/src/comparisons.rs),
[workspace backends](../../crates/workspace/src),
[relay routes](../../apps/server/src/api.rs).
