# Security audits

The Security workspace launches explicit audit workflows and displays their
artifacts. It is **not a bundled scanner or proof that a project is secure**.
Use **Settings → Security Audits** for configuration/project audit actions.

## Start and review

Install the recommended pinned Cloudflare `security-audit-skill` bundle into the
selected harness's skill directory. Companion files, schema and license are
installed together. **Start audit in chat** creates a dedicated conversation and
prepares a source-only Phase 1 prompt; it does not run target code itself.

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
