# Session import

**Settings → Agents → agent card → Import / Reimport** scans and copies the
selected harness's histories automatically. There is no assignment/selection preview.
The card's first line contains connection status, Test connection / Set up and
visibility. Its second line contains import actions, progress, warnings and the
number of imported sessions, including copies awaiting a matching project.
Import is available for OpenCode, Claude Code and OpenAI Codex. There is no manual
file picker; other harness cards do not offer an unsupported import action.

## Supported sources

- **OpenCode**: Dunlin opens the local `opencode.db` in
  read-only mode only and reads top-level sessions from the current schema
  (`session_v2` + `session_message`) **and** the older `session`/`message`/`part`
  tables. Readable V2 history wins for duplicate IDs; empty migrated V2 shells fall
  back to V1 individually. Early V1 schemas without a session-level model are supported.
  Pre-SQLite `storage/session`, `storage/message` and `storage/part` JSON is also scanned.
  It imports text, reasoning, tool history,
  saved token counters and file diff statistics; it does not read
   credentials, blobs or snapshots. Dunlin never writes to the OpenCode database.
- **Claude Code**: one JSONL session log per file (`sessionId`, `cwd`, `message`).
  The scan covers `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`, recursively.
- **OpenAI Codex**: rollout JSONL (`session_meta`, `response_item`, `turn_context`).
  The scan covers `$CODEX_HOME/sessions` or `~/.codex/sessions`, recursively.
- **Other harnesses**: ACP does not define a universal history store; the app does
  not guess the private database format of a custom adapter.

**There is no limit on the number of scanned files or imported sessions**,
including in an OpenCode database. There is also no aggregate 64 MiB limit. Individual files are still
validated: at most 16 MiB and 30,000 entries/session. An invalid or too
large file is reported, but does not stop reading the remaining files.

Scanning and saving run in the background. The status shows the number of found files,
then read files/sessions and saved sessions (`done / all`).
The importer stages full histories in a private temporary directory on disk, not
all at once in RAM. Warnings appear on the corresponding harness card.
The temporary directory is cleaned up after import completion or replacement.
Free space is needed for
history copies and the SQLite transaction.

## Matching and safety

- Automatic assignment only to existing local projects by
  canonical path. For a directory inside a project, the deepest root wins.
- Project names are not used for matching. Missing directories, duplicate roots
  or paths from another computer are not guessed.
- Sessions without a match are saved in a durable pending-import table, not skipped.
  Adding a matching local project attaches them automatically, without reimporting.
  Pending copies survive app restarts and are included in the harness import count.
- Import saves the source directory as the session workspace. Reimport corrects
  the project automatically; without a match it keeps an existing assignment.
  Assignment changes do not overwrite content or local continuations.
  Before moving an open session, close its agent and select another session;
  the app does not move running or parked runtimes between projects.
- A stable harness identity and source ID protect against re-import.
  New sessions are streamed in a single transaction. A save or
  temporary-history read failure rolls back new and pending sessions; use Reimport to retry.
  A normal close/Quit is blocked until saving finishes.
  Opening the Dashboard triggers
  a one-time metric backfill in existing OpenCode copies; a manual scan
  additionally updates existing copies. Both paths add
  missing tokens/diffs and normalize old tool entries; reimport also corrects
  the project and workspace. They do not replace conversation text or local
  continuations.
- Import is a **copy of history**, not a resume of the source session. We do not attach the
  original ACP session ID. No prompt, tool call or script is
  executed during import. Tool records and unknown types are preserved as
  text; attachments are not automatically downloaded or opened from the source.
- This is not a billing usage scanner: import does not reconstruct prices or work time.
  It carries only token counters and changed lines saved by the source.

The file scan covers standard harness folders, with no limit on the number of files or
directory depth; it skips hidden directories and symbolic links. OpenCode
reads all top-level sessions; the existing safeguards for a single
history (30,000 messages, 60,000 parts, truncating large tool outputs)
remain unchanged. Metric backfill also processes histories one at a time.
There is no background sync or session import from relay/SSH; source files and the
OpenCode database remain unchanged.

## Contract and regressions

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| The 512/256/64 MiB limits were local preview policy, not a harness limit | `session_import.rs`: `collect_session_files`, `discover_session_files`, `read_files`; `adapters.rs::parse`; `opencode_db.rs`: session queries | current source | High | None | Tests for 600 files >64 MiB and both database schemas |
| The source is read-only; the target identity is harness + source ID | `opencode_db.rs::visit`, `session_import.rs::imported_id`, `adapters.rs::match_project`, `Store::import_session_stream` | Working tree | High | Real user histories were not tested | Isolated JSON/JSONL/SQLite fixtures |
| A stream failure rolls back new sessions; a retry does not overwrite conversations | `storage/src/lib.rs::streamed_import_reports_duplicates_and_rolls_back_on_a_source_read_failure`, `session_import.rs::staged_read_failure_rolls_back_inserts_and_leaves_preview_available_for_retry` | Working tree | High | None | `cargo test -p dunlin-storage`; app tests |
| Project history uses the local `SessionSummary.project_id`, not the harness project ID or repo name | `session_list.rs::session_scope`, `views/sidebar.rs::sessions_section`, `storage/src/lib.rs::sessions` | Working tree | High | None | Import smoke with two projects |
| Old copies can be reassigned; the change affects only project and workspace | `session_import.rs::finalize_import`, `assign_imported_location`, `Store::update_session_location` | Working tree | High | Old imports did not save cwd; without a rescan it cannot be recovered | Reimport the source harness |

Data path: harness selection → standard folder/database → validated
`Candidate.source_id` → private temporary history → automatic
directory matching (or durable pending copy) → `imported_id(harness, source_id)` →
transaction in the Dunlin database. Every step has an implementation in the above files;
none requires starting an agent, accessing credentials or writing to the source.

Link confirmed in code: `Candidate.directory` →
`match_project(canonical path)` → local `Project.id` →
`SessionSummary.project_id` → `session_scope` → history of the selected project.
Repo names and project identifiers from foreign harnesses are not substitutes for this
link. Full history has no 8-hour limit; the limit applies only to the recent-sessions
shortcut under a repository in the project tree. Private user
histories are not modified by automated verification. Unit fixtures cover mixed V1/V2
databases, empty migrated shells, pre-SQLite storage and durable pending copies.
The native 700-session smoke checks visibility, adding a project after import,
reimport without duplicates, content preservation, fallback without cwd and blocking
a move of an open session. Next step: re-import
in the app by the user. Owner: maintainer/user.

Confirmed: no session count limit, progress and streamed saving. Rejected:
raising the limit to another arbitrary number or removing file validation.
Next owner: app maintainer — unit tests, native large-import smoke and manual UX
assessment on real histories (without automatically importing user data). Isolated GUI test:
`dunlin --session-import-smoke <new absolute dunlin-session-import-smoke-… directory>`
creates 700 synthetic sessions and one invalid journal, renders the Agents settings,
checks progress, background saving, retry without duplicates, cleanup and the Quit block.
Success writes `PASS` to a file in the specified directory.
