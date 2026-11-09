# Import projects

Import adds configured projects; it is not synchronization or implicit permission
to run an agent. Local collections and hosted repository search have separate
previews and trust boundaries.

## Local folders and collections

Choose **Add from local drive** from the sidebar **+**, project context menu or
onboarding. **⌘O** opens the system multi-folder picker. Cancelling does nothing;
there is no second in-app folder explorer to navigate first.

Select a repository, ordinary folder or collection directory. Discovery is
bounded, canonicalizes/deduplicates roots and stops at Git/worktree boundaries.
Hidden/generated directories and implicit symlinks are skipped; it does not fetch
or execute repository scripts. A collection becomes a logical group, not a parent
filesystem grant. Reimport adds newly discovered repositories while preserving
manual group choices, names, trust, pins and settings.

For a multi-repository collection, explicitly opt into root tracking to discover
new Git roots every 30 seconds while Dunlin runs. Tracking survives restart;
stop it in **Settings → Workspace → Project discovery**. It only adds projects,
never deletes missing entries or pulls updates. New automatic discoveries are
untrusted and do not steal active project selection.

Exact discovery/group-agent limits: [collection import contract](../reference/collection-import.md).

## GitHub and GitLab repositories

1. Choose **Add from remote repo**. Select GitHub or GitLab; this means downloading
   a hosted Git repository to the Mac, **not** creating a relay project.
2. Use a supported direct repository URL or search. GitHub accepts its native
   repository-search qualifiers (for example `org:example archived:false language:Rust`);
   GitLab uses its own search contract, not GitHub qualifiers.
3. Search and review the frozen repository list/count. Search is scoped to the
   installed CLI account's access, not every private repository in an organization.
4. Choose the destination with **Browse…** (system picker), review and explicitly
   confirm download. Existing directories are not overwritten.
5. Review each imported project and trust it separately before starting an agent.

Hosted actions use installed `gh`/`glab` credentials, not extracted tokens. Sign
in outside Dunlin; **Settings → Workspace** can refresh CLI/account status. Current
hosting support is `github.com` and `gitlab.com`, not arbitrary self-hosted servers.
PR/Actions integration remains removed; imports do not restore it.

Default immediate clones go beneath the application's `repositories/` directory
or a chosen destination. This differs from the host/namespace layout for
[deferred repositories imported from a backup](../administration/settings-project-backup.md).
Failed/partial clones are retained for inspection, never silently deleted/reused.

## Search and cancellation boundaries

GitHub search validates actual pagination, fixed endpoints, names/duplicates,
incomplete results and changing totals. The actual result set is capped at 1,000;
the importer does not claim an atomic server snapshot. Narrow an incomplete or
oversized query rather than bypassing the cap. A frozen list records names, not
immutable hosted repository IDs.

GitHub/GitLab preview work shares a 30-second deadline across pages. Cancel
invalidates results and signals the CLI process group; a new search need not
wait for a stale one. Started clones have separate timeout/partial-directory
semantics and are not deleted by cancelling a later preview. A late OS selection
cannot modify an obsolete import form.

## Remote server projects

To work on code that stays on another machine, pair/manage that
[relay](../administration/relay-setup.md) and clone/copy there. Do not interpret
the hosted-provider import form as server authorization. One-time project copy
is not ongoing sync.

## Verification

```sh
cargo test -p dunlin-workspace github:: -- --test-threads=2
cargo test -p dunlin project_import::tests
cargo test -p dunlin local_projects::tests
```

The discovered [native suite](../quality/test-quality.md) includes
`--project-import-smoke` with a fresh `dunlin-project-import-smoke-…` directory.
It uses fake CLIs/disposable repositories and supplies OS picker results at the
selection handler. It does not automate Finder's own dialog, use live credentials
or prove real-account clone acceptance. [Original evidence](../archive/project-import.md)
is historical, not a new run.

Sources: [import UI](../../apps/desktop/src/project_import.rs),
[local discovery](../../apps/desktop/src/local_projects.rs),
[repository discovery](../../crates/workspace/src/repositories.rs),
[hosting backends](../../crates/workspace/src).
