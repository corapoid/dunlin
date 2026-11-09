# Collection import and group-agent contract

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| Selection owns the collection relationship; repository discovery stops at Git roots | `crates/workspace/src/repositories.rs`: `discover`, `LocalProject.collection`, repository tests | Working tree | High | None | Repository tests |
| Canonical parent path identifies an imported group across rescans; its label/settings remain editable | `apps/desktop/src/local_projects.rs`: `collection_group`, `import_local_folders`; `crates/storage/src/lib.rs`: `upsert_project_group`, `upsert_project` | Working tree; existing storage schema | High | None | Import smoke and persisted group/project assertions |
| A group agent already freezes trusted repository roots and scopes extra directories | `apps/desktop/src/project_groups.rs`: `GroupContext::build`, `validate`, `additional_directories`, `start_group_session` | Working tree | High | None | Group-context unit tests and native smoke |
| The current transport sends client context as user-message content, not a system-role override | `apps/desktop/src/harness.rs`: `client_prompt_text`, `dispatch_pending_prompt`; `crates/agent/src/lib.rs`: `HarnessCommand::Prompt`; installed `schema/v1/agent.rs`: `PromptRequest` fields | agent-client-protocol 3.5.0 / schema 1.12.0, locked in Cargo.lock | High | No portable system-prompt field in this request; agent-specific extensions were not required | Keep the existing portable context prefix |

Proven graph: selected folder → bounded discovery → canonical repository plus
selected collection → stable group ID and project membership → explicitly started
group session → frozen `GroupContext` keyed by session ID → prompt prefix plus
additional directories. Existing session snapshots are not widened on reimport.

Compatibility: no database or settings migration; existing project identities and
manual memberships survive. Repeated import does not create duplicate groups.
Only explicitly selected collection parents become groups; same-named folders do
not merge. A collection is not itself a repository or an extra access grant.
The existing scan limits and 1–32-root group-session bound remain unchanged.
Partial discovery/import errors remain visible; failure to persist a required
group skips that group's candidates instead of creating orphan memberships.

Rejected interpretations: importing a container does not make all its files part
of the agent scope; a system-role override is not equivalent to client context.
No unresolved identity/ownership edge remains for this local import path. Tests
cover nested/overlapping selections, monorepos, repeated import, manual membership
preservation, persistence and group prompt construction. Next owner: maintainer
for a real-harness acceptance check; automated fixtures send no provider request.
