# Security audits

The Security workspace launches explicit audit workflows and displays their
artifacts. It is **not a bundled scanner or proof that a project is secure**.
Use **Settings → Security Audits** for configuration, installation and external
scanner setup; use **Project workspace → Security Audit** to run an audit and
review results for the active project.

## Start and review

Install the recommended pinned Cloudflare `security-audit-skill` bundle globally
once, shared across supported harnesses. Companion files, schema and license are
installed together. The workspace entry appears only while the skill is installed
and enabled (and Security tools are enabled).

In the project audit tab, choose a harness and model. The initial selection is the
default harness and that harness's last dispatched model. **Load / refresh harness
models** discovers choices through an empty ACP session without sending a prompt
or changing the default agent. Model IDs can also be entered explicitly.
**Run audit** creates a dedicated conversation, connects the selected harness,
selects the exact advertised model, and sends the complete source-only skill
workflow. An unavailable model is an error, never an implicit replacement. The
tab stays open and displays artifact-backed runs, findings, coverage and reports.
The form and results use the full document width. **Refresh results** appears
only in the top header once a report exists; completion of a turn refreshes the
open audit workspace automatically.
Normal harness permission prompts still apply; this does not sandbox the harness.

The legacy **Start audit in chat** action in configuration still only prepares
a Phase 1 prompt for manual dispatch. Continue actions likewise prepare prompts.
For remote projects the relay owns the harness; the model must be advertised by it.

Local run artifacts live under
`~/Library/Application Support/Dunlin/audits/<project>/run-<id>/` by default.
Supported readers handle Cloudflare findings/coverage ledgers, Codex Security
findings/coverage and its SARIF 2.1.0 results. Unsupported scanner formats are not
silently interpreted. Raw artifacts remain authoritative.

Findings show source, evidence, reported/confirmed distinctions, coverage and
structural errors. Empty findings, missing coverage or successful parsing never
mean a security pass. Local triage (open, accepted, false positive, fixed, won't
fix) does not alter the original severity/verdict. A verifier writes a separate
record; its decision is not automatically applied to findings.

Continue actions prepare the next phase prompt without sending it. SARIF, JSON,
CSV and Markdown exports do not rewrite canonical findings.

## External commands and sandbox limits

Codex Security is external. An explicitly detected `codex-security`/`cs` CLI owns
its authentication, provider policy and sandbox. Dunlin confirms argv and reads
artifacts back; it does not implicitly use `npx` or repository-local binaries.
Patching requires separate confirmation and a clean worktree; no commit/push is
created automatically.

A separate sandboxed local check can run explicit JSON argv under macOS seatbelt
with network denied, restricted writes/credential reads and bounded resources.
This does **not** sandbox the ordinary ACP agent or turn all workbench checks
into sandboxed commands.

## Remote and disable behavior

Relay projects store audit runs in `.dunlin/audits/<run-id>/` on the host. Keep
`.dunlin/` out of Git history. Bounded granted artifact APIs populate a read-only
client cache. Remote verifier writes and remote Codex CLI execution have separate
limitations; local availability is not remote availability.

Disabling the Security workspace hides its surfaces/catalog recommendations, not
installed skills, saved run artifacts or existing permissions. Real-host acceptance
is tracked in the [roadmap](../project/roadmap.md); repository vulnerabilities must
be reported through [SECURITY.md](../../SECURITY.md), not a public issue.

Sources: [Security UI](../../apps/desktop/src/security.rs),
[audit parsers](../../crates/audit/src), [relay](../../crates/relay/src/api.rs).
