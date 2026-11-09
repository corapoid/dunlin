# Git workflow

Source Control uses the system `git`. Non-repository folders show an explicit
Initialize action; Dunlin does not automatically initialize, stage or commit.

## Daily operations

Inspect status and staged/unstaged diffs before staging, unstaging, discarding,
committing, amending, pulling or pushing. A changed file opens its diff rather
than silently replacing the editor buffer. [AI commits](skills.md#ai-assisted-commits)
are a separate confirmed agent workflow, not an automatic commit suggestion.

The branch badge or **Switch branch…** opens a searchable branch picker. Unsaved
buffers require save/cancel. Keeping working-tree changes across a switch needs
separate consent; Git can refuse collisions or a branch used by another worktree.
There is no automatic stash, reset, force checkout or rollback. External Git
clients, editors and hooks are not locked out by Dunlin's in-process guards.

## History and mutations

**Repository → Branch Graph & History…** (**⌘⌥G**) opens history, changed paths and
bounded commit patches locally or on an updated relay. Root and merge-parent
comparisons have explicit baselines. Creating a branch is separate from checking
it out. See [history limits and safety](git-history.md).

Stash, registered worktrees, sequencer operations and conflicts appear in
Repository state. Mutations use fresh plans and explicit confirmations; dirty
buffers, stale state and concurrent operations can block them. See
[Git operations and blame](git-operations-and-blame.md) and the
[repository-state contract](../reference/git-repository-state.md).

**Subtask Worktrees** creates one separately confirmed branch/worktree from an
explicit revision; it does not start agents, merge or push. A worktree is not a
sandbox. See [subtask worktrees](standalone-chat-and-worktrees.md).

## Hosting links, not a PR client

The status bar can open validated `github.com` or `gitlab.com` repository links
derived from Git remotes. Unknown/ambiguous hosts stay unavailable. Detection
does not authenticate or contact the hosting provider. GitHub/GitLab project
imports remain separate features.

**Pull Request and GitHub Actions integration has been removed.** Old PR panels,
polling, mutation routes and `--github-smoke` instructions in the archive are not
supported functionality. Use the hosting service outside Dunlin for those tasks.

Sources: [Git UI](../../apps/desktop/src/git_history.rs),
[Git actions](../../apps/desktop/src/git_actions.rs),
[workspace Git](../../crates/workspace/src/git.rs).
