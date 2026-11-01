# Release acceptance / hardening — 2026-11-01

## Status: automatic beta hardening accepted; manual matrix explicitly deferred

The already published **0.1.0-beta.3** is distinct from the **unreleased hardening
changes** described here. Never retag beta.3 or replace its assets to imply that
new gates ran for that release. A new tag must be approved separately.

### Final implementation acceptance

- [PR #6](https://github.com/corapoid/dunlin/pull/6) merged as
  `f61251f50c24b07b35669ad1a48fe998a8b2c81c`. GitHub commit verification is
  **valid**; its tree exactly equals the final green PR head `5ccce78`.
- Normal exact-head REST squash enforced the repository rules; rule suite
  `4844202002` reports **pass**, not bypass. No admin merge, bypass actor,
  policy relaxation or personal signing-key impersonation was used.
- Final PR [CI](https://github.com/corapoid/dunlin/actions/runs/41332004707),
  [Security](https://github.com/corapoid/dunlin/actions/runs/41332004701), dependency
  review and aggregate CodeQL passed. All eight required checks are green.
- Signed implementation-source [main CI](https://github.com/corapoid/dunlin/actions/runs/41333497656)
  and [main Security](https://github.com/corapoid/dunlin/actions/runs/41333497547)
  passed. **Zero open CodeQL security alerts** on main. Six individually proven
  false positives retain their audit trail; actual findings were fixed, not waived.
- [Read-only source verification](https://github.com/corapoid/dunlin/actions/runs/41335759337)
  passed using the workflow's limited `GITHUB_TOKEN`, not an owner credential.
  `release_source.py` authorized the exact signed implementation SHA.
- Downloaded main CI app/relay without installing or executing them. Both binary
  attestations verify against `corapoid/dunlin/.github/workflows/ci.yml`,
  `refs/heads/main`, exact source digest `f61251f…`, and GitHub-hosted runners.
  SHA-256: app `23cc9013125b4cd3fe7429dd1e036920c08f6a1ba5e6105b9ac5d78dabdcfd3c`;
  relay `e6a059d17ef75f42812a8e0b1916fee87fdc5866fec707999af72ad7ca9eefdd`.
  These are **CI binaries**, not a new beta release or retroactive beta.3 attestations.
- Clean signed-source local report `dunlin-release-qa-sdjuyysk/report.json`:
  `source_dirty=false`, all **18 automatic checks pass**, 491 Rust tests pass /
  4 optional ignored and 25 Python tests pass; all nine native fixtures pass.
  Five additional isolated rav1e C API tests pass. Vendor integrity and
  dependency audit pass without RustSec advisory exceptions.
- Manual GUI/TCC/real-host acceptance remains **deferred, not passed**, only for
  `0.1.0-beta.N`. Stable/RC acceptance and Apple Developer ID/notarization remain
  separate prerequisites. No app/relay deployment, website deployment, new tag,
  release publication or replacement of beta.3 artifacts was performed.

The sequence below preserves historical failures and remediation evidence; its
earlier pending states are superseded by the final implementation acceptance above.

### Confirmed published beta.3 evidence

- Source/tag: `36398df78972930dd261ce5442a8d8ab7a76210d` / `v0.1.0-beta.3`.
- [CI](https://github.com/corapoid/dunlin/actions/runs/41291827294) and
  [Release](https://github.com/corapoid/dunlin/actions/runs/41291827251): **success**.
- [Prerelease](https://github.com/corapoid/dunlin/releases/tag/v0.1.0-beta.3)
  published `2026-11-01T09:19:50Z`, not a draft.
- Downloaded the ZIP, DMG, relay tarball and SHA256SUMS to disposable storage.
  All bytes match both the manifest and GitHub's asset SHA-256 digests.
- App Info.plist version/identity/macOS baseline and thin arm64 app/relay headers
  match the release. `codesign --verify --deep --strict` passes for the extracted
  app; relay signature verification passes. `hdiutil verify` passes for the DMG.
- The app is **ad-hoc signed**, not an Apple-notarized Developer ID release.
  Inspection did not run/install the downloaded binaries, mount the DMG, replace
  the installed app/relay or use the user's session database.

### Implemented for subsequent releases

- `scripts/release_gate.py`: reject injected/invalid versions, stale workspace/
  lockfile/changelog metadata, incomplete manifests, altered bytes, wrong bundle
  identity/architecture, archive traversal/symlinks, and unaccepted notary reports.
  Explicit signing states: ad-hoc / Developer ID / notarized; partial credentials
  fail closed and credential values are not echoed.
- Release workflow: exact existing tag/commit validation, tests before packaging,
  reusable security gate, signing secrets only in relevant steps, app **and relay**
  signing. Notarize/staple the app **before** ZIP/DMG creation, notarize/staple the
  final DMG, and compute hashes only after the final bytes are ready. An accepted
  Apple JSON result is required for every notarization submission.
- Separate read-only build and write-capable publish jobs; recheck transferred
  artifacts, require the tag already to exist, include actual changelog notes,
  and generate GitHub artifact-provenance attestations. Attestations are not
  substitutes for Apple notarization or signed developer commits/tags.
- `.github/workflows/security.yml`: cargo-deny 0.20.2 and CodeQL security-extended
  for Rust, JavaScript/TypeScript, Python and Actions, including scheduled scans.
  Rust's no-build CodeQL analysis does not claim compiler/generated-code completeness.
- `deny.toml`: shipped macOS arm64 graph, license/source/wildcard checks and no
  blanket advisory exceptions. Local workspace path dependencies are allowed;
  duplicate transitive major versions remain visible warnings.
- `scripts/repository_policy.py`: read-only verification of default-branch review,
  signatures, bypasses, required CI/security checks and secret scanning. It does
  **not** silently change GitHub settings, invite reviewers or create signing keys.
- `scripts/release_qa.py`: isolated automatic regression runner with machine-readable
  per-check exit codes/logs, process-group timeouts and opt-in native fixtures.
  Manual/deployed-host acceptance is explicitly excluded from its pass flag.
- Explicit CODEOWNERS entries cover agent/network/auth/storage/Git/release paths.
- Website source links now point to the verified published beta.3 artifacts;
  page titles and URLs share one version constant. This is not a website deployment.

### Local validation of the hardening

- 491 Rust tests passed / 4 optional ignored; 25 Python release/policy tests passed.
- Formatting, strict Clippy, desktop/relay/mock builds and `actionlint` passed.
- Relay smoke and paired Git HTTP smoke passed. All nine native fixture runs
  passed: Git/Markdown/GitHub/workbench in dark and light-narrow, plus glass.
- The glass smoke initially failed on stale background geometry; it now explicitly
  draws both fixture windows before assertions. No layout/focus check was removed.
- A bundle built in a new disposable output directory verifies with strict
  codesign; invalid app names fail before cleanup. No installed app was replaced.
- Web format/check/build passed using an existing supported Node runtime.
- The migrated-source automatic QA report `dunlin-release-qa-7i3oxuxt/report.json`
  passed all 17 then-configured checks, including audit and nine native fixtures.
  Vendor integrity passes separately; the current runner includes it as well.
  Earlier failed reports remain historical evidence, not overwritten successes.

### Remediated findings and accepted beta boundary

1. **Dependency audit passes without exceptions.** The user chose migration,
   not exemptions. All five obsolete packages below are absent from the entire
   current lockfile and explicitly banned against reintroduction:

   | Advisory | Dependency path / follow-up |
   | --- | --- |
   | RUSTSEC-2025-0141 | bincode 1.3.3 through syntect / GPUI / two-face |
   | RUSTSEC-2024-0384 | instant 0.1.13 through GPUI |
   | RUSTSEC-2024-0436 | paste 1.0.15 through GPUI / metal / image |
   | RUSTSEC-2026-0206 | rustybuzz 0.20.1 through usvg/resvg / GPUI |
   | RUSTSEC-2026-0192 | ttf-parser 0.25.1 through fontdb/usvg / GPUI |

   Nine local maintained forks replace clocks/macros/font shaping and migrate
   bundled syntax/theme/license dumps to versioned MessagePack. Full upstream
   archive checksums, changed-file records and tree integrity are preserved in
   `vendor/provenance.json`; [migration details](../vendor/README.md). All ten
   assets reproduce byte-for-byte; runtime tests preserve the exact license text.
   No legacy decoder ships and `advisories.ignore` remains empty.
2. **Repository-policy audit passes in explicitly approved solo mode.** Active
   ruleset `25821094` requires PRs, resolved threads, strict eight checks bound
   to GitHub Actions app `15368`, GitHub-verified signatures and squash-only merges.
   There are no bypass actors. Zero independent approvals is the accepted solo
   policy, not a fabricated second-person review. Secret scanning/push protection
   remain enabled. The authorized policy update was backed up before applying.
   `repository_policy.py` audits owner-visible settings separately and fails if
   bypass visibility is missing. Release jobs instead use `release_source.py`
   with read-only APIs: verify an actual signed mainline PR merge, trusted green
   PR-head and merge-source checks, and no open CodeQL security alerts. They do
   not need an owner/admin token or `RELEASE_POLICY_TOKEN`.
3. **Manual R11 acceptance is explicitly deferred, NOT passed.** Fresh macOS account/TCC denial,
   visual/keyboard/a11y/IME/bidi checks, real GitHub write/merge lifecycle and a
   deployed relay cannot be certified by fixture smokes. Computer Use discovery
   failed: `FileNotFoundError: [Errno 2] No such file or directory: 'orca'`.
   No GUI fallback was used to change the user's installed app or permissions.
   A separate agreed test repository/account/host and the manual matrix remain
   necessary for full acceptance; unit/loopback tests must not be relabeled as it.
   The user explicitly accepted deferral only for `0.1.0-beta.N`, recorded in
   `.github/release-policy.json` and enforced by the acceptance gate. Stable/RC
   releases cannot inherit that decision.
4. **Historical workflow/provenance preparation.** Initial PR #6
   CI and dependency review passed; all four CodeQL jobs in
   [Security](https://github.com/corapoid/dunlin/actions/runs/41319994183) passed,
   while its pre-migration dependency audit failed. Two unpinned-action findings
   have local SHA-pin fixes. Fresh scans must validate the migrated sources and
   fixes before normal squash merge; mainline push CI must then produce verified
   binary attestations. These outcomes are not yet claimed here.
   The existing successful beta.3 runs predate this hardening.
   Developer ID/notary execution additionally depends on configured Apple credentials.

### Historical scan and remediation sequence

- Head `8323237`: [CI](https://github.com/corapoid/dunlin/actions/runs/41326160605)
  passed `test` and `web`; dependency review and dependency audit passed. All four
  [CodeQL analysis jobs](https://github.com/corapoid/dunlin/actions/runs/41326160513)
  completed successfully, but the aggregate finding gate correctly **failed**.
- The full Rust scan reported 21 findings: 15 addressed in source (nullable FFI
  handles/results and unsafe log contents), six individually classified as proven
  false positives with exact SARIF/type/argv evidence. No query or Rust-path
  exclusion was added. [Per-alert triage](codeql-triage.md).
- New adversarial real-repository Git tests pass; all four isolated rav1e C API
  tests pass, including null-input checks. The new source fixes require a fresh
  GitHub scan before merge; earlier green job execution does not waive findings.
- Repeat head `836c9fb`: all eight named checks passed again, but the aggregate
  Rust finding gate found three additional null frame paths and two remaining
  raw test views. These now have frame/buffer guards and checked reference views;
  the four C API tests pass again. Local post-remediation QA report
  `dunlin-release-qa-woa1znbm/report.json` passed all 18 checks, including nine native
  fixtures. Final remote revalidation remains required.
- Head `5405a7d` removed those frame/test-view findings; its eight named jobs
  passed, with one remaining decoder-input finding. The decoder now validates
  pointer slots, buffers and length arithmetic locally; all five isolated C API
  tests pass. A further scan must confirm the final remediation before merge.

## Repeatable checks

```sh
python3 -m unittest discover -s scripts/tests -v
python3 scripts/verify_vendor.py
actionlint
cargo install cargo-deny --locked --version 0.20.2
cargo deny --locked check advisories licenses bans sources
python3 scripts/release_qa.py --tmp-root "$TMPDIR" --native --audit
python3 scripts/repository_policy.py --repo corapoid/dunlin
python3 scripts/release_gate.py metadata --version 0.1.0-beta.3
python3 scripts/release_gate.py acceptance --version 0.1.0-beta.3
python3 scripts/release_source.py --repo corapoid/dunlin --commit FULL_SIGNED_MAIN_SHA
python3 scripts/release_gate.py verify --version 0.1.0-beta.3 --directory /path/to/downloads
```

The source authorization must fail until the exact signed merge and its checks
exist. Local integrity checks do not establish real-host
acceptance, Apple notarization or a remote CodeQL result.
