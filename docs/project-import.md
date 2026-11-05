# Project import contract

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| Local collections use canonical identities, bounded discovery and stop at repository roots | `crates/workspace/src/repositories.rs`, `apps/desktop/src/local_projects.rs`, `docs/collection-import.md` | Existing working tree | High | None | Discovery/import regression tests |
| The app owns persistent preferences and its internal clone directory | `apps/desktop/src/app.rs`: `StoredSettings`, `stored_settings`; `app/helpers.rs`: `data_dir` | Existing working tree | High | None | Settings round-trip test |
| GitHub repository search accepts native qualifiers and returns `total_count`, `incomplete_results`, `items[].full_name`; at most 1000 results are retrievable | https://docs.github.com/en/rest/search/search#search-repositories | REST documentation checked 2026-11-05 | High | No atomic snapshot across pages | Refuse incomplete, changing or oversized result sets; mock pagination |
| `gh repo clone <repository> [directory] -- <gitflags>` downloads via installed CLI credentials | `gh repo clone --help`; https://cli.github.com/manual/gh_repo_clone | Installed gh 2.102.0 | High | Real user/network acceptance intentionally separate | Mock CLI arguments and collision tests |
| Existing GitHub integration bounds subprocess lifetime/output and suppresses credential-bearing stderr | `crates/workspace/src/github.rs`, `github/process.rs`, `github/tests.rs` | Existing working tree | High | None | Reuse runner and mock executable |
| The source chooser, explorer, preview invalidation and tracked-root persistence share the existing project import path | `apps/desktop/src/project_import.rs`; `local_projects.rs`: `finish_project_import`; `app.rs`: `poll_project_folders`, `StoredSettings`; `project_import/smoke.rs` | This working tree | High | Real-account/network acceptance intentionally not automated | Isolated native smoke below |

Proven resource graph: local selection → bounded discovery → canonical repository
and optional collection → existing group/project persistence. Optional explicit
watch consent → saved canonical root → periodic bounded scan → only new Git roots.
GitHub URL/native query → `gh api repos/{owner}/{repo}` or paginated
`gh api search/repositories -f q=...` → frozen full-name list → explicit confirmation
and destination → `gh repo clone` → local project persistence.

Implementation scope: in-app folder explorer and source chooser; local preview and
opt-in polling (30 seconds while Dunlin runs, including after restart); settings
to stop watching and refresh CLI/auth status; CLI-backed preview and cloning into
`data_dir()/repositories/<owner>/<repo>` or a user-chosen directory. Existing local
imports keep identities/settings/group memberships. Automatic and GitHub imports
are untrusted and never activate a workspace automatically. No database migration,
new dependency, shell interpolation, token extraction, deletion or overwrite of
existing destinations. Factory reset retains cloned files, as it retains all
repository files.

Rejected interpretations: a filter is not a shell command, glob or `gh` flags;
it is GitHub's repository-search query. A collection is not an agent scope.
Tracking only adds projects; it never deletes missing projects or pulls updates.
Search results reflect the active gh account's permissions, not all repositories
in an organization. The preview is a frozen list, not a fresh search at clone time.

Required verification: search parsing/pagination/incomplete and oversized responses,
CLI installation/auth failures, safe destination collisions, partial clone failure,
ordinary folder vs repository vs collection, settings migration, duplicate and
untrusted automatic imports, build and native modal/input smoke checks. Owner of
the implementation/tests: current change; real-account cloning acceptance: maintainer.

## Acceptance

```sh
cargo test -p dunlin-workspace github:: -- --test-threads=2
cargo test -p dunlin project_import::tests
cargo test -p dunlin local_projects::tests
cargo build -p dunlin
target/debug/dunlin --project-import-smoke "$TMPDIR/dunlin-project-import-smoke-$(uuidgen)"
```

The native smoke creates its own database and repositories, places a fake `gh`
first in PATH, renders the sheet at the minimum supported window size, and checks
single-repository versus collection previews, persistence, new-root tracking,
existing membership preservation, trust, query edits, destination browsing and
cloning. It never uses a real GitHub account or downloads a real repository.
The CLI command environment also supports Homebrew paths for Finder launches.
Actual clones have a ten-minute per-repository timeout; failed or timed-out clone
directories are retained, not silently deleted or reused. A preview captures
repository names, not immutable GitHub repository IDs or a transactional snapshot.

Verification on 2026-11-05: `cargo build -p dunlin`, the isolated native smoke and
all 146 `dunlin-workspace` tests passed (`--test-threads=2`; the unconstrained run
hit Git subprocess timeouts). The three new desktop import unit tests passed
before concurrent chat-control changes introduced a separate test-compilation
blocker: `apps/desktop/src/views/chat_controls.rs:283` references nonexistent
`dunlin_agent::SessionChoice`. The subsequent desktop unit-test rerun is blocked
there, not in the import module. Native smoke still exercises the current import
implementation. No real-account clone, installer bundle or manual visual review
was performed. Next owner: maintainer for the unrelated test fixture and real-gh
acceptance.
