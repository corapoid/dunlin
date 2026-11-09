# Workspace, navigation and files

## Projects and groups

**⌘O** opens project import; **⌘⌥P** searches configured projects by name/path.
The title-bar project selector switches projects. Use a project's sidebar context
menu for settings, trust, pinning, grouping, locating and removal.

Groups are logical organization, not parent filesystem grants. Drag a project
onto a group to change membership without moving files. Nested groups reject
cycles; deleting a group promotes direct members, with guarded Undo. Group agents
use a frozen set of explicitly trusted project roots, not every future member.
See [collection/group contracts](../reference/collection-import.md).

Removing a project removes Dunlin's local record/history, not repository files;
it does not delete a project from its relay. Conversation/project deletion is not
a backup strategy. [Standalone chats](standalone-chat-and-worktrees.md) remain
separate from repository projects.

## Navigation and tabs

- **⌘K** opens the command palette; **⌘[ / ⌘]** navigate back/forward.
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

Project icons come from bounded local assets or an explicit custom image, never
from executing repository configuration or fetching favicons. Settings → Workspace
controls discovery and project presentation; Settings → Appearance controls theme
and density. See [settings](settings.md).

Implementation: [app](../../apps/desktop/src/app.rs),
[workbench](../../apps/desktop/src/workbench.rs),
[Explorer](../../apps/desktop/src/explorer.rs),
[project groups](../../apps/desktop/src/project_groups.rs),
[dashboard](../../apps/desktop/src/dashboard.rs).
