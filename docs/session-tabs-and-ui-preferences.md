# Session tabs and UI preferences — 2026-11-05

## Scope and contracts

Density reaches project/session/explorer rows and app controls. Remove flat project
view, top-bar search, minimalist mode, editor's duplicate headers and the global
group-appearance dropdown (group-header/context actions remain). Persist blame as
an application preference, with Explorer and settings toggles. Add system theme,
draggable/clamped Settings, visible topic chips, dismissible QA details, multiple
session-owned chat tabs with dynamic titles, and harness/model response identity.

| Claim | Evidence and exact location | Revision/version | Confidence | Missing link | Next automated lookup |
| --- | --- | --- | --- | --- | --- |
| Session id, not project id, identifies a conversation/runtime | desktop `harness.rs::{park_current_runtime,restore_runtime,route_harness_event}`, `app.rs::new_session`, storage transcript keys | current working tree | high | remote parking currently disconnects | check relay leases and adapter routing before extending |
| Local background agents already preserve their runtime and permissions | desktop `harness.rs::{route_harness_event,handle_background_event}` and `ParkedRuntime` | current working tree | high | per-session visible tabs/drafts | native mock concurrent-session acceptance |
| Chat title is the persisted session title, updated from first prompt | desktop `harness.rs::{session_title,update_active_session}`, `SessionSummary` | current working tree | high | none | title/tab regression |
| Blame content is tab/backend/HEAD scoped, but enablement currently lives on each tab | desktop `inline_blame.rs::{State,scope_current,refresh_inline_blame}` | current working tree | high | persisted enablement | settings roundtrip and cross-file smoke |
| Native appearance observation exists; component theme changes do not need an app restart | `vendor/gpui-pre-0.3.9/src/app/context.rs::observe_window_appearance`, component theme `Theme::change` | GPUI pre 0.3.9, component 0.8.0 | high | mode preference/migration | native appearance observer and explicit-mode tests |
| Settings is drawn in a parent-sized attached render plane, not a separately movable content window | desktop `overlay_window.rs::{prepare,reconcile}` and settings `render` | current working tree | high | content drag offset | drag content panel only, clamp to viewport |

```text
project --sessions(project_id), proven--> session.id
session.id --parked_runtimes[id], proven--> ACP generation/runtime/transcript
ACP generation --route_harness_event(generation), proven--> exact owning session
chat tab --session.id, planned--> existing runtime and persisted session title
system appearance --observe_window_appearance, proven--> resolved light/dark theme
```

No transcript deletion or agent interruption is authorized by closing a tab.
Unknown historical response models must not be invented; new response metadata
must preserve the actual model/harness at dispatch. Existing explicit light/dark
settings migrate without resetting unrelated preferences. No new dependencies.

## Confirmed lifecycle and compatibility

- Relay `acp.rs` explicitly owns one process per WebSocket; leases are keyed by
  session/connection. The desktop now parks that connection too, rather than
  cancelling it on session navigation. Pending remote actions belong to each
  `HarnessState`; server-created IDs rekey only the matching tab/runtime. No
  relay authentication, grant, lease or process-limit relaxation was made.
- `crates/agent/src/lib.rs::create_session` emits session-created/loaded first,
  then Models, then Modes (including explicit `None` for unadvertised metadata).
  Native testing exposed first-prompt dispatch before those snapshots. Dispatch
  now waits through that sequence and records optional `TurnTiming.speaker`.
  Existing JSON remains readable through `serde(default)`; legacy imports do
  not invent a model. Queued background requests still wait for a visible tab.
- Old `minimal_mode` and `project_view` JSON fields are ignored, not migrated
  into a hidden mode. Existing `dark` values migrate to explicit appearance;
  new installs default to System. Blame enablement is persisted separately from
  file-scoped caches. No history or stored evidence was deleted.
- Native marks are bundled (Simple Icons CC0, OpenCode and Hermes MIT). Unsupported
  or custom harnesses use their initials, never the Dunlin mark. No runtime logo
  fetching or extra dependency was added.

## Verification log

- Desktop 376 tests and UI 5 tests passed, including preference migration,
  model-signature formatting, bundled logo decoding and clamped panel movement.
- Native workspace: Compact project row is measurably shorter without a tiny hit
  target; Settings moved by the rendered header drag; QA Close dismissed details.
- Native mock ACP: two ordinary sessions ran concurrently, with distinct drafts
  and transcripts; closing/reopening active and background tabs did not cancel
  agents; persisted turns contain dispatch model/harness and elapsed time.
- Native Git: blame remained enabled and correctly attributed a second source
  file, and the preference was persisted. Existing dirty-buffer/stash/blame
  safety acceptance also passed. The first extended fixture closed the second
  tab without reactivating the first before its edit; the fixture was corrected.
- Relay: an isolated-Git parallel run reported 47/48 (merge-fixture expectation);
  the full serial rerun with `SHELL=/bin/sh` passed 48/48 without code changes.

Further native acceptance passed in dark/wide and light/narrow workbench windows.
The in-process remote routing fixture verified generation/session ownership,
pending→server session-ID rekeying and rejection of unrelated session updates
without a network connection. Pending remote model selection is runtime-owned.

Native appearance acceptance changed only the fixture application's AppKit
appearance override (not macOS settings): System followed dark→light changes;
explicit Dark did not follow a native Light change; clearing the override restored
System. The first extended workspace run stalled in an earlier Git preflight;
the fixture now disables fsmonitor and reports its last notice on timeout. The
full isolated-Git rerun and the signed bundle's full workspace run passed.

Bundle: `dist/session-tabs/Dunlin Session Tabs.app`, a separate ad-hoc-signed debug
application. Previous bundles are retained; changes remain uncommitted. No
notarized/release build is claimed. `codesign --verify --deep --strict` passed.

Final rebuilt bundle passed the full workspace UI smoke again. The final source
binary passed concurrent-session/remote-rekey acceptance, and desktop 376 + UI 5
tests passed again. Including the separate 48-test relay rerun, 429 relevant tests
passed. Dispatch additionally waits for in-progress model/mode changes, not just
initial metadata; server-ID changes update saved session slots and project
selection as well as the tab/runtime key.
No live model account, real remote relay or manual visual acceptance is claimed.
