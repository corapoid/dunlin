# Usage, account limits and billing evidence

Three measurements answer different questions. None should be substituted for
another or silently filled with zero.

| Surface | Meaning | Boundary |
| --- | --- | --- |
| Session usage | Observed ACP counters/deltas and reported costs. | Context occupancy is not consumed tokens; legacy/unreported data stays unknown. |
| Account usage | Available allowance/rate-limit data for the active environment/harness/provider. | Missing, cached and unavailable data are explicit; no guessed percentages. |
| Settings → Usage & Limits → Cost report | Bounded scans of local provider billing evidence. | Priced totals are a lower bound when models or usage are unknown. |

## Account scope

Local Claude, Codex and OpenCode accounts are not mixed; remote projects do not
borrow this Mac's accounts. OpenCode Free does not inherit paid-plan allowances.
Only the selected local harness is probed. Applicable OpenCode Go/Console limits
require a running OpenCode service at 2.2+; updating the CLI alone does not restart
that service. Finish active work before a service restart.

Relay host-account usage is administrator-only. Members see an explicit restricted
state, not another user's budget. A successful sign-in or model list is not proof
of balance, subscription entitlement or complete billing coverage.

## Billing scans and attribution

The local scanner reads supported Claude Code JSONL, Codex rollout JSONL and
OpenCode SQLite stores read-only. It deduplicates provider messages, checkpoints
complete lines and reports source/offset/errors rather than claiming a complete
scan of every provider. Work attribution is **Main conversation**, **Subagents**,
**Internal / synthetic** or **Unknown attribution**; missing evidence is not
assumed to be main work.

Known-priced monthly alert preferences warn at 80% and 100% of configured budgets.
They are alerts, not hard spending caps. Task dispatch limits in the
[workbench](agent-workbench.md#budgets) are a separate control with their own
accounting limitations.

**Copy QA report** exports aggregate evidence, source states and warnings, not
prompts or raw log contents. **Sync evidence** explicitly uploads aggregates to
paired relays; it is not background synchronization. Members can upload their own
evidence; administrators can read per-device aggregates. Review metadata before
sharing. See [attribution/sync contracts](standalone-chat-and-worktrees.md).

Sources: [usage UI](../../apps/desktop/src/usage.rs),
[account probes](../../crates/usage/src/lib.rs),
[billing](../../crates/usage/src/billing.rs).
