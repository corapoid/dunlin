# UI input and adapter discovery handoff

Scope: group-name input, project-list dismissal and Harnesses settings.

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| The active modal had a native first responder different from its GPUI view; restoring it after native style/parent changes fixes the input route | `apps/desktop/src/glass_smoke.rs`: `assert_native_input_responder`, `post_native_key`; `apps/desktop/src/overlay_window.rs`: `native::attach`, `makeFirstResponder` | gpui-pre-macos 0.3.9 / working tree | High: native responder regression failed before the fix | Physical keyboard/IME QA remains manual | Run AppKit-key smoke and then check a rebuilt app |
| The project list lacked outside dismissal; component menus already provide it | `apps/desktop/src/views/sidebar.rs`, `project_menu`; `vendor/gpui-component-0.8.0/src/menu/popup_menu.rs`, `on_mouse_down_out` | Working tree / component 0.8.0 | High | None | Native smoke |
| Claude Code's separate ACP executable is `claude-agent-acp`, not `claude`; the reported install is home-local | `~/node_modules/@agentclientprotocol/claude-agent-acp/package.json`, `bin`; executable symlink `~/node_modules/.bin/claude-agent-acp`; `apps/desktop/src/harness_discovery.rs`: `home_bins`, `SPECS` | Installed 0.100.1-preview.4, inspected 2026-11-05 | High | Actual authentication/agent connection was not exercised | Read-only scan, then user-controlled connection |
| Discovery previously omitted NVM version bins | `apps/desktop/src/harness_discovery.rs`, `search_paths`, `nvm_bins` | Working tree | High | Other custom npm prefixes are not inferred | Explicit Custom ACP path |
| Set up must not synchronously open a browser or pretend to install a package | `apps/desktop/src/views/settings.rs`: `harness_setup`; `apps/desktop/src/harness_discovery.rs`: `show_harness_setup`, `open_harness_docs`, `rescan_harnesses` | Working tree | High | Browser launch itself is not tested against user preferences | In-app setup/async rescan smoke; browser opener uses the tested bounded process helper |
| Settings selection changes only the new-conversation default | `apps/desktop/src/views/settings.rs`, `harnesses`; `apps/desktop/src/harness.rs`, `connect_harness` executable guard | Working tree | High | None | Native smoke checks the existing session remains unchanged |

Proven retrieval path: Settings Scan → `discover` → `search_paths` → executable
metadata → `Detected.adapter` → settings status. Launch independently resolves
the same command and search paths. Detection does not execute adapters, authenticate,
or install software. Missing files remain unavailable; rescans read fresh metadata.

Rejected equivalence: an installed Claude CLI does not prove an ACP adapter exists.
No catalog names, session identities or stored settings formats change. New
discovery scope covers the local user's NVM version bins and home-local npm bins;
repository dependencies and remote detection are unchanged.

Verification: desktop unit tests, `--glass-smoke` (group typing/Enter, outside click,
radio selection, new NVM adapter rescan), and `--onboarding-smoke`. Next owner: app
maintainer for native keyboard and real adapter connection QA; no provider request
or user installation is required by the automated fixtures.

## Chrome, navigation and confirmations — 2026-11-05

- Harness status and its connection/error detail moved out of the title-bar badge
  into the selected Local host/relay row. Non-selected devices do not borrow the
  active session's status. The host connection dot remains in the title bar.
- Back/Forward destinations contain content only: project, relevant session,
  main/file/Git/spec surface, Settings/Dashboard/Security. Sidebar section/layout,
  window bounds, panel widths and sidebar collapse do not create destinations;
  file/Git destinations do not record unrelated session changes. Returning to
  content does not restore old window dimensions. Dirty-editor/navigation guards
  remain in place.
- Usage & Limits uses the same single-color Gauge icon as other rail controls.
  Projects, their sessions and inline group editor are inset another 24 px within
  sidebar group sections; ungrouped projects and the flat List are unchanged.
- Add projects starts at the user's home. Folder labels are left-aligned within
  full-width buttons. Entries are actual directories only, excluding dot names,
  symlinks, and macOS Finder-hidden flags. Up stops at home; an explicitly entered
  absolute path remains supported (this is not a filesystem sandbox).
- `crates/ui/src/prompts.rs` installs a single themed GPUI prompt builder. The
  inventory of `window.prompt`/`w.prompt` covers AI/manual commit, Git/checks/
  review/comparison/budget/subtask actions, Explorer, task runner, spec/security,
  vault, skill deletion, editor conflicts and factory reset. These callers retain
  their original warnings, answer indices and post-confirmation scope/stale checks.
  Escape chooses only an explicit Cancel answer; Enter on the dialog itself
  cannot authorize the destructive option. Buttons remain keyboard-focusable;
  long detail text scrolls and actions remain outside that scroll area. Native
  backdrop glass is cleared while a GPUI confirmation is active so it cannot blur
  or cover the new in-window panel. OS file/keychain/permission sheets remain native.

Acceptance fixtures add checks for content-only Back after layout/resize changes,
themed prompt mouse selection and exact three-answer Escape semantics. Folder
tests include dot folders, Finder-hidden flags, files and directory symlinks; the
import smoke also asserts that its initial browser path is home. No AI request,
real credential action or production-repository confirmation is executed by these
checks. Final execution results are recorded below.

Initial automated results: 373 desktop tests, the UI prompt cancellation test and
3 UI asset integration tests passed. Native workspace smoke passed content-only
navigation, device-menu status placement, project indentation and themed prompt
mouse/Escape checks. A first extension of the smoke dispatched Escape while the
owner entity was borrowed; input dispatch now happens outside that borrow, like
the existing native click tests. Project-import smoke and glass/input smoke passed.
No provider session, actual AI commit, vault mutation or factory reset was run.
The two non-"Cancel" cancellation labels are marked explicitly as cancel answers
at their call sites: vault **Deny** and quit **Keep Running**. Their labels and
indices are unchanged; Escape/default Enter cannot grant a credential or quit.
The final workspace fixture also checks default Enter with an explicitly typed
non-"Cancel" cancellation label. Dialogs expose a Dialog role, accessible title,
tab group and ordinary focusable application buttons. Source search found no
separate NSAlert/runModal confirmation implementation bypassing the shared builder.
Final build and test rerun: 373 desktop tests and 4 UI tests passed. The standalone
bundle is `dist/ui-polish/Dunlin UI Polish.app`; `codesign --verify --deep --strict`
and `git diff --check` passed. Older bundles were preserved. Pixel-by-pixel manual
review, production relay/harness connections and actual destructive confirmations
remain untested; automated confirmations use harmless fixture-only receivers.
The final packaged executable passed the full extended native workspace smoke,
including selected-device status placement, indentation, resize/navigation,
mouse selection, Escape and safe default Enter with the typed Keep Running label.
