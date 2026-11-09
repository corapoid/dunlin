# Persistence and lifecycle

## Authorities

| Data | Authority | Important boundary |
| --- | --- | --- |
| Source and governance Markdown | Files in the project. | A derived index must not silently rewrite them. |
| Local projects/groups, session summaries/transcripts, settings | Desktop SQLite store. | This is not an encrypted credential store. |
| Knowledge/spec discovery | Rebuildable SQLite indexes derived from files. | Decisions/evidence are separate persisted records; not all database tables are disposable caches. |
| Captured artifact content | Blob files plus metadata. | A database backup without blobs does not preserve artifact content. |
| Live agent context | Harness-owned ACP session. | A rendered saved transcript is not sufficient to recreate provider state. |
| Remote users/devices/grants/session index | Relay database. | Desktop removal does not delete remote authority. |
| Credential values/policies | Platform credential store. | Display metadata in SQLite is not authority to widen a grant. |

## Desktop storage

[Store](../../crates/storage/src/lib.rs) opens SQLite with WAL/foreign-key and
migration support. Schema version is available through `PRAGMA user_version` and
the troubleshooting report; do not hard-code the current migration number into
general user instructions. Add migrations and preservation tests together.

Settings serialize as JSON within SQLite so preferences can evolve without a
table per option. Transcripts are persisted at lifecycle boundaries such as turn
completion, permission handling, session switches and shutdown. They are not a
guarantee that every unflushed streaming chunk survives a forced kill.

Canonical file buffers, Undo stacks, drafts and pane layout have launch-local
state. Durable coordination records, subtask worktree registry, budget ledger and
comparison evidence must not be mistaken for authorization to replay paid work
after restart. See [workspace lifecycle](../user-guide/session-tabs-and-ui-preferences.md)
and [coordination](../user-guide/agent-coordination.md).

## Remote storage

The relay can store transcript snapshots (default) or metadata only. A live
session has one active device lease, with explicit takeover. Disconnect/linger
is bounded; pending permission requests are not automatically approved because
the desktop disappeared. Remote credentials, audit artifacts, blobs and process
state remain host-scoped.

## Recovery implications

Use [data and recovery](../administration/data-and-recovery.md) for locations and
backup scope. Portable settings export is not full history backup. Reset removes
application records, not repository trees or an assurance of forensic erasure.
Schema downgrade is not a restore mechanism.
