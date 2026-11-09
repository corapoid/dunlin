# Project import contract

> **Archive — implementation investigations from 2026-11-05/06.** Tool versions,
> compile blockers and verification counts below belong to those runs, not the
> current checkout. Use [project import](../user-guide/project-import.md) for
> maintained instructions and current boundaries.

## Add project source menu — 2026-11-06

The sidebar `+`, project context menu and onboarding action now offer **Add from
local drive** and **Add from remote repo**. Local and Cmd+O go straight to the
system multi-folder picker, then the existing bounded local import. Cancelling
does nothing. Remote opens the existing provider/search/destination form with
GitHub selected, keeping GitLab available and clone confirmation unchanged.

The native smoke checks source choices before a form opens, chooses remote using
the keyboard, and verifies the default provider. Local discovery and native-picker
result handling remain fixture-tested; Finder's own picker is not automated.

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| Local collections use canonical identities, bounded discovery and stop at repository roots | `crates/workspace/src/repositories.rs`, `apps/desktop/src/local_projects.rs`, `docs/reference/collection-import.md` | Existing working tree | High | None | Discovery/import regression tests |
| The app owns persistent preferences and its internal clone directory | `apps/desktop/src/app.rs`: `StoredSettings`, `stored_settings`; `app/helpers.rs`: `data_dir` | Existing working tree | High | None | Settings round-trip test |
| GitHub repository search accepts native qualifiers and returns `total_count`, `incomplete_results`, `items[].full_name`; at most 1000 results are retrievable | https://docs.github.com/en/rest/search/search#search-repositories | REST documentation checked 2026-11-05 | High | No atomic snapshot across pages | Refuse incomplete, changing or oversized result sets; mock pagination |
| `gh repo clone <repository> [directory] -- <gitflags>` downloads via installed CLI credentials | `gh repo clone --help`; https://cli.github.com/manual/gh_repo_clone | Installed gh 2.102.0 | High | Real user/network acceptance intentionally separate | Mock CLI arguments and collision tests |
| Existing GitHub integration bounds subprocess lifetime/output and suppresses credential-bearing stderr | `crates/workspace/src/github.rs`, `github/process.rs`, `github/tests.rs` | Existing working tree | High | None | Reuse runner and mock executable |
| The source chooser, native folder picker, preview invalidation and tracked-root persistence share the existing project import path | `apps/desktop/src/project_import.rs`; `local_projects.rs`: `finish_project_import`; `app.rs`: `poll_project_folders`, `StoredSettings`; `project_import/smoke.rs` | This working tree | High | Real-account/network acceptance intentionally not automated | Isolated native smoke below |

Proven resource graph: local selection → bounded discovery → canonical repository
and optional collection → existing group/project persistence. Optional explicit
watch consent → saved canonical root → periodic bounded scan → only new Git roots.
GitHub URL/native query → `gh api repos/{owner}/{repo}` or paginated
`gh api search/repositories -f q=...` → frozen full-name list → explicit confirmation
and destination → `gh repo clone` → local project persistence.

Implementation scope: system folder picker and in-app source chooser; local preview and
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

## Search pagination and import UI — 2026-11-06

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| `gh search repos` follows `Link: ...; rel="next"`; `total_count` is displayed but does not determine which pages exist | `https://github.com/cli/cli/blob/v2.102.0/pkg/search/searcher.go`: `Repositories`, `search`, `nextPage`; local `gh --version` and `gh search repos --help` | gh 2.102.0 | High | None | CLI mock pagination regressions |
| GET fields preserve the full native query, without shell splitting or qualifier rewriting | `https://github.com/cli/cli/blob/v2.102.0/pkg/cmd/api/http.go`: `httpRequest`, `addQuery`; `github/imports.rs`: `request`, `search_page` | gh 2.102.0 / current working tree | High | None | Assert the exact `q=` argument, including spaces and punctuation |
| A final page with fewer than 100 repositories is not rejected solely because its reported total exceeds 1000 | `github/imports.rs`: `preview`; `github/tests.rs`: `search_uses_the_actual_final_page_even_when_the_reported_total_is_larger` | Current working tree | High for behavior; not a reproduction of the live user query | Exact user query and live headers were not supplied | Compare the same native query in gh and Dunlin if the issue persists |

Proven read path: query field → `Source::parse` → `gh api --method GET
--include search/repositories -f q=<unchanged query>` → bounded response and Link
headers → validated repository names → frozen preview → explicit clone confirmation.
The active gh account still owns authentication and visibility; no token is read,
copied or logged. No schema or persisted-data change.

The importer caps the **actual** paginated result set at 1000 and refuses a next
page after that limit, incomplete responses, changing totals, malformed headers,
invalid names and duplicates. It never follows a response-supplied URL: page
numbers are requested against the fixed GitHub endpoint. Results and errors for
obsolete queries are discarded.

The UI separates find / destination / review, places a primary **Search repositories**
button next to the query, explains Enter and retains explicit download confirmation.
Mock CLI and native sheet tests cover inflated totals, the visible search action
and stale queries. Real-account verification of the exact reported filter remains
with the maintainer; its text is the missing reproduction input, not a reason to
raise or remove the safety cap.

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
existing membership preservation, trust, query edits, destination selection and
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

## Native selection and cancellation — 2026-11-06

The custom folder explorer and folder-creation form have been removed. Both local
root and download destination buttons call GPUI's system path prompt with the
existing native-overlay tracking. File attachments already use this same system
picker. Dismissing the picker leaves the current preview/destination untouched;
late selections from an obsolete import sheet are ignored. Native folder creation
belongs to the OS picker, not a second in-app explorer.

GitHub and GitLab previews share a 30-second deadline across all pages. Cancel
remains enabled during discovery/search, invalidates its request and signals the
CLI process group to stop; a new search need not wait for the old one. Query
validation, full pagination, result limits and the frozen clone list are retained.
Started clone operations are not cancelled or deleted: their existing finish and
partial-directory safety remains unchanged.

Verified: CLI process cancellation regression, GitHub/GitLab mock tests, desktop
unit suite and native import smoke (including an actual Cancel click during a
sleeping fake CLI, then a successful new search). The smoke supplies OS picker
results at the selection handler; it does not automate Finder's own dialog or use
real hosting credentials. Live query latency still depends on the provider/network;
the deadline bounds waiting rather than claiming a faster remote API.
