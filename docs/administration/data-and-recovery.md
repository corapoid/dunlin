# Data, backup and recovery

## Locations and backup scope

| State | Default location / owner |
| --- | --- |
| Desktop database/settings/history | `~/Library/Application Support/Dunlin/dunlin.db` and SQLite sidecars. |
| Window preferences | `window.json` under the active desktop data directory. |
| Scratch chat files | `chat-spaces/<id>/` under the data directory. |
| Artifact blobs, audits, proposals, installed packs, SSH sockets | Corresponding subdirectories of application data; some tool/helper paths depend on the invoking process's HOME. |
| Lifecycle log | `~/Library/Logs/Dunlin/dunlin.log` and rotation, independently configurable. |
| Desktop vault values/policies and pairing tokens | macOS Keychain, not a portable JSON export. |
| Source files and worktrees | Their actual project/worktree directories, not copied into the history database. |
| Relay state | macOS `~/Library/Application Support/Dunlin/relay.db`; Linux `~/.local/share/dunlin/relay.db`, plus host-managed files/vault/coordination state. |

`DUNLIN_DATA_DIR` redirects desktop app data, not every platform credential store,
log, repository or remote service. A test database alone is not Keychain/TCC
isolation. Read [configuration](../reference/configuration.md) for exact overrides.

## Portable export versus full backup

[Settings/project export](settings-project-backup.md) preserves selected
preferences and project organization in JSON or an existing repository. It
**excludes conversations and credentials**, and imported catalog entries do not
download repositories until explicitly requested. It is not a full backup or sync.

For a full local recovery copy:

1. Save buffers, stop work and close **all** Dunlin processes using that database.
2. Back up the whole active data directory, including blob files and any SQLite
   `-wal`/`-shm` sidecars that remain. Do not copy only a live `.db` file and assume
   it is a consistent backup.
3. Back up project/worktree directories separately. Plan platform-approved
   Keychain/account recovery separately; do not export credential values into Git.
4. Protect the backup as sensitive data. Transcripts, drafts saved elsewhere,
   paths, audit artifacts and logs can contain private content.

Relay backups require stopping/coordinating the relay service and protecting
its database, managed directories, blob/vault metadata and platform credential
store separately. Copying a project to a relay is a one-time snapshot, not backup
synchronization.

## Restore and diagnose

Keep a recovery copy before opening older data with a newer build. Migrations
run on storage open; there is no promised downgrade/migration rollback. Restore
into a separate directory with the app stopped and verify version/schema/history
before replacing a working setup. A restored database does not guarantee a
harness-owned ACP session can resume.

Use **Settings → Diagnostics & About → Troubleshooting** to inspect the selected
directory, schema/quick-check and counts. Missing history, a failed quick-check
or a migration error warrants preserving evidence, not deleting the database.
See [troubleshooting](troubleshooting.md). [Factory reset](factory-reset.md) is an
explicit destructive application-data operation, not a repair step to try first.

Sources: [data directory](../../apps/desktop/src/app/helpers.rs),
[storage](../../crates/storage/src/lib.rs),
[logging](../../apps/desktop/src/logging.rs),
[relay config](../../apps/server/src/config.rs).
