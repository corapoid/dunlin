# CodeQL Rust triage — 2026-11-01

The first full migrated-source Rust scan was
[Security 41326160513](https://github.com/corapoid/dunlin/actions/runs/41326160513).
Its analysis `2072159507` reported 21 open alerts. A successful analysis job does
not mean its findings passed: the aggregate CodeQL check correctly failed.

## Source fixes (15 alerts)

- **#3–#11, `rust/access-invalid-pointer`**: rav1e's optional C API tests used
  nullable context/packet/stats results without checking them. Add explicit
  constructor/output assertions, fail-closed null guards in the affected context
  entry points and a null-input regression test. Non-null C handles still require
  valid caller-owned allocations; null checks cannot validate dangling pointers.
  This C API is not enabled in Dunlin, but its sources remain scanned.
  A repeat scan found three further frame-entry null paths (#24–#26); guard
  frame/buffer inputs too. Use checked `as_ref().expect()` for test packet/stats
  views instead of raw dereferences after assertions. Null inputs are tested,
  while missing successful outputs still fail the test, never silently skip it.
  The next scan (#27) also motivated local validation of decoder input slots:
  guard null data/length pointers, missing buffers and oversized lengths before
  dereferencing or constructing slices. Check rate-control prefix arithmetic and
  C integer range before mutation; adversarial decoder tests cover these cases.
- **#12–#15, `rust/cleartext-logging`**: smoke binaries no longer print raw harness
  session identifiers; they log only static created/loaded events.
- **#22–#23, `rust/log-injection`**: relay operational logs no longer include
  untrusted user/project names or the raw harness startup error/configuration.
  Method diagnostics use Rust Debug escaping to prevent newline/control injection.

## Individually reviewed false positives (6 alerts; no query/path exclusion)

### #16–#20 — OAuth identifier attributed to numeric pane geometry

The SARIF flow begins at `apps/desktop/src/provider_auth.rs`, `#[derive(Clone)]`
on `MethodKind`. `OAuth(String)` stores an authentication **method identifier**
from discovery (`method["id"]`), not an access/refresh token. More importantly,
the flow then incorrectly resolves this enum's derived `Clone` as
`Size<T>::along()`'s `self.width.clone()` in `vendor/gpui-pre-0.3.9/src/geometry.rs`.
The actual resizable state is `Bounds<Pixels>` / `Vec<Pixels>`; Rust cannot put
`MethodKind` into `Pixels` or call its clone there. Its reported sinks are
`Vec::insert`/`Vec::remove` of numeric sizes/indices and pane node IDs, not log
statements. There is neither an auth-data path nor a logging operation.

These five exact alerts are classified **false positive**, not accepted risk.
Do not rename OAuth types, alter geometry logic, suppress the logging query or
exclude vendor Rust to camouflage the extractor's impossible trait resolution.

### #21 — one filename after Git's option terminator, not a command line

The flow is the relay diff query's `file` into `file_diff()`, then
`git(path, &["ls-files", "--error-unmatch", "--", file])`. The helper uses
`std::process::Command::new("git").args(args)` with a constant executable and
separate argv entries; it never invokes a shell. This call's options are constant
and the only untrusted value is one operand **after `--`**. Absolute/parent paths
are rejected and untracked reads use the confined filesystem resolver. The same
terminator protects both staged/unstaged diff calls. Therefore shell characters
or leading dashes cannot become commands/options in this reported flow.

Separately harden the helper with Git's `--literal-pathspecs`: an in-project file
named `:(glob)*` must not select other files. A real-repository regression checks
that name, a leading `--output=INJECTED`, shell-substitution/semicolon filenames,
single-file diff scope, no injected side-effect and parent-path rejection.
The command-injection alert itself is **false positive**; literal pathspec
handling is additional input-scope hardening, not evidence of shell execution.

No RustSec advisory exceptions, CodeQL query suppressions, blanket dismissals or
vendor Rust exclusions are used. Each false-positive classification records this
specific proof in GitHub's alert audit trail. All actual findings must disappear
from a fresh scan; no signed merge is authorized while open security alerts remain.
