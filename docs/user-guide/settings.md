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
| System & Data | Remote & SSH; Usage & Limits; Data & Backup; Diagnostics & About |

- **General:** startup destination, session restoration and desktop notification preference.
- **Appearance:** theme, interface font, density, motion and shortcut hints.
- **Workspace:** Dashboard widgets, sidebar/project presentation, Explorer sizes,
  GitHub/GitLab CLI status and watched project folders. Project/group default
  editors are linked here and remain available from their context menus.
- **Agents:** local ACP adapters, installation/import/default-agent actions,
  and session import. Auto-connect and restart on
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
view** enables compact spacing. The interface font list always starts with
**Geist (Default)**. Editor's live font preview sits directly below Typography.
Resize the sidebar by dragging its edge; double-click the edge to reset its width.
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

Project appearance/agent/approval choices are explicitly marked **saved
immediately**. The separate model/environment editor has its own **Save / Cancel**;
closing with that form open asks before discarding it. Group settings use a single
draft and **Save group settings / Cancel**. Both show their scope and inheritance
sources. See [runtime defaults](../reference/project-runtime-settings.md).

Remote & SSH names the server being managed. Remote tools, accounts and project
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
the credential broker, renders all 15 pages in both themes/densities at 1100×720
and 1360×860, and tests keeping/discarding an unsaved credential form plus keyboard
activation of filtered search results and theme cards. It also tests reset
blocking/cancellation and a confirmed reset of its disposable fixture database,
stopping at the restart boundary. It never touches real data or credentials.
