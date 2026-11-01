# Release acceptance / hardening — 2026-11-01

## Status: not all release gates are closed

The already published **0.1.0-beta.3** is distinct from the **unreleased hardening
changes** described here. Never retag beta.3 or replace its assets to imply that
new gates ran for that release. A new tag must be approved separately.

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

- 487 Rust tests passed / 4 optional ignored; 14 Python release/policy tests passed.
- Formatting, strict Clippy, desktop/relay/mock builds and `actionlint` passed.
- Relay smoke and paired Git HTTP smoke passed. All nine native fixture runs
  passed: Git/Markdown/GitHub/workbench in dark and light-narrow, plus glass.
- The glass smoke initially failed on stale background geometry; it now explicitly
  draws both fixture windows before assertions. No layout/focus check was removed.
- A bundle built in a new disposable output directory verifies with strict
  codesign; invalid app names fail before cleanup. No installed app was replaced.
- Web format/check/build passed using an existing supported Node runtime.
- The combined automatic QA report is **failed**, correctly, because the dependency
  audit is red. Repository-policy audit also exits nonzero separately. Do not
  summarize these results as an entirely green release acceptance.

### Blocking findings — not waived

1. **Dependency audit is red**, not a clean security bill. Licenses, sources and
   bans pass; five transitive dependencies have unmaintained RustSec advisories
   with no safe version-only upgrade:

   | Advisory | Dependency path / follow-up |
   | --- | --- |
   | RUSTSEC-2025-0141 | bincode 1.3.3 through syntect / GPUI / two-face |
   | RUSTSEC-2024-0384 | instant 0.1.13 through GPUI |
   | RUSTSEC-2024-0436 | paste 1.0.15 through GPUI / metal / image |
   | RUSTSEC-2026-0206 | rustybuzz 0.20.1 through usvg/resvg / GPUI |
   | RUSTSEC-2026-0192 | ttf-parser 0.25.1 through fontdb/usvg / GPUI |

   These are maintenance advisories, not evidence of five proven exploitable
   vulnerabilities. They must still be resolved through reviewed upstream
   migrations, or an explicitly approved, ID-scoped, justified and expiring
   exception. No exception was inserted merely to turn the gate green. The new
   security prerequisite consequently blocks a subsequent release until resolved.
2. **Repository-policy audit is red.** Existing `main` rules permit an always-bypass
   actor, require zero approvals, do not enforce code-owner/last-push approval or
   signed commits, and do not yet require the new security checks. Secret scanning
   and push protection are enabled. The current owner is the only configured
   CODEOWNER; enforcing independent approval requires a real second trusted
   reviewer and signing setup. Do not invent a reviewer, remove the owner's only
   recovery route, or silently bypass these missing prerequisites.
   The release workflow now requires this audit before building. If its default
   token cannot read security/policy metadata, supply only a read-only
   `RELEASE_POLICY_TOKEN` with metadata/administration-read permission. Lack of
   visibility fails closed; do not give builds admin-write credentials.
3. **Manual R11 acceptance is not completed.** Fresh macOS account/TCC denial,
   visual/keyboard/a11y/IME/bidi checks, real GitHub write/merge lifecycle and a
   deployed relay cannot be certified by fixture smokes. Computer Use discovery
   failed: `FileNotFoundError: [Errno 2] No such file or directory: 'orca'`.
   No GUI fallback was used to change the user's installed app or permissions.
   A separate agreed test repository/account/host and the manual matrix remain
   necessary; unit/loopback tests must not be relabeled as this acceptance.
4. **New workflow execution / provenance is not yet confirmed.** Local workflow
   lint/tests are not GitHub CodeQL scans or artifact attestations. The existing
   successful beta.3 runs predate this hardening; do not attribute new checks to them.
   Developer ID/notary execution additionally depends on configured Apple credentials.

## Repeatable checks

```sh
python3 -m unittest discover -s scripts/tests -v
actionlint
cargo install cargo-deny --locked --version 0.20.2
cargo deny --locked check advisories licenses bans sources
python3 scripts/release_qa.py --tmp-root "$TMPDIR" --native --audit
python3 scripts/repository_policy.py --repo corapoid/dunlin
python3 scripts/release_gate.py metadata --version 0.1.0-beta.3
python3 scripts/release_gate.py verify --version 0.1.0-beta.3 --directory /path/to/downloads
```

The audit commands intentionally exit nonzero for the blockers above. Reports
must preserve those failures. Local integrity checks do not establish real-host
acceptance, Apple notarization or a remote CodeQL result.
