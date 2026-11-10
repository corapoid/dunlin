# Linear through MCP

Open **Settings → Remote · Tailscale**, choose **Manage** on a paired relay, then
**Linear → Connect Linear…**. Enter a Linear API key or access token, the issue
label (for example `dunlin`) and optionally a Linear team. Choose **Connect and
save token on server**. This explicitly sends the token to that relay.

The relay connects directly to Linear's official Streamable HTTP MCP endpoint,
`https://mcp.linear.app/mcp`. No plugin, Node process, `npx`, local agent or model
request is involved. Start with a read-only Linear API key if you only need the
inbox. Interactive OAuth login and automatic OAuth refresh are not implemented;
an expired access token requires reconnecting the integration.

## Scope and credentials

The connection and cached inbox belong to **your relay user**. Another device
paired as that same user sees the same configuration; a different relay user
does not inherit the token or inbox. Configure each intended execution relay;
there is no automatic copying of credentials between hosts.

Tokens live in the relay OS keyring (Keychain on macOS, Secret Service on Linux),
not in SQLite, app settings/backups, logs or agent context. They are not
registered as agent-accessible vault credentials. A missing/locked keyring is
an error, never a reason to use a plaintext file. Revoked client devices cannot
read or change this integration. Existing administrator-only host Git account
restrictions still apply to the relay API.

**Disconnect** removes this user's configuration and cached issues, not the
issues in Linear. Key deletion is retried automatically if the keyring is
temporarily unavailable. Rotated credential cleanup is also durable.

## Synchronization

While `dunlin-relay serve` runs, configured integrations refresh approximately
once a minute, including with all desktops closed. Revoking a device disconnects
that client; it does not disable an already configured server integration. Use
Disconnect from another authorized device to disable synchronization.

The desktop displays the server's cached tagged tasks, their identifiers,
titles, statuses, last successful sync time and links to Linear. **Sync now**
requests an immediate refresh. Opening the panel only reads the cache.

The adapter discovers tools and checks the required filter inputs. It follows
pagination, checks returned labels, rejects duplicate issue ids and imports at
most 500 issues / an 8 MiB cached inbox. Unsupported schemas or exceeded limits
are explicit errors, not an unfiltered or silently partial success. Narrow the
label/team filter when necessary.

On failure the previous complete snapshot is retained and marked with an error
and its original timestamp. This is not an empty inbox or evidence that tasks
were completed. Redirects and automatic mutation retries are disabled. The MCP
client supports JSON and SSE responses, bounded tool catalogs, session/version
negotiation and status-only calls through an advertised Linear update tool.

## Assign and execute

Choose **Assign to host…** on a tagged issue, select a paired Tailscale server
and a granted Git project, then enter the exact harness/model/mode and review
status. Review the native approval dialog before enqueuing. Each target needs
its own Linear authorization and a fresh, identical issue snapshot. Tokens are
never copied from the source inbox to another host.

The relay owns a SQLite-backed queue shared by its users and runs **one job at
a time per host**, even with every desktop closed. Approval authorizes one ACP
turn, 30–3600 seconds including preparation, and 1–256 client tool requests.
These are not dollar caps or limits on invisible harness-internal model calls.
The permission switch governs client permission/write requests; disabling it
does not make harness-native tools a read-only sandbox.

Execution requires the relay sandbox and dedicated agent homes. Each run clones
committed HEAD into a private independent checkout, creates a `dunlin/job-…`
branch and removes its origin. Uncommitted source edits are excluded. Agents
receive no Linear token. The worker does not automatically push or merge;
inspect the granted review project and reported checks yourself. The sandbox
remains a best-effort host privacy boundary, not a hostile-code container.

Normal `end_turn` means **Review required**, not verified completion. A durable
status outbox rereads Linear and sends the approved review status only if the
initial status and required label still match (or accepts an already-matching
review status). Pre-mutation connection failures retry after a delay. Ambiguous
mutation failures require explicit reconciliation; they are not auto-replayed.
MCP has no compare-and-swap update: human changes between reread and mutation
cannot be ruled out.

## Queue management and recovery

Owners/admins can cancel or archive eligible records. Administrators can pause,
resume, prioritize queued work and reconcile interrupted execution after
verifying that orphaned processes have stopped. Pause stops new dispatch, not
the current run. Device revocation, removed project authority, changed project
settings or changed Linear authorization stop a run at its next check.

After relay restart, an executing job becomes **Interrupted** and blocks new
dispatch until administrator reconciliation. No prompt is automatically
replayed. Interrupted status delivery similarly becomes unknown. Archiving
removes only a finished record with resolved delivery, not its review checkout.
Use **Mark status manually resolved…** after handling Linear yourself; it sends
no status mutation. History is bounded to 512 records per host. A compact
approval ledger remains after archiving to prevent replay of old requests
(100,000 approvals per host before administrator maintenance is needed).

Queue visibility and server-card workload counts follow explicit project
grants. Peer-user reports are withheld from non-admin viewers. Deduplication
prevents simultaneous active copies of an issue **on one host only**. There is
no distributed cross-host lock: do not approve the same issue concurrently on
multiple hosts.

## Acceptance boundary

Tests use local MCP fixtures. Production acceptance still requires an explicitly
authorized Linear workspace, its current tool schemas and the real relay keyring.
