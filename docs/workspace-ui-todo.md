# TODO — terminal, workspace UI, Git and project sessions

- [x] Terminal: render Nerd Font / Powerline icons and preserve VT cell positions, including right-aligned prompts and wide characters.
- [x] Conversation: use the full available workspace width, without widening the composer.
- [x] Composer: reduce the text area slightly and increase corner radius.
- [x] Agent graph: extend the dotted background across the entire viewport, also for small graphs.
- [x] Source Control: file context menu with discard and add-to-gitignore actions.
- [x] Source Control: add Unstage all and confirmed Discard all changes.
- [x] Branch switching: replace the native keep-changes prompt with a themed in-app confirmation.
- [x] Branch switching: add explicitly confirmed Discard changes and switch, with fresh repository checks.
- [x] Project tree and list: show running sessions and sessions active within the last eight hours.
- [x] Project sessions: add a running-only filter, session selection and New session per project.
- [x] Project sessions: display each session's recorded branch and actual workspace/worktree, without inventing bindings.
- [x] Project list: remove directory/breadcrumb text below projects.
- [x] Dashboard: hide the projects sidebar without overwriting the user's sidebar preference for other views.
- [x] Document tabs: rounded browser-style tabs; inactive close buttons visible only on hover/focus.
- [x] Document tabs: move Close all into the tab strip's ellipsis menu; preserve unsaved-buffer guards.
- [x] Regression tests: temporary Git repositories, terminal glyph/cell geometry, session time boundaries, UI/native smoke and a separate test bundle.

## Safety and scope

Destructive Git actions require explicit confirmation tied to a fresh repository
snapshot. Never run them against the developer's working repository while testing.
Ignored files, nested repositories and dirty editor buffers must not be silently
deleted. Existing local/remote permission and operation guards remain authoritative.
"Running" comes from live runtime state; recent activity comes from stored session
timestamps. A current project branch is not proof of a historical session's branch.

## Implementation notes

- Composer stays at 824 px maximum width; text area is 72 px and card radius 20 px.
- Terminal uses the configured code font plus bundled Symbols Nerd Font Mono
  3.4.0. VT columns, wide characters and colored empty cells keep their positions.
- New Git actions use the existing local/relay two-phase operation protocol.
  Confirmation pins repository identity, refs, index, dirty bytes and `.gitignore`.
  No reset-hard, recursive deletion, automatic stash, retry or rollback is used.
  Old relays must be updated; a failed request never falls back to local Git.
- Discard includes staged and unstaged edits for the chosen path, restores rename
  sources and removes selected untracked files. Directories, submodules and nested
  repositories are refused, not recursively removed. A failure can be partial;
  inspect refreshed state before another operation and stop external writers first.
- `.gitignore` receives an escaped, literal root-relative entry. Tracked files
  remain tracked. Symlinked or hard-linked `.gitignore` files are refused.
- Save dirty editor buffers before Git mutations. Buffers changed by Git remain
  open with an explicit reload/compare conflict instead of losing their text.
- Sessions use live runtimes, not persisted Running labels. The eight-hour window
  includes its boundary. Filter state is per app run; history is never deleted.
- Schema 14 adds the nullable recorded session workspace. New conversations/turns
  record it; unknown historical workspaces remain unknown. Legacy `now`/relative
  date labels use the last persisted transcript time (creation time if absent).
  Exact imported dates and explicitly unknown imported dates remain unchanged.

## Verification — 2026-11-05

- `cargo build -p dunlin`: passed.
- `cargo test -p dunlin -- --test-threads=4`: 360 passed, including concurrent
  session-scope work and terminal cell/time-boundary regressions.
- `cargo test -p dunlin-storage`: 32 passed, including schema 14 migration,
  workspace persistence, transcript preservation and unchanged unknown dates.
- Workspace suite: 135/136 passed on the full run; the existing comparison
  second-checkout test hit a remote-check timeout under parallel load and passed
  on its own retry. All new discard/ignore/unstage/switch safety tests passed.
- `env SHELL=/bin/sh cargo test -p dunlin-relay -- --test-threads=1`: 47 passed,
  including authenticated one-use/stale-consent coverage for the new Git actions.
  With the default login shell, the existing PTY-exit test intermittently timed
  out; an isolated default-shell run passed. Shell lifecycle code was not changed.
- `--workspace-ui-smoke`: passed in a fresh isolated fixture. Native shaping
  verified Powerline, Nerd Font and CJK glyphs; measured conversation/composer and
  graph viewport geometry; exercised project tree/list rendering, dashboard
  sidebar animation, themed Git cancel/discard-switch and stale-consent refusal.
  It also drew active/inactive file tabs and Source Control, and verified the
  concurrent general/group-session navigation and delete-isolation changes.
- `git diff --check`: passed.
- Native regression smokes: 700-session import, glass/popover/input handling and
  onboarding/permissions/collection import all passed in fresh fixture directories.
- Separate ad-hoc-signed debug bundle:
  `dist/workspace-ui/Dunlin Workspace UI.app`. Previous bundles were preserved.
  `codesign --verify --deep --strict` passed; all 19 Mach-O code/data sections
  match the tested executable (whole-file signatures differ after app signing).

No destructive operations were run against real project repositories. No manual
visual/screenshot acceptance, production relay deployment or notarized release
build was performed. Inactive tab hover/focus appearance still needs human visual
acceptance; its GPUI styles and existing dirty-buffer close guards are retained.
