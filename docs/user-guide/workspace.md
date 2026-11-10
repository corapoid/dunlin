# Workspace, navigation and files

## Projects and groups

**⌘O** opens project import; **⌘⌥P** searches configured projects by name/path.
The title-bar project selector switches projects. Use a project's sidebar context
menu for settings, trust, pinning, grouping, locating and removal.

The **+** beside **SWITCH PROJECT** inserts a draft at the top of the selector.
Type a name to update its monogram live; **Enter** confirms and **Escape** cancels.
Confirmation creates an empty local project under Dunlin's data directory using
an opaque directory ID (not the display name). It is not a standalone chat, does
not initialize Git, and does not switch away from unsaved work. Click the new row
to open it. Use project import for an existing folder or repository.

Groups are logical organization, not parent filesystem grants. Drag a project
onto a group to change membership without moving files. Nested groups reject
cycles; deleting a group promotes direct members, with guarded Undo. Group agents
use a frozen set of explicitly trusted project roots, not every future member.
See [collection/group contracts](../reference/collection-import.md).

The group settings dialog uses a scrolling form with fixed actions. **Delete
group…** confirms before removing the group, preserving project files and history
and promoting projects/subgroups one level. Cancel retains the settings draft.

Removing a project removes Dunlin's local record/history, not repository files;
it does not delete a project from its relay. Conversation/project deletion is not
a backup strategy. [Standalone chats](standalone-chat-and-worktrees.md) remain
separate from repository projects.

### Clean generated project data

Right-click a project and choose **Clean project…**, or right-click a group and
choose **Clean group…**. These actions are also available in project/group
settings. Group cleanup includes
projects in subgroups. This is separate from removing a project from Dunlin.

The background scan shows recognized generated folders, their estimated sizes,
and checkboxes. Use **Select all / Select none** or choose individual folders;
the selected count and size update before **Move selected to Trash…** asks for
confirmation. Cancel, including Escape during inspection/review, changes no files.

- Dependencies: `node_modules`.
- Caches: `.turbo`, `.pytest_cache`, `__pycache__`, `.next/cache`.
- Builds: Rust `target` beside `Cargo.toml`; JS `dist`, `build`, `.next`, `.nuxt`,
  `.output` and `.svelte-kit` beside `package.json`, including monorepo packages.

Only folders ignored by Git, containing no tracked files, are offered. Generic
`build`/`dist` names without a matching project manifest are not enough. Scans do
not follow symlinks, cross mount points, include Git metadata/nested repositories,
or run clean scripts from the project. Incomplete inspections are skipped.
Local trusted Git repository roots are supported; remote, untrusted, missing and
non-repository projects are reported as skipped without blocking other group
members. Scanning has bounded entry/depth/time limits. Sizes are logical file-size
estimates, not guaranteed disk savings.

Save edited files and stop agents, terminal sessions and repository operations
before cleanup. Stop external build/install tools too; Dunlin cannot detect all
external processes. Close tabs inside selected folders. Membership, trust, paths,
Git eligibility and folder identities are checked again before moving anything.
The result lists folders moved to Trash and skipped items with reasons, including
partial system-Trash failures. Dependencies/build outputs must be recreated before
working on the project again. All contents of a selected folder move together,
including any custom files you placed inside it.

**Cleanup uses system Trash only.** Folders can be restored from Trash; disk space
is reclaimed only after you empty it yourself. Dunlin never empties Trash and has
no permanent-delete cleanup mode.

## Navigation and tabs

- Click the centered title-bar **Search…** field or press **⌘K** for global search.
  Suggestions appear before typing, with **Dashboard** always first. Search spans
  configured projects, saved sessions, filenames from the current loaded Explorer,
  settings and workspace actions. It does not scan file contents or query servers
  when opened. **↑/↓**, **Enter** and **Esc** navigate, open and dismiss results.
- The single sidebar has **Projects / Sessions / Explorer** icons at the top.
  **Projects** uses the same inline name search as Explorer: focus the list or
  select its tab, then type a name. **↑/↓** selects a match, **Enter** opens it,
  **Backspace** edits and **Esc** clears the query. Matching groups are temporarily
  expanded and restored on clearing. **+ New** opens the project source menu.
  Its footer contains **Source Control, Knowledge, Intent, Artifacts, Security
  Audit**. Click **Project workspace** to collapse this list downward; its state is
  remembered between launches. A bottom row keeps the **Settings** icon on the
  left and host/connection status on the right, even with the list collapsed.
  Intent appears only with its package enabled; Security Audit appears only with
  an installed, enabled audit skill for this project or globally.
  **⌘B** collapses the entire sidebar, including this
  footer; no separate navigation rail remains.
- **Chat / Graph / Timeline** appears beside the user avatar when a conversation is
  active. The avatar menu opens below the avatar and contains appearance controls,
  Minimal view and, when multiple GitHub accounts are logged into `gh`, an account
  switcher. Switching changes the active `github.com` CLI account; credentials
  remain in `gh`. Settings is available from the sidebar, not this menu.
  Connection status is not duplicated in the title bar.
- **⌘[ / ⌘]** navigate back/forward, including Knowledge and Artifact destinations.
- **Knowledge**, **Artifacts** and **Security Audit** open independent project
  documents in the normal tab strip. They share the workspace's rounded surface,
  support closing/splitting/reordering and do not open overlay windows.
- Right-click any document tab for **Lock/Unlock tab** and **Close tab**. Locked
  tabs show a lock instead of the close icon and are skipped by Close all tabs;
  unlock them before closing, including with **⌘W**.
- Git sync and change counts remain in the status bar in every workspace view,
  including Chat. Sync actions are shown when a configured upstream is known.
- Source Control keeps its commit controls fixed above a virtualized change list;
  only visible file rows are built. **Stage all** becomes **Unstage all** when
  every change is staged. The adjacent ellipsis menu includes Unstage all and
  Discard all changes (discard still confirms). File actions align to the right.
- Chat / Graph / Timeline and AI / Manual commit use flat sliding selection surfaces,
  without a glass lens or press zoom. Reduced motion disables the sliding animation.
- **Minimal view** keeps the avatar menu unchanged while simplifying session
  lists to one-line rows and tightening Project workspace entries and the
  project selector (including both project and group rows). Session
  metadata remains available in tooltips and context menus.
- **⌘N** opens another conversation tab. Closing one hides it; reopen saved history
  from Sessions. [Session scopes](session-list.md) keep general and project history separate.
- File buffers, undo, drafts and pane layout are not interchangeable with saved
  transcripts. Two panes share canonical buffers/runtimes; layout and drafts are
  in-memory. See [tab lifecycle](session-tabs-and-ui-preferences.md).
- **⌘+ / ⌘− / ⌘0** adjust reading zoom for chat, code and terminals during this run,
  without changing sidebar/settings size or terminal configuration files.

Save or close dirty files before switching projects. Navigation is not Undo and
must not silently discard buffers.

## Explorer and dashboard

Explorer supports file/folder creation, local Finder-copy imports, internal moves,
rename and confirmed deletion. Existing destinations are not overwritten. Local
deletion uses Trash; remote deletion is limited to files/empty directories. Dirty
or loading tabs can block mutations. Finder-drop uploads are not supported on a
remote project. [Editor and previews](editor.md) describes supported file content.

The Dashboard uses recorded activity and recent project visits. Trusted local
recent projects can perform a bounded read-only upstream check; this does not
fetch, pull or update tracking refs. Missing upstream, unavailable comparison and
failed checks are distinct from zero changes. Remote status must not borrow a
local repository's results.
Dashboard has no permanent navigation button; open the first global-search result.

Project icons come from bounded local assets or an explicit custom image, never
from executing repository configuration or fetching favicons. Settings → Workspace
controls discovery and project presentation; Settings → Appearance controls theme
and density. See [settings](settings.md).

Implementation: [app](../../apps/desktop/src/app.rs),
[workbench](../../apps/desktop/src/workbench.rs),
[Explorer](../../apps/desktop/src/explorer.rs),
[project groups](../../apps/desktop/src/project_groups.rs),
[dashboard](../../apps/desktop/src/dashboard.rs).
