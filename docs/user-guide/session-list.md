# Session list scope contract

Status: confirmed from local source, based on `b2c4d02` plus the working tree.
Question: which conversations are general, which belong to the selected project,
and how can opening/deleting a general conversation retain its original owner?

## Evidence

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
|---|---|---|---|---|---|
| Every session has an owning project, including standalone chats | `crates/domain/src/lib.rs`: `ProjectKind`, `SessionSummary.project_id`; `apps/desktop/src/app.rs`: `create_chat_space` | current source | High | None | None |
| A group conversation is stored under an anchor repository, not a separate group project | `apps/desktop/src/project_groups.rs`: `start_group_session`; `apps/desktop/src/app.rs`: `new_session`, `StoredSettings.group_contexts` | current source | High | None | None |
| Group identity comes from the session's frozen context, not current project membership | `apps/desktop/src/project_groups.rs`: `GroupContext`, snapshot test; `apps/desktop/src/session_list.rs`: `session_scope` | current source | High | None | None |
| The old list and draft scan only used the current project's sessions | `apps/desktop/src/views/sidebar.rs`: `sessions_body`; `apps/desktop/src/session_list.rs`: previous `refresh_session_list` | b2c4d02 | High | None | None |
| Opening retains existing navigation, runtime parking, dirty-editor guards and remote loading | `apps/desktop/src/session_list.rs`: `open_session_id`; `apps/desktop/src/app.rs`: `select_project_internal`, `import_remote_sessions`; `apps/desktop/src/harness.rs`: `select_session` | current source | High | Live relay verification | Relay acceptance test |

```text
all_sessions --project_id--> projects                         [proven]
session.id --group_contexts lookup--> frozen group context    [proven]
chat-space owner OR frozen context --> General sessions      [implemented]
otherwise selected repository owner --> Project sessions     [implemented]
session.id --owner selection--> project --id lookup--> history [proven]
```

## Implementation and compatibility

- General conversations are visible across known projects; ordinary sessions from
  unrelated repositories are not. Group sessions appear once, in General.
- Each scope heading independently collapses its list, with the choices remembered
  between launches. Empty scopes do not reserve half the sidebar.
- Opening history and loading connection/model metadata preserve its timestamp
  and position, including the temporary Connecting state in the project view.
  A newly submitted user message or an approved pending operation updates activity
  and moves it up. Merely opening, loading or declining a request does not.
- A repository merely belonging to a group does not make all its sessions general.
  Standalone chats with explicit repository context are still standalone chats.
- Reads use the local `all_sessions` snapshot and stored group contexts; no schema
  migration, data move, inferred path mapping or external SDK is needed.
- Draft detection scans both scopes, reads transcripts off-thread through the
  existing read-only store, and preserves legacy title/usage evidence on failures.
  Each section has an empty state, count and newest-first ordering; blank drafts
  remain hidden and persisted. Existing cache behavior is unchanged.
- Remote sessions retain their origin filter and owning relay host credentials.
  Opening waits for that owner's remote session refresh. Takeover remains available
  for loaded sessions in the selected project; open another workspace first.
- Open/delete actions and session context menus identify sessions by stable IDs.
  Deletion keeps confirmation, storage error handling and remote deletion behavior.
  Deleting another workspace's session leaves current selection/transcript alone.
- Minimal view renders each session in one line (status dot, title and delete
  action), omitting date headers and the metadata row. Full metadata stays in the
  row tooltip; scope, ordering, opening and deletion rules are unchanged.

## Verification and handoff

Unit tests: `cargo test -p dunlin session_list::tests` covers scope classification,
group membership versus frozen context, switching projects, draft visibility and
legacy history. Native acceptance: `--workspace-ui-smoke <fresh-absolute-directory>`
requires a directory basename starting with `dunlin-workspace-ui-smoke-` and
also checks both rendered sections, persisted first-message discovery outside the
selected project, navigation, hidden drafts, unrelated history, cancelled deletion
and confirmed cross-project deletion without changing the active conversation.
Fixtures use isolated data, no agents or credentials.

Verified on 2026-11-05: all 360 desktop unit tests passed; the native workspace UI
smoke passed, including dirty-editor navigation protection and active-transcript
preservation after cross-project deletion.

No unresolved local ownership edges. Live relay behavior is not covered by the
isolated native fixture. Next owner: maintainer runs relay acceptance before release;
no backend change or new grouping/assignment feature is required by this fix.
