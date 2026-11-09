# Desktop behavior scenarios

Coverage here means a concrete interaction and its observable outcome, **not a
line percentage**. Each scenario starts a new process, database and project.
The inventory lives in `apps/desktop/src/gui_scenarios.rs`; CLI validation,
dispatch and `--list-native-smokes` all use that same inventory. The existing
long workspace smoke remains intact until its other assertions are migrated.

## Run

```sh
cargo build -p dunlin -p dunlin-agent --bin dunlin --bin mock_acp_agent --locked
python3 scripts/release_qa.py --native-only --native-filter 'gui-*' --tmp-root "$TMPDIR"
# Rerun just one failure:
python3 scripts/release_qa.py --native-only --native-filter gui-close-cancel-smoke --tmp-root "$TMPDIR"
```

No filter means **all** native tests, including these scenarios, run in the
existing required macOS CI gate. An unmatched filter fails rather than reporting
an empty successful suite. `report.json` records the selected checks, individual
step outcomes/durations, and fixture paths. A filtered run is not full acceptance.

## Scenario ledger

The names below are the discovered check names (also CLI flags after adding `--`).

| Priority | Check | Actual interaction | Visible/state + persisted/backend effect | Negative assertion |
| --- | --- | --- | --- | --- |
| P0 | `gui-save-reopen-smoke` | Type; Save All shortcut; close tab | Clean buffer; disk content; reopened buffer | No extra file writes |
| P0 | `gui-close-save-smoke` | Type; tab X; dialog Save | Dialog and tab close; disk and reopened content | No hidden file tab |
| P0 | `gui-close-discard-smoke` | Type; tab X; dialog Discard | Tab closes; original file survives reopening | Draft never reaches disk |
| P0 | `gui-close-cancel-smoke` | Type; tab X; dialog Cancel | Dialog dismisses; dirty buffer/tab remain | Disk unchanged |
| P0 | `gui-save-conflict-smoke` | Type; external writer; Save All shortcut | Conflict retained; local draft and external file preserved | No silent overwrite |
| P1 | `gui-close-clean-smoke` | Tab X | No dialog; closed/reopened clean file | No mutation/hidden tab |
| P1 | `gui-chat-send-click-smoke` | Type; Send button | Rendered reply; empty composer; saved transcript | Exactly one ACP prompt; neighboring history unchanged |
| P1 | `gui-chat-send-enter-smoke` | Type; Enter | Same as button path | No duplicate prompt or file write |
| P1 | `gui-chat-queue-smoke` | Enter during a blocked turn | Queue retained; after release, two saved turns | Second prompt never reaches ACP before release |
| P1 | `gui-chat-queue-clear-smoke` | Queue button; Clear | Queue closes; first turn completes; local user history retained | Cleared prompt never reaches ACP |
| P1 | `gui-chat-stop-smoke` | Stop button during a blocked turn | Not running; transport records cancellation | No agent answer or write |
| P1 | `gui-chat-retry-smoke` | Retry on previous user message | Second response and saved user turn | Same ACP text, exactly one extra request |
| P0 | `gui-permission-allow-smoke` | Allow once | Resolved permission, agent reply and saved decision | File absent before consent; exactly one decision |
| P0 | `gui-permission-deny-smoke` | Reject | Denial reply and saved decision | Agent's output file never created |

All chat scenarios additionally verify an unrelated stored conversation remains
unchanged. This does not yet test switching tabs *during* a running response.
The ACP mock writes only synthetic fixture artifacts; a journal verifies requests
and permission choices independently of desktop state. Its response gate is a
file controlled by the test, so queue/stop checks don't depend on beating a timer.

## Harness contract

- `Step::act` performs one action; `Step::until` polls a condition with a 10-second
  step deadline. The 20 ms tick only yields frame opportunities, never establishes
  correctness. Scenario processes also have an outer QA timeout.
- `Step::click` uses measured complete hit targets, checks viewport geometry and
  dispatches pointer events. Bounds are cleared and redrawn on every tick so a
  disappeared control cannot be clicked using stale geometry.
  Owner-attached dialog windows are drawn and receive input separately on macOS.
- `Step::key` uses GPUI's installed key routing; `Step::type_text` dispatches
  keystrokes through the focused input handler. Fixture setup focuses the editor
  or composer; input-focus traversal itself is covered separately by controls QA.
- All input dispatch happens **outside** `DunlinApp::update`. Fixture preparation
  can use controller APIs; tested Save/Close/Send/Queue/Stop/Retry/consent actions
  do not call their controllers directly.
- Measurement wrappers are disabled outside these scenarios and never change
  normal production layout.
- `GUI_STEP:` JSON records start, pass and failure. A caught assertion or timeout
  writes `failure.json` with the failing step, app state, synthetic transcript and
  target geometry. Uncaught crashes retain the last started step in the runner's
  report. CI uploads failure JSON, mock journals, app/process logs, not databases.
- Exit zero and `PASS:` are still required; an unfinished/failed step cannot pass
  even if a process emits a premature completion marker.

## Explicit gaps / next increments

- Save failures/close while HTTP is stalled: retain `save-smoke`; split its gates
  into this harness before retiring its assertions.
- Tab reorder/drag, cross-pane draft routing, switching projects during writes,
  retry after a failed backend response, disconnect/reconnect, denied budgets,
  trust revocation and restart/effect tests for individual settings remain work.
- Light/compact/narrow variants of every new workflow are not claimed. Existing
  controls/settings/layout matrices remain separate evidence.
- These are real AppKit windows with **in-process GPUI events**, not physical OS
  input. IME/bidi, TCC, VoiceOver, native material and visual acceptance remain
  separate. No screenshot/visual-diff acceptance is claimed.

For isolation and broader acceptance boundaries, see [test quality](test-quality.md).
