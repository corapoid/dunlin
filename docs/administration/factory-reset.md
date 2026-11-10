# Factory reset

Reset is destructive and cannot be undone through the app. Make a
[backup](data-and-recovery.md) first; do not use it as the first diagnostic step.
Popover/menu implementation rules belong in [UI surfaces](../development/ui-surfaces.md#floating-lists-and-modals).

## Reset contract

**Settings → Data & Backup → Recovery → Reset…** opens a themed warning with **No**
first/default and **Yes, reset Dunlin** second. Dismissal, failure to receive an
answer and every answer other than the explicit Yes leave data untouched.
**Esc** or **Enter** on the dialog itself selects No. If reset is blocked, a
visible dialog explains what must finish or be closed (including idle terminal
shells), rather than leaving a workspace notice hidden behind Settings.

Before confirming and again before starting, reset refuses unsaved editors,
running terminals/agents, pending imports, credential/skill writes and active
file/Git/security/comparison operations. During reset, application input, menu
actions, ordinary Quit/window-close and window-geometry persistence are blocked.

After confirmation, Dunlin disconnects adapters and revokes their vault sessions,
stops the broker and drains its in-flight request/cleanup writers off the UI
thread. It deletes the desktop vault values/policies recorded in storage and
current/legacy relay tokens from Keychain, removes `window.json`, then clears
application records (including pending imported histories and knowledge/evidence/plugin/SSH metadata) in one
SQLite transaction. Schema/migrations remain usable. Old in-memory history is not
saved again on exit; Dunlin restarts automatically with defaults and an empty project list.

No directory tree is deleted, even with a custom `DUNLIN_DATA_DIR`:

- Repository files, worktrees, standalone-chat files and generated audit files stay.
- Installed agents, external provider/CLI logins, skills/extensions on disk and
  diagnostic logs stay. Existing installed tools can be rediscovered after restart.
- Relay/server-side projects and histories are not deleted; only local pairings go.
- Do not infer deletion of SSH passphrases, provider credentials or every
  Dunlin-related platform item: the reset implementation targets the desktop
  vault and known relay tokens, not a blanket Keychain purge.
- This is an application reset, not forensic secure erasure of SQLite/free disk space.

Close **all other Dunlin instances** first, including separately named test builds.
The warning states this requirement: another process can save old data again.
This change does not introduce a cross-process single-instance lock.

Keychain, filesystem and SQLite cannot share a transaction. A Keychain or preference
failure keeps database metadata for retry, though some credentials/window preferences
may already have been removed. A database failure rolls back all table deletions.
On failure, the app reports the partial result and keeps adapters/broker stopped;
retry reset or restart before using agents again.

## Verification

- `cargo test -p dunlin-storage`: all application tables populated/cleared, repeat
  reset, reopen/write after reset and rollback on an injected last-table failure.
- `cargo test -p dunlin --bin dunlin`: explicit confirmation, memory-only credential
  deletion/retry, preference failure and repository-file preservation; broker drain.
- `--glass-smoke <fresh dunlin-glass-smoke-… directory>`: both branch presentations,
  command/remote/context/project lists without dimming, Escape/search focus,
  project outside dismissal, blocked menu actions/Quit during simulated reset,
  plus existing AppKit/modal input regressions.
- `--onboarding-smoke <fresh dunlin-onboarding-smoke-… directory>`: startup/import
  regressions with isolated application data.

Automated reset tests never target the user's database or Keychain. Real-data reset
and physical keyboard/IME/visual acceptance remain user-controlled manual QA.
