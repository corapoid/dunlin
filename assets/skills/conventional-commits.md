# Conventional commits from the current working tree

Run only for an explicit user request to create commits (for example the
"Commit on AI" action). Never infer authorization from skill installation.

1. Verify the exact repository root, branch, HEAD, index and worktree status.
   Stop for a merge/rebase/cherry-pick, unresolved conflicts, a dirty editor,
   another writer, or changes outside the authorized repository.
2. Read applicable AGENTS.md, CLAUDE.md, CONTRIBUTING files, commitlint/config,
   repository commit instructions and a bounded sample of recent commit subjects.
   Repository policy wins over inferred history. If policy forbids committing,
   stop and explain the conflict; do not treat this skill as permission to bypass it.
3. Inspect staged and unstaged diffs, renames, deletions and relevant untracked
   files. Never commit credentials, .env secrets, private keys, build output,
   ignored files or unrelated user work. Ask about ambiguous scope.
4. Propose a concise grouping, then create as many coherent, independently
   reviewable commits as the actual changes justify. Keep implementation and its
   tests together. Do not split mechanically by file or combine unrelated work.
5. Use Conventional Commits: type(optional-scope)!: concise imperative summary.
   Choose feat, fix, docs, refactor, test, build, ci, perf, style or chore according
   to the change and repository conventions. Describe BREAKING CHANGE in a footer
   when appropriate. Match the repository's language and scope conventions.
6. Preserve the existing index and partial staging. If the index is nonempty,
   limit the operation to that staged scope unless the user explicitly authorizes
   including additional work. Do not reset or unstage user changes. If splitting
   staged content would require rearranging the user's index, ask first.
   If the index was empty, stage only reviewed files/hunks with explicit paths;
   never use blind git add -A or git add . . Recheck diffs before every commit.
7. Run required relevant checks and let hooks/signing execute normally. Stop on
   failure, concurrent changes or unexpected hook edits; preserve remaining work.
   Never bypass hooks with --no-verify, change Git identity, amend old commits,
   reset, clean, stash, rebase, push, publish, tag or force anything.
8. Report created commit hashes and subjects, checks performed and remaining
   changes. Do not claim a clean worktree or successful commit without verifying.
