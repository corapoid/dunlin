# Application settings

Open Settings with **⌘,**. **⌘F** focuses settings search, which matches page titles,
setting names and aliases such as *harness*, *secrets*, *sidebar*, *backup* and
*terminal*. The navigation list scrolls independently of the page. **Tab** and
**Shift-Tab** reach controls; **Enter/Space** activate action controls. **Esc** or
**⌘W** closes settings, asking before discarding an open credential or skill form.

## Organization

| Group | Pages |
| --- | --- |
| Application | General; Appearance; Workspace; Chat; Editor & Terminal |
| Agents | Agents; Skills; Extensions |
| Security & Privacy | Permissions; Credentials; Security Audits |
| System & Data | Remote · Tailscale; Usage & Limits; Data & Backup; Diagnostics & About |

- **General:** startup destination, session restoration and desktop notification preference.
- **Appearance:** theme, interface font, density, motion and shortcut hints.
  The text-only **Skin** dropdown is independent of **System / Light / Dark**:
  **Default** preserves the original appearance; **Solid** uses opaque surfaces without native glass;
  **Grain** adds static fine grain to soft gray/deep midnight navy interface backgrounds
  with only 3% window translucency. Reading areas, code, terminal and inputs stay
  smooth. macOS Reduce Transparency makes the window opaque without removing the
  grain. Skin changes apply immediately, save locally and travel with settings
  backups. Existing installations keep Default.
  Solid uses stronger separation between dark chrome, reading panels and the
  composer, and darker neutral gray chrome in Light. Dark Grain also separates
  its reading surfaces from the deep navy window background more clearly.
  On macOS this also shows the active Metal GPU renderer. Acceleration is already
  enabled; the current backend has no CPU fallback or functional GPU-off switch.
- **Workspace:** Dashboard widgets, sidebar/project presentation, Explorer sizes,
  GitHub/GitLab CLI status and watched project folders. Project/group default
  editors are linked here and remain available from their context menus.
- **Agents:** local ACP adapters, installation/import/default-agent actions,
  session import, and **OpenCode Free privacy**. Free consent is local permission,
  not the provider account's training opt-in; enabling opens a disclosure window,
  and disabling blocks new Free sends. Auto-connect and restart on
  crash are configured only here. Opening a remote project never silently turns
  this page into server administration; use **Manage server** instead.
- **Extensions:** package installation, capabilities and global enablement.
  **Intent, Concepts & Decisions** includes Spec, Intent Inbox and agent contracts;
  there is no separate Spec & intent switch. Installation does not imply enablement.
- **Permissions:** agent approval policy and boundaries are separate from macOS
  permissions, including notification permission and its macOS status.
- **Credentials:** the Mac's Keychain storage and broker. Basic entries are separate
  from expandable access policies, HTTP operations, CLI recipes and activity.
- **Security Audits:** Configuration controls tool visibility and describes
  boundaries; Project audits contains explicit audit actions and evidence. External
  Codex scans and sandboxed checks do not sandbox ACP agents or prove security.
- **Usage & Limits:** Limits & alerts contains preferences, even before scanning;
  Cost report contains the evidence and breakdowns. A budget alert is not a hard
  spending cap and excludes unknown prices.
- **Data & Backup:** portable export/import, local storage and confirmed recovery
  actions. Portable exports do not contain conversation history or credentials.
- **Diagnostics & About:** agent logs/export, environment report and app version.

The user menu shares Appearance's **System / Light / Dark** selector. **Minimal
view** enables compact spacing, including project/group rows in the title-bar
project selector. The interface font list always starts with
**Geist (Default)**. Editor's live font preview sits directly below Typography.
Resize the sidebar by dragging its edge; double-click the edge to reset its width.
The resize indicator is hidden until hover or dragging. It is 1px wide, with a
separate 10px drag target in every skin.
The three primary sidebar tabs use standard interface icons. The repository
button in the bottom status bar aligns vertically with Settings.
The project selector and Search share the same borderless field style. Search
opens with a short fade/slide animation; Reduce motion disables it.
Settings is available in the sidebar footer, with **⌘,**, or through global search.
The avatar popover contains appearance, density and available GitHub accounts;
it does not duplicate Settings.
Single-line text fields use the same height in settings, dialogs and search bars
throughout the application; Compact applies the same reduction everywhere.
Multiline editors and the chat composer retain their content-based sizing.

## Scope and saving

Most switches and choices save immediately on **this Mac**. Named forms have
explicit **Save / Cancel**. Credential drafts are cleared after confirmed discard,
never retained as a hidden pasted value. Navigation history and workspace commands
do not silently discard these forms.

Project appearance/agent choices are explicitly marked **saved immediately**.
The default harness uses a dropdown; the color picker shows a circular dot.
The separate model editor has its own **Save / Cancel**;
closing with that form open asks before discarding it. Group settings use a single
draft and **Save group settings / Cancel**. Both show their scope and inheritance
sources. See [runtime defaults](../reference/project-runtime-settings.md).

Group icon choices show their logos, with a colored dot for the dot option.
Group color and default harness use dropdowns. Neither project nor group settings
contains permission/environment controls or a bulk override-reset action; existing
saved values are preserved. Both windows offer a scoped generated-data cleanup
preview through **Clean project… / Clean group…**.

Group settings have a fixed header/footer and one scrolling form with a visible
scrollbar when needed. **Delete group…** asks for
confirmation, then promotes projects and subgroups one level without deleting
files or conversations. Unsaved form changes are discarded; the deletion offers
guarded **Undo**.

Remote · Tailscale names the server being managed. Remote tools, accounts and project
operations run there; appearance stays local. Copying a project or exporting to
a repository is explicit, not background synchronization.

## Regression validation

`cargo test -p dunlin` covers menu completeness, search aliases, preference
compatibility and security-catalog visibility alongside existing application tests.
In a debug build, run:

```sh
target/debug/dunlin --settings-smoke /absolute/path/dunlin-settings-smoke-unique
```

The directory must be new. This fixture uses an empty home, disables agents and
the credential broker, renders all 15 pages in all three skins, both appearances
and densities at 1100×720 and 1360×860, and tests keeping/discarding an unsaved
credential form plus keyboard activation of filtered search results, appearance
cards and the skin dropdown. It checks skin restoration and removal of native
glass when switching to Solid/Grain, repository/Settings alignment, and the thin
resize indicator's hover/leave visibility, wide drag target and double-click reset.
It also tests reset
blocking/cancellation and a confirmed reset of its disposable fixture database,
stopping at the restart boundary. It never touches real data or credentials.
