# R11 — manual QA matrix

Status: **manual acceptance, not covered by the automated pass flag.** This
document is the checklist for a fresh-account, TCC/GUI, upgrade, large-project
and real-relay run. `scripts/release_qa.py` covers only the isolated automatic
checks and must never be cited as R11 acceptance.

## Evidence

1. Launch the packaged `Dunlin.app` (not `target/debug/dunlin`).
2. Open **Settings → Advanced → Release QA → Run QA checks**.
3. Use **Copy report** and attach the report to the run record.
4. Record every case below as **PASS**, **FAIL**, or **DEFERRED** with a short
   observation. The report itself prints `Manual R11 status: NOT evaluated by
   this report.`

Template:

```text
Build/version:
Commit:
Date/tester:
macOS/account type:
Display/window sizes:
Relay host/build:
Overall result: PASS / FAIL / DEFERRED
Notes:
```

## Automatic prerequisites

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/release_qa.py --tmp-root "$TMPDIR" --native --audit
./scripts/bundle-macos.sh --build
```

The native set includes `--onboarding-smoke`: a fresh data directory, no saved
project, empty harness discovery, five permission rows, the copyable QA report
and the Permissions settings surface. It does not approve TCC prompts.

## Cases

| # | Case | Steps | Expected |
| --- | --- | --- | --- |
| R11.1 | Fresh account / empty data | Move or start with no `~/Library/Application Support/Dunlin`; launch the bundle. | Onboarding is shown; no demo project or session appears; no crash or hidden history. |
| R11.2 | No harness | On step 1 remove every ACP adapter from `PATH` (or use a clean account). | Harness list can be empty; Continue still works; step 2 explains install-later; no command is executed. |
| R11.3 | Notifications not determined | Open **Settings → Permissions** and click **Allow…** for Notifications. | macOS asks for permission once; allow and deny both return to the app without a dead UI. |
| R11.4 | Notifications denied | Choose Deny, then review **Settings → Permissions**. | State reads Denied; **Open Settings** opens the notification pane; no permission is granted by Dunlin. |
| R11.5 | Notification recovery | Enable notifications in System Settings, return and click **Recheck**. | State becomes Granted; enabling the Dunlin toggle produces a test notification when the app is inactive. |
| R11.6 | TCC center | Review Notifications, Accessibility, Screen Recording, Automation and Full Disk Access. | States and explanations are present; links open the matching panes; no prompt is triggered just by Recheck; Automation stays Unknown until a real prompt. |
| R11.7 | Project trust | Create a local project, then use the project menu to **Untrust project**. | Chat shows the trust banner; harness/runtimes stop; Trust project restores the normal state without losing history. |
| R11.8 | Database upgrade | Copy a pre-beta.5 data directory, launch the new bundle, open the QA report. | Schema version advances; `quick_check` is `ok`; projects, sessions and transcripts are preserved; no migration error. |
| R11.9 | Large project | Open a large repository; inspect Explorer sizes, branch/status and a long imported chat. | Explorer stays responsive; sizes are bounded; chat opens with the latest window and **Show earlier messages**; no hang. |
| R11.10 | Remote beta | Pair a disposable relay host; create a remote session, send a prompt, open Dashboard and Usage. | Remote session works; Dashboard refreshes; imported/billing data is read-only; member sees Admin-only usage without a request/error. |
| R11.11 | Keyboard/focus/a11y | Traverse onboarding, Settings, project menu, chat composer, Graph (arrows/Home/End/PageUp/PageDown/Left/Right) and Trace (rows, filters, Requests) with Tab/Shift-Tab/Esc/Enter; check both themes and the narrow window. | Focus is visible and restored; graph/trace selection scrolls into view and never points at a filtered row; tree levels/states are announced; no trap; labels are meaningful; Reduced Motion is respected; text remains readable. See [graph-trace-virtualization.md](graph-trace-virtualization.md) for the full matrix. |
| R11.12 | IME/bidi | Type composed text in the composer and Markdown editor; inspect an RTL sample. | Composition commits once; selection/caret and direction remain usable; no lost text. |
| R11.13 | Remote/GitHub boundary | Repeat the read-only Git/GitHub checks against the disposable host. | Read-only screens work; destructive operations require explicit confirmation; no credentials are printed. |

R11 is **not complete** until every required row has evidence and the release
owner signs off. A green `release_qa.py` run only proves the automatic subset.
