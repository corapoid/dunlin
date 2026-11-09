# Test quality and coverage

## What a green check means

- `cargo test --workspace --locked`: Rust unit and integration assertions. These
  do not automatically execute native UI fixtures.
- `python3 scripts/release_qa.py --native-only`: every registered native UI
  scenario, including shared controls, settings, workspace interactions, project
  and session import, Git, Markdown, concurrent conversations and stalled saves.
  Git and workbench also run in light mode at the supported 1100 px minimum
  window width; Markdown retains its additional sub-minimum-width stress case.
  Settings, controls and the shared surface-paint gallery cover their own
  theme/density matrices. See [rounded surface rules](ui-surfaces.md).
- The native suite runs inside `macos-checks`, so failure blocks the existing
  required `test` gate. No missing-desktop condition is silently skipped. Hosted
  WindowServer behavior still needs verification in the first Actions run.
- The separate Coverage workflow measures Rust unit/integration coverage for the
  entire workspace, including desktop. It uploads JSON, HTML and per-package
  Markdown summaries. It is not a newly configured branch-protection requirement.

## Local native checks

```sh
cargo build -p dunlin -p dunlin-agent --bin dunlin --bin mock_acp_agent --locked
python3 scripts/release_qa.py --native-only --tmp-root "$TMPDIR"
```

The runner asks the built debug binary for `--list-native-smokes`; there is no
second hardcoded scenario list to go stale. Unknown/release-unavailable flags
fail before GPUI, logging or storage initialization.
The runner also invokes removed/unknown/malformed CLI modes against that actual
binary, requiring exit code 2 and no application data/log initialization.
Fixtures have disposable
HOME, data/config/cache directories and no inherited provider keys, SSH agent,
autoprompt or harness override. Real gh/glab are masked because their credentials
can live in the OS Keychain rather than HOME. Import fixtures supply fake CLIs.
This is process/environment isolation, not an OS-level network sandbox.
Native fixtures use real AppKit windows; the Markdown copy/cut/paste checks also
use the shared system clipboard. Use a disposable macOS login for unattended
local GUI acceptance when the current clipboard/window focus must be preserved.

Each scenario must exit zero **and** emit its final `PASS:` marker. Closing a
window before assertions finish is a failure. Logs and `report.json` are retained
in a new `dunlin-release-qa-*` directory; CI uploads them even after failure.
The runner never automatically invokes the live Keychain or governance-WASM
checks: those require separate explicit setup/consent and are not GUI fixtures.

## Measured coverage

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.9.1 --locked
mkdir -p target/coverage
cargo llvm-cov --workspace --locked --json --output-path target/coverage/workspace.json --ignore-filename-regex '(^|/)(vendor|target)/|(^|/)([^/]*_smoke|smoke)\.rs$'
python3 scripts/coverage_summary.py target/coverage/workspace.json --require-desktop
cargo llvm-cov report --html --output-dir target/coverage --ignore-filename-regex '(^|/)(vendor|target)/|(^|/)([^/]*_smoke|smoke)\.rs$'
```

Coverage uses a separate instrumented build and can consume considerable disk.
Its percentages cover executed code, not the correctness of assertions. Dedicated
tests, native smoke fixtures, vendor and generated build output are excluded;
inline `#[cfg(test)]` modules and development helper binaries (such as mock ACP)
can still contribute to source-file totals.
Native fixture execution is a separate gate and is **not** merged into this
unit/integration percentage. Functions and regions are not branch coverage.

For actual branch coverage, manually dispatch Coverage with `branches=true`.
That run uses nightly Rust and `--branch`; stable PR runs explicitly report
branches as unmeasured rather than zero percent.

### First measured baseline — 2026-11-09

The full local instrumented workspace run passed, with vendor and native fixtures
excluded as above. This was a dirty working-tree snapshot, not release evidence:

| Scope | Measured lines |
| --- | ---: |
| Entire Rust workspace | 39.92% (47,816 / 119,780) |
| Desktop | 23.55% (19,078 / 81,008) |
| Shared UI crate | 35.56% |
| Storage | 95.42% |
| Workspace/Git/files | 88.81% |
| SSH | 21.88% |

The largest untested areas were app/controllers, settings, overlays/sidebar,
Spec, Security and Knowledge. The new coverage job has initial regression floors
of **39% workspace / 23% desktop**, rounding down this measured baseline; these
are a regression alarm, not quality targets or proof of complete UI coverage.
Changes to the floors require reviewed measurement, not an exception added to
make a failing run green. Inline test code still inflates these source-file
metrics, as disclosed above. Native scenarios remain a separate gate.

## Assertion strength

Tests should call production code, not a test-only copy of its implementation.
The SSH parser tests now use the parser called by `resolve()`, including whitespace,
invalid ports, missing fields and ProxyJump cases. To probe assertion strength:

```sh
cargo install cargo-mutants --version 27.1.0 --locked
cargo mutants --package dunlin-ssh --file crates/ssh/src/service.rs --timeout 120
```

Inspect caught, missed, timed-out and unviable mutants separately. A timeout or
compilation error is not proof that an assertion caught the behavior change.
This targeted experiment is not a workspace-wide mutation score.

The parser-only experiment (`--re parse_ssh_config`) caught eight behavior
mutations; one additional mutant was unviable. No missed or timed-out parser
mutants were reported. This does not imply that SSH connection/bootstrap paths
are fully covered.

## UI coverage ledger

| Surface | Automated evidence | Remaining boundary |
| --- | --- | --- |
| Shared controls | Real click/key events, focus, disabled/loading, tab-close propagation, theme/density geometry | Each application's action still needs its own wiring assertion |
| Rounded surfaces | Central geometry and literal-radius guard; native curve-grid checks for fills/borders, tiny capsule segments, hover/focus/expanded states and a negative control; settings shell/content and Graph surfaces | Image/text rasterization, native material appearance and real multi-display acceptance |
| Settings | Every page/theme/density/size rendered and checked; keep/discard, keyboard search/theme, persisted theme restored by a fresh app instance | Not every preference has a full restart/effect test |
| Session deletion | Real Cancel/Delete clicks, unchanged history after Cancel, persisted removal after Delete | Manual VoiceOver/focus acceptance |
| Save/conflicts | Installed Save All keyboard binding, stalled HTTP, concurrent typing, delayed stat and external-edit protection | Deployed relay/OS failure acceptance |
| Project/session import | Synthetic local histories and fake hosted CLIs, cancellation and stale results | Real hosted account authentication |
| Graph/Trace | Layout/filter/selection/virtualization invariants; real Home/End/Escape keys, offscreen reveal, filtered selection and search-focus protection | Full disclosure/filter-control matrix and VoiceOver |
| Chat/workbench | Mock ACP, independent drafts/transcripts, background agents and permissions/budgets | Full send/queue/retry/cancel control matrix and real providers |
| Knowledge/Security/Spec/SSH | Selected logic/API tests | Complete panel action workflows and real-host integration |

Source-code UI contract tests remain architectural guardrails, not substitutes for
these behavioral scenarios. System TCC prompts, IME/bidi, visual acceptance and
VoiceOver still require the evidence in [R11 QA](r11-qa.md). A green automatic
report must never be described as complete interface acceptance.
