# Workspace polish, hosting links and billing repair

## Scope and discovery — 2026-11-05

Remove dashboard metric controls (explicitly confirmed by user), move sidebar
toggle to the leading title-bar slot, center tab close controls and allow closing
Chat without deleting/stopping its session, improve both palettes, remove PR and
Actions integration, and replace it with a read-only repository-host link.

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| GitHub PR/Actions currently includes automatic detection, polling, UI, spec evidence and relay mutation routes | desktop `git_history/{github.rs,mod}`, `spec.rs`; relay `api.rs` | working tree | high | none | remove reachable support, retain imports/topics |
| Repository identity must come from configured Git remotes, not project display names | workspace `github.rs::{metadata_identity,repository_name}`, `git/activity.rs::RemoteStatus` | working tree | high | GitLab parsing | local parser tests for HTTPS/SSH and nested GitLab namespaces |
| Opus 5.5 is $4 input/$20 output/$0.20 cache-hit/$5 5m-write/$8 1h-write per million tokens | `https://platform.claude.com/docs/en/about-claude/pricing`, retrieved 2026-11-05 | official current pricing | high | none | price and cache-TTL tests |
| Existing byte bound counts entire file sizes before the cached/unread-offset check, so a large file can starve every scan | usage `billing.rs::scan_jsonl` | working tree | high | none | bounded incremental/chunk tests |
| OpenCode sessions can store directory/parent as SQL columns rather than JSON; message ownership can come from `session_id` | desktop `session_import/opencode_db.rs::{load_v1,load_v2}` and fixtures | supported v1/v2 schemas | high | billing coverage | read-only relational fixtures |
| Unknown attribution/prices must remain explicit, not silently assigned to main work or zero cost | usage `BillingEvent`, `WorkKind`, `Summary::alerts` | working tree | high | none | preserve unknown-model tests and deduplicate warnings |

```text
active project --configured Git remote URL (proven)--> validated public hosting URL
remote project --existing authenticated granted Git read (proven)--> relay remotes --> same parser
provider logs --bounded incremental complete lines (proven)--> token events --> documented model price
OpenCode message.session_id --SQL session key (proven)--> session.parent_id/directory --> attribution/project
```

No credentials are extracted, no network probe is needed to identify a repository,
and a generic hostname is not proof of a self-hosted GitLab instance. The minimal
hosting contract covers explicit github.com and gitlab.com HTTPS/SSH remotes;
unknown/ambiguous remotes use a disabled placeholder. Remaining real missing data
stays visible. Existing stored evidence is retained when removing PR/Actions.

## Verification

- Desktop: 373 tests passed; UI: palette edge/text contrast and maintained assets.
- Billing: 20 deterministic tests passed, including a 40 MiB JSONL scan, cached
  bytes, partial/oversized lines, incremental Codex deltas, OpenCode relational
  v1/v2 ownership, Opus 5.5 and 1-hour cache pricing, warning deduplication.
- The existing live Codex test failed separately. Local `codex-cli 0.46.0`
  accepts initialization but returns JSON-RPC `-32600 Invalid request` for
  `account/rateLimits/read`, also after sequential initialization. This is not
  evidence of available account limits. No CLI upgrade, login or credentials
  changes were performed; unavailable limits remain explicit.
- Native workspace acceptance passed with a rendered click on Chat's X,
  vertical-center measurement, a running-agent flag and unchanged transcript.
  A/B/context/review/check acceptance uses mock ACP only, not live providers.
- The default-shell relay run passed 47/48 tests; the PTY test did not observe
  shell exit. A separate run with `SHELL=/bin/sh` passed all 48 tests, including
  authentication/grants, terminal exit and 404s for removed PR/Actions routes.

Workspace unit results are recorded separately from the above desktop/UI runs.
No manual visual assessment or live hosting/account acceptance is claimed.

The initial workspace run timed out after 180 seconds and reported a comparison
stale-approval failure. That exact comparison test passed on its separate rerun
(no code change). The complete suite was rerun with a 600-second deadline,
`SHELL=/bin/sh`, `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1` to
isolate developer Git/shell configuration: all 139 tests passed in 220.25 seconds.
Separate targeted GitHub metadata/import tests (8/8) and hosting tests (2/2)
also passed. Final deterministic counts: desktop 373, workspace 139, UI 5,
billing/usage 20, relay 48 (585 passed across the separate runs).

Final native runs passed:

- Workspace UI from `target/debug/dunlin` and the signed application bundle:
  rendered Chat close, preserved transcript/running flag, Chat/Git X centering,
  existing project-picker, session, layout and themed-confirmation acceptance.
- Workbench with mock ACP, dark/wide and light/narrow: A/B permission/budget
  handling, context decision memory, preexisting saved-file checkpoint baseline,
  review diff and checks rendering. The initial moved review fixture lacked its
  preexisting disk edit; the fixture was corrected before both passing runs.
- Native Git safety: stash push/pop, dirty-buffer guard, canonical-buffer
  preservation, conflict/reload behavior and locks surviving tab close.

## Bundle

`dist/workspace-polish/Dunlin Workspace Polish.app` is a separate debug bundle;
previous application bundles were not replaced. Ad-hoc signing and
`codesign --verify --deep --strict` passed. This is not a notarized release.
Working-tree changes remain uncommitted.
