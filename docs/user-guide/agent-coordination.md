# Project agent coordination

Dunlin exposes observed project peers, durable dependencies, bounded result
reports and an inbox to ACP agents through a built-in stdio MCP server. A skill
explains the workflow; it is not the source of status and is not needed for tool
discovery. This is sequencing of existing conversations, not an autonomous
agent factory, repository lock or automatic merge system.

## Use it

1. Connect/reconnect the project harness so it receives `dunlin-coordination`.
2. Ask, for example: **“Implement this task after the currently running agents
   in this project finish.”** The agent calls `continuations_create` and ends
   its current turn. It must not implement the future task early.
3. Review the **Deferred turn** card in the chat. **Review & approve…** shows the
   exact prompt, fixed dependency ids, root, harness, model, mode and permissions.
   Approval authorizes one later model request, including with that chat parked.
4. Dunlin waits for normal completion of both the requesting turn and its
   dependencies. It sends one continuation through the normal provider/model,
   group-context, permission and task-budget gates. The model does not poll.
5. The continuation receives bounded predecessor reports and instructions to
   refresh files/Git before editing. Other worktrees are not silently merged.

**Open Agent Coordination** in the command palette opens the bottom **Agents**
panel: latest observed project runs, queue observations, reports, tasks, current
continuation controls and the selected conversation's inbox. Messages require
acknowledgment; receiving one never interrupts or starts a paid turn.

Local **Change dependencies…** replaces the fixed list with a reviewed snapshot
of current peer runs and revokes the old execution approval. For an exact subset
or task dependencies, cancel and ask the agent to propose explicit ids. The same
cancel-and-replace workflow works on a relay. **Cancel** cannot recall a prompt
already sent; stop its normal agent turn instead.

The optional **Project agent coordination** skill is in the bundled Skills
catalog and follows the existing explicit, ownership-checked install workflow.
The MCP initialization instructions and a short local prompt envelope make the
tools discoverable without installing files in a repository.

## What “finished” means

- A **session** is the long-lived chat. A **run** is one concrete ACP prompt.
  A **task** is a named goal that can span multiple runs.
- Dependencies refer to immutable run/task ids, not the current state of an
  indefinitely reused chat.
- Omitting `dependencies` atomically snapshots active peer runs in the granted
  project scope, excluding the caller. Later agents are not added. A connection
  which has not started a prompt is not yet an observed run.
- `end_turn` becomes `turn_finished`, not “verified task success”. Refusal,
  token limits, errors, cancellation and lost observation are not success.
- Permission waits remain active dependencies. Unsent queued prompts are shown
  separately; waiting for a run does not mean waiting for a conversation's entire
  future queue. Relay queue observations are **unknown/null**, not zero, because
  other devices' unsent composer queues are not visible to the server.
- `runs_report` records the agent's summary/evidence/blockers. If absent, the
  desktop/relay stores a bounded final-text summary. Neither is verification.
- `tasks_report` records a terminal success/failure **claim** by the owner.
  Blockers force failure. Task dependents wait for the reporting run to finish
  normally and all recorded member runs to stop. Earlier failed attempts remain
  evidence, rather than preventing an explicitly reported successful retry.

Self-dependencies, duplicate dependencies and session wait cycles are refused.
A new user turn in the target chat invalidates an older continuation instead of
letting it unexpectedly run after unrelated work. Queued user prompts take
precedence. The scheduler does not acquire exclusive access to a worktree.

## Tools

| Tool | Purpose |
| --- | --- |
| `agents_list` | Latest observed runs in the capability's scope; 32 sessions/page. |
| `runs_get` | Run facts, execution scope and bounded report. |
| `continuations_create` | Propose the exact future prompt; no approval or execution granted. |
| `continuations_list` / `continuations_cancel` | Read/cancel the caller's proposals; 8/page. |
| `messages_send` | Deposit a same-project peer message with an idempotency key. |
| `inbox_read` / `inbox_ack` | Read/ack the caller's inbox; 8/page. |
| `runs_report` | Record the caller's current run result/evidence/blockers. |
| `tasks_create` / `tasks_list` / `tasks_get` | Register an owned goal and inspect project task evidence. |
| `tasks_report` | Record an owned task's terminal result claim. |

Proposals, messages and task creation use stable idempotency keys. Reusing a key
with different content is rejected. Explicit archival releases old dedupe keys;
use new keys for new work. Agents cannot approve execution, launch another
agent, add a worktree scope grant, read arbitrary transcripts, reveal a broker
token, merge or push through this surface. Reports and messages are untrusted
data, not permission to expand the user's request.

## Durability and security

`dunlin-coordination` is a harness-independent Rust crate. Its bounded, versioned
SQLite ledger lives under the desktop data directory at
`coordination/coordination.sqlite`, or beside the relay database in its
`.coordination` directory. It uses WAL and FULL synchronous commits, with a
process ownership lock and a private directory. Corrupt/newer data is rejected,
never overwritten with an empty success state.

The Unix broker and MCP proxy use random, memory-only session capabilities.
Project/user identity and root are granted by the app/server, never tool input.
The proxy does not open the database. Capabilities are revoked with their runtime
and follow trust/budget revocation. The socket directory is `0700`, socket `0600`;
frames, client concurrency, input lines, reports and lists are bounded. Same-user
native agents are not a sandbox against attacks on the app process or its files.

A continuation records a durable wire reservation and new run identity **before**
sending. Duplicate dispatch is refused. A crash between reservation, wire send
and acknowledgment is ambiguous: the system preserves evidence and requires
manual reconciliation; it does not claim distributed exactly-once delivery or
blindly resend. Startup marks interrupted observations unknown and waiting/
dispatching approvals as needing attention. Completed dependency facts survive.

Approval expires after one hour. A changed root, configuration digest, model,
mode, permission policy, failed budget gate or unknown model/mode blocks deferred
execution. No automatic plan-to-implementation switch is permitted. Cancelling a
dependent does not cancel its predecessors. **Archive finished coordination
history** removes only scoped, terminal, unreferenced records; live dependency
evidence remains. Bounded limits are 2,048 runs, 512 tasks/continuations/messages,
64 dependencies, 16 KiB prompt/message/summary and 32 evidence/blocker entries.
The ledger retains the latest 4,096 audit metadata entries; it is not an
unbounded forensic archive of every past interaction.

## Worktrees

Projects are isolated by default, even when paths belong to the same repository.
For a registered subtask worktree opened as a project, use **Share coordination
with parent project…** in Agents. The native confirmation binds exact child and
trusted parent ids/roots; only then does the worktree use its parent's peer scope.
Finish/stop agents first and reconnect after changing the grant. One-level grants
are explicit, durable and removable; no path heuristic silently widens visibility.
This grants coordination visibility, not filesystem permissions.

## Relay and multiple devices

The relay injects `dunlin-relay --mcp-coordination` into ACP new/load/fork sessions
and records runs from its own request/response stream. Its ledger is scoped by
**relay user + project**, so another granted user does not inherit private agent
messages or proposals. Session ownership, device authorization and project grants
are rechecked; the broker reauthorizes every call. Normal ACP leases still apply.

Authenticated `POST /v1/coordination` supports scoped snapshots, exact-proposal
approval, cancellation/pause, inbox acknowledgment and archival. It never accepts
a caller-chosen filesystem root or executable. Snapshots are capped at 8 MiB.
The desktop fetches them off the UI thread, without interpreting relay paths as
local ones or falling back to local history on old/unavailable relays.

Remote approval is bound to the approving device **and live ACP connection**.
Only that device's connected/parked conversation executes it, through normal
client budget/permission gates. Before forwarding, the relay atomically reserves
the continuation and replaces caller text with the server's exact approved task
plus bounded result data. Changing devices, losing the lease, reconnecting or
restarting requires review again. The approving desktop must remain running to
execute a future turn: there is no unattended relay scheduler bypassing desktop
budgets while all clients are offline. In-flight turns retain the existing relay
linger behavior, but a later continuation is not auto-started by the linger loop.

Unmanaged external agents, opaque harness-internal children, automatic task
decomposition/spawning, role routing, automatic review/merge and cross-user
shared governance are outside this feature's authority. Unknown stays unknown.

## Verification

```sh
cargo test -p dunlin-coordination -p dunlin-agent -p dunlin-relay
cargo test -p dunlin --bin dunlin
cargo build -p dunlin -p dunlin-agent --bin dunlin --bin mock_acp_agent
./target/debug/dunlin --coordination-smoke /absolute/fresh/dunlin-coordination-smoke-example
```

The registered native scenario drives an actual mock ACP harness, calls the Unix
MCP broker, approves a fixed two-run barrier in an isolated fixture, switches away
from the requester and verifies exactly one background send with the original
plan mode and unchanged foreground/startup selection. It performs no real model
requests and also verifies that a stopped task budget blocks another approved
continuation. Rust tests cover stop reasons, both finish orders, retry/idempotency,
task claims, authorization/revocation, cycles, expiry, persistence/recovery and
server-side prompt-substitution rejection.

Manual acceptance still requires reviewing real native approval dialogs with
keyboard/VoiceOver, real ACP adapters/MCP discovery, and two paired devices on a
deployed relay (disconnect, takeover, restart and budget revocation). Automated
fixtures are not evidence that these real-host checks passed.
