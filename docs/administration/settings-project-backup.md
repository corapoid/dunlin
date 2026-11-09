# Export and import of settings and project structure

**Settings → Data & Backup → Export and import**.

1. Select **Settings**, **Project structure**, or both scopes.
2. JSON export shows a preview before choosing the target file.
3. Export to a repository requires an explicit GitHub.com/GitLab.com URL. After preview
   and confirmation it creates/updates `dunlin-backup.json` on the default branch
   of an existing, initialized repository. Saving creates a commit; it does not run
   a local `git push`. Updates use the file SHA / `last_commit_id`, without
   automatic retries on conflict or an uncertain result.
4. JSON import or import from a repository reads the same format and shows a preview.
   You can choose the import scope and enter a download directory or use **Browse…**.
5. **Import selected data** adds missing groups and projects. It does not clone code.
   New repositories have a download icon and a **Not downloaded** status in the list.
6. Clicking the icon downloads a single repo. The icon next to a group downloads all
   not-downloaded repositories of that group and its subgroups **sequentially**, not in parallel.
   Each repo then requires separate trust before an agent can be started.

## Scope and safety

- The structure includes names, groups and subgroups, colors, pins and canonical
  HTTPS repository addresses from GitHub.com/GitLab.com (including GitLab subgroups).
- Local repositories must have a supported, unambiguous Git remote. `origin`
  takes precedence. A repo already imported but not downloaded can also be exported.
- **Projects on remote hosts/relay are not exported**, even if their code
  comes from GitHub/GitLab. The export preview contains an explicit list of excluded
  projects with the reason. Repos without a supported remote and subsequent
  entries with the same URL are also skipped. The names of excluded projects alone do not go into the file.
- We do not export local paths or identifiers containing paths.
  Import chooses a new directory; checkouts go to
  `<selected directory>/<github.com or gitlab.com>/<namespace>/<repo>`.
- Settings include portable appearance, editor, chat, startup,
  notification, panel and budget preferences. They do not include history, relay accounts, SSH, permissions,
  trust, run commands, secrets, environment variables,
  extension configuration, skills, project/group runtime overrides or custom icons.
- Import does not delete existing projects or sessions. Repositories are matched by
  canonical URL case-insensitively; an existing entry remains
  unchanged. Groups are matched by name and full hierarchy. Re-import
  does not create the same groups/repositories a second time.
- Directory import and selected preferences are saved in a single SQLite transaction.
  An invalid format, version, URL, group cycle or save error does not leave a partial
  import. Unsupported machine fields are rejected, not ignored.
- Cloning does not overwrite or delete existing directories; it rejects symlinks
  in created host/namespace subdirectories. Partial files after an error remain
  on disk for manual inspection. Not-downloaded/failed entries keep the icon.
- Authorization remains in `gh`/`glab`. Log in outside Dunlin through
  `gh auth login --hostname github.com` / `glab auth login --hostname gitlab.com`.
  Saving requires access to repository contents and consent of branch protection rules.
- A **private repository** is recommended: project names and preferences alone
  may be confidential, even though secrets are skipped. Dunlin does not create a repo or change
  its visibility. GitHub Enterprise / self-managed GitLab are not supported.
- Format v1 has a limit of 1 MiB, 1000 groups and 2000 repositories. The file limit matches
  base64 handling in the GitHub Contents API. It is a configuration copy, not a code backup.

## Contract and historical evidence

The source audit, tool availability and counts below belong to the original
implementation run; they are not fresh results for this checkout. The export/import
safety contract above remains distinct from live-account/manual acceptance.

Initial state: `38c0853` + existing local changes. Existing contracts used:

| Claim | Evidence and location | Version | Confidence | Missing link | Next verification |
|---|---|---|---|---|---|
| A remote project means a relay project, not a Git remote | `crates/domain/src/lib.rs`: `RemoteProject`, `Project::is_remote` | current source | high | none | export exclusion test |
| Supported repo identities are github.com/gitlab.com URLs | `crates/workspace/src/git/hosting.rs`: `RepositoryLink::parse`, `detect` | as above | high | none | test with real local Git remotes |
| Safe clones through the CLI exist | `github/imports.rs::clone_with`, `gitlab.rs::clone_repository` | as above | high | none | test for overwrite and symlink refusal |
| Preferences belong to the local store | `apps/desktop/src/app.rs::StoredSettings`, `stored_settings`; `crates/storage/src/lib.rs` | as above | high | none | transaction and database reopen test |
| The repository stores a base64 file with revision control | [GitHub Contents API](https://docs.github.com/en/rest/repos/contents#create-or-update-file-contents), [GitLab Repository Files API](https://docs.gitlab.com/api/repository_files/), `catalog_backup.rs` | REST, file format v1 | high | no save performed on a real account | integration test on a private test repo |
| The CLI accepts an explicit host, HTTP status and JSON through stdin | local `gh api --help`, [glab api](https://docs.gitlab.com/cli/api/) | installed gh; glab not available locally | high for the contract, test transport | no test with real glab | smoke with installed glab |

Flow (all links confirmed by code/contract):

```text
Project.remote --is_remote()--> relay project exclusion
local Project.path --Git remote/detect--> RepositoryLink --allowlist--> export URL
ProjectGroup.id --synthetic ID map--> portable hierarchy
copy repo URL --gh/glab API(default branch)--> dunlin-backup.json
JSON --validation/prepare--> new local ID + PendingClone(destination)
confirmation --Store.import_catalog(transaction)--> durable catalog
download icon --next operation clone(url,destination)--> local checkout without trust
```

Rejected interpretations: a Git remote is not a relay project; the export file is not a
SQLite dump; cloning does not mean trust; a repo name is not a substitute for a URL.
There are no unresolved data relationships. Verification on real
accounts and manual smoke of the native UI remain open. Owner of the next step: maintainer/QA,
on a private test repo, after installing and authorizing both CLIs.

Tests: `cargo test -p dunlin backup`, `cargo test -p dunlin-workspace catalog_backup`,
`cargo test -p dunlin-storage catalog_import`. Before release: full tests and a manual
export → import to another directory → single/group download → restart run.

Original verification: 393 app tests, 35 storage tests, 17
domain tests and 3 new repository-layer tests pass. A broad parallel workspace
run reported failures in other Git/comparison/cleanup tests; when rerun individually,
`cleanup_revalidates_ignores_and_repository_identity_and_leaves_nonignored_folders`
passed. Do not treat the entire workspace regression as passed.
