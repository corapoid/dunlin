# Session import

**Settings → Harnesses → harness card → Import sessions…** automatically
scans the standard session folders of the selected harness. A preview of titles,
directories, entry counts and assignments appears first. Nothing is saved
before confirmation. The separate **Import a portable file…** link remains for the
explicit portable format; it does not open as the default import action.

## Supported sources

- **OpenCode**: Dunlin opens the local `opencode.db` in
  read-only mode only and reads top-level sessions from the current schema
  (`session_v2` + `session_message`), with a fallback to the older `session`/`message`/`part`
  tables. It imports text, reasoning, tool history,
  saved token counters and file diff statistics; it does not read
  credentials, blobs or snapshots. The V2 `experimental.session.export` JSON export is still
  supported through **Import a portable file…**:
  `{ "data": { "info": {…}, "messages": […] } }`, or just `{info,messages}`.
  Exports with `{info,parts}` messages are also accepted.
  The public endpoint is `GET /api/experimental/session/{sessionID}/export`;
  the export can be obtained through the documented OpenCode client/API. Dunlin never
  reads credentials or writes to the OpenCode database.
- **Claude Code**: one JSONL session log per file (`sessionId`, `cwd`, `message`).
  The scan covers `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`, recursively.
- **OpenAI Codex**: rollout JSONL (`session_meta`, `response_item`, `turn_context`).
  The scan covers `$CODEX_HOME/sessions` or `~/.codex/sessions`, recursively.
- **Custom ACP** (also Claude/Codex): the explicit portable format below.
  ACP does not define a universal history export; the app does not guess
  the private database format of a custom adapter.

```json
{
  "format": "dunlin.session.v1",
  "id": "stable-source-session-id",
  "title": "API refactoring",
  "directory": "/Users/me/code/project",
  "model": "model-name",
  "updated_at": "2026-10-30T12:00:00Z",
  "messages": [
    {"role": "user", "content": "Check the API", "timestamp": "2026-10-30T11:59:00Z"},
    {"role": "assistant", "content": "Analysis result", "timestamp": "2026-10-30T12:00:00Z"}
  ]
}
```

The JSON file may be an array of exports of the same type. **There is no limit on the number
of scanned files or imported sessions**, including in a single JSON array or
OpenCode database. There is also no aggregate 64 MiB limit. Individual files are still
validated: at most 16 MiB and 30,000 entries/session. An invalid or too
large file is reported, but does not stop reading the remaining files.

Scanning and saving run in the background. The status shows the number of found files,
then read files/sessions and saved sessions (`done / all`).
The preview keeps full histories in a private temporary directory on disk, not
all at once in RAM. The list renders only visible rows; warnings have
a separate scrollable area. The temporary directory is cleaned up after cancellation,
preview replacement or import completion. Free space is needed for
history copies and the SQLite transaction.

## Matching and safety

- Automatic assignment only to existing local projects by
  canonical path. For a directory inside a project, the deepest root wins.
- Project names are not used for matching. A missing directory, moved project,
  duplicate roots or a path from another computer require manual assignment.
- Sessions without an assignment are skipped. The automatic assignment can be changed
  and selected sessions unchecked before confirmation.
- Import saves the source directory as the session workspace. A rescan lets you
  correct the project also for **Update · project name**; if the path gives no
  match, the preview keeps the existing project. Confirmation changes the
  assignment in history without overwriting content and local continuations.
  Before moving an open session, close its agent and select another session;
  the app does not move running or parked runtimes between projects.
- A stable harness identity and source ID protect against re-import.
  New sessions are streamed in a single transaction. A save or
  temporary-history read failure rolls back new sessions and leaves the preview for retry.
  A normal close/Quit is blocked until saving finishes.
  Opening the Dashboard triggers
  a one-time metric backfill in existing OpenCode copies; a manual scan
  additionally marks them as **Update · project name**. Both paths add
  missing tokens/diffs and normalize old tool entries; manual import also lets you
  correct the project and workspace. They do not replace conversation text or local
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
| The 512/256/64 MiB limits were local preview policy, not a harness limit | `session_import.rs`: `collect_session_files`, `discover_session_files`, `read_files`; `adapters.rs::parse`; `opencode_db.rs`: session queries | Dunlin beta.8 / working tree | High | None | Tests for 600 files >64 MiB and both database schemas |
| The source is read-only; the target identity is harness + source ID | `opencode_db.rs::visit`, `session_import.rs::imported_id`, `adapters.rs::match_project`, `Store::import_session_stream` | Working tree | High | Real user histories were not tested | Isolated JSON/JSONL/SQLite fixtures |
| A stream failure rolls back new sessions; a retry does not overwrite conversations | `storage/src/lib.rs::streamed_import_reports_duplicates_and_rolls_back_on_a_source_read_failure`, `session_import.rs::staged_read_failure_rolls_back_inserts_and_leaves_preview_available_for_retry` | Working tree | High | None | `cargo test -p dunlin-storage`; app tests |
| Project history uses the local `SessionSummary.project_id`, not the harness project ID or repo name | `session_list.rs::session_scope`, `views/sidebar.rs::sessions_section`, `storage/src/lib.rs::sessions` | Working tree | High | None | Import smoke with two projects |
| Old copies can be reassigned; the change affects only project and workspace | `session_import.rs::finalize_import`, `assign_imported_location`, `Store::update_session_location` | Working tree | High | Old imports did not save cwd; without a rescan it cannot be recovered | Rescan the source harness and confirm the preview |

Data path: harness selection → standard folder/database or manually selected
files → validated `Candidate.source_id` → private temporary history → manual
confirmation/assignment of a local project → `imported_id(harness, source_id)` →
transaction in the Dunlin database. Every step has an implementation in the above files;
none requires starting an agent, accessing credentials or writing to the source.

Link confirmed in code: `Candidate.directory` →
`match_project(canonical path)` → local `Project.id` →
`SessionSummary.project_id` → `session_scope` → history of the selected project.
Repo names and project identifiers from foreign harnesses are not substitutes for this
link. Full history has no 8-hour limit; the limit applies only to the recent-sessions
shortcut under a repository in the project tree. Private user
histories were not verified. Fix verification: 17 import tests, 34 storage tests
and native smoke with 700 sessions in two projects passed. The smoke checks visibility
in project history, assignment correction through re-import, content preservation,
fallback without cwd and blocking a move of an open session. Next step: re-import
in the app by the user. Owner: maintainer/user.

Confirmed: no session count limit, progress and streamed saving. Rejected:
raising the limit to another arbitrary number or removing file validation.
Next owner: app maintainer — unit tests, native large-import smoke and manual UX
assessment on real histories (without automatically importing user data). Isolated GUI test:
`dunlin --session-import-smoke <new absolute dunlin-session-import-smoke-… directory>`
creates 700 synthetic sessions and one invalid journal, renders the real preview,
checks progress, background saving, retry without duplicates, cleanup and the Quit block.
Success writes `PASS` to a file in the specified directory.
