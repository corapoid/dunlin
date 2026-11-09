# Session tabs and UI preferences

## Ownership and close behavior

A conversation/runtime is identified by **session id**, not only project id.
**⌘N** or the conversation-tab **+** opens another session tab. Titles follow
the saved conversation title/first prompt. Each tab has its own draft, transcript,
scroll state and harness/model metadata.

Closing a conversation tab hides it; it does not delete saved history or authorize
agent interruption. Reopen it from Sessions. Closing the selected tab returns to
the most recently visited remaining tab in the same pane; closing an inactive tab
does not change selection. File close/Save All has separate dirty-buffer guards.

Local and remote connections can be parked in a session-owned runtime while
another conversation is visible. Incoming ACP events route by generation/session
identity; a newly assigned remote server id rekeys the matching runtime only.
Foreground model/mode metadata must be ready before a prompt dispatch. Background
queued follow-ups wait for foreground guards; parking is not a durable scheduler.

A real relay disconnect, shutdown, revoked grant or lease takeover has separate
process/linger semantics. Do not infer indefinite remote execution from an open
tab. See [relay setup](../administration/relay-setup.md).

## What survives restart

| State | Lifetime |
| --- | --- |
| Saved sessions and transcripts | Persisted history; harness resume depends on adapter support. |
| Connected/parked processes | Runtime state, not promised replay/resume after app restart. |
| Conversation drafts, open tabs, file Undo and pane layout | In-memory; not a durable workspace snapshot. |
| Theme, density, inline-blame enablement and supported preferences | Persisted settings; caches remain file/backend/HEAD-scoped. |
| Reading zoom | Shared chat/code/terminal zoom for the current run. |

Two document panes share canonical file buffers and runtimes rather than creating
duplicate processes/files. Layout restoration and persistent drafts are future
work, not implied by saved conversation selection/history.

## Appearance and response identity

Settings → Appearance offers System, Light and Dark. System observes native
appearance changes; explicit modes remain overrides. Density reaches project,
session and Explorer rows while retaining hit-target floors. Reduced motion
suppresses decorative motion without hiding status.

Assistant headers use recorded dispatch-time harness/model/duration, not today's
picker selection. Unknown imported/legacy model metadata remains unknown.
Bundled harness marks fall back to initials; no runtime logo fetch is required.
Old removed minimal/flat-project settings must not silently revive hidden modes.
See the [settings map](settings.md).

## Verification boundaries

Native fixtures exercise concurrent mock sessions, distinct drafts/transcripts,
tab close/reopen, remote-id routing, settings movement and preference compatibility.
They are not live-provider/deployed-relay, physical keyboard or manual visual
acceptance. See [GUI scenarios](../quality/gui-scenarios.md) and
[historical implementation evidence](../archive/session-tabs-and-ui-preferences.md).

Sources: [runtime parking/routing](../../apps/desktop/src/harness.rs),
[session slots](../../apps/desktop/src/workbench.rs),
[panes](../../apps/desktop/src/workspace_panes.rs),
[shared settings](../../crates/domain/src).
