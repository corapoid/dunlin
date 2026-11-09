# Release process and acceptance

Reviewed against repository policy/workflows on **2026-11-09**. This is a
procedure/status boundary, not evidence that a release was performed.

## Current distribution

- Desktop is beta software; source builds are recommended.
- Desktop DMG/PKG/ZIP publishing, Homebrew advancement and OCI/GHCR publication
  remain paused pending Apple Developer ID signing and notarization.
- Releases distribute macOS ARM64 and Linux x86_64/ARM64 **relay** archives,
  bootstrap scripts and `RELAY-SHA256SUMS`.
- An ad-hoc signature is not Developer ID signing; Developer ID signing alone is
  not notarization. Missing/partial signing credentials have explicit workflow
  behavior and must not be described as a notarized build.

## Before authorizing a version

1. Agree release scope and inspect the exact source/working tree. A documentation
   change does not authorize commits, tags, publication or deployment.
2. Run required formatting/lint/tests, native fixtures and security checks on the
   reviewed PR. Preserve logs for failed/skipped/cancelled gates rather than
   treating them as green. See [CI/CD](../development/ci-cd.md).
3. Complete or explicitly decide the applicable [manual matrix](manual-acceptance.md),
   with actual evidence for GUI/TCC/provider/deployed-host cases. Automated mock
   success does not pass manual acceptance.
4. Match workspace, workspace lockfile packages, changelog and requested SemVer
   version. Validate metadata with the release tooling.
5. Check the applicable acceptance policy and unresolved dependency/repository
   audit findings. Do not copy a previous release's green counts or policy waiver.

**Current blocker:** the workspace targets `0.2.0`, while
[release-policy.json](../../.github/release-policy.json) only records a manual
deferral for `0.1.0-beta.N`. The fail-closed
[acceptance policy check](../../scripts/release_gate.py) does not accept `0.2.0`
under that exception. A new explicit acceptance decision and supported policy
must precede publication; do not relax it as part of documentation work.

## Source authorization and publication

The release workflow starts from an existing `v*` tag matching metadata, or manual
dispatch against that same tagged commit. A merge alone does not publish.
Source authorization checks the GitHub-verified squash merge, exact reviewed PR
tree and trusted successful PR CI/security results, including both Linux relay
architectures and CodeQL state.

Platform release builds wait for source authorization. CD builds/verifies relay
artifacts, applies configured signing/provenance steps, validates transferred
artifacts and publishes only after required jobs pass. It does not replace PR
tests by rerunning the whole test/security suite in CD. Regular releases can be
marked Latest even for a beta suffix; that is not manual acceptance/notarization.

After publication, independently inspect the intended release assets/checksums,
architectures, version and attestations before updating distribution links.
Verification alone must not install/run the app, restart a service or deploy the
website without separate authorization.

## Reproduction and sources

```sh
python3 scripts/release_gate.py metadata --version "$(python3 scripts/release_gate.py version)"
python3 -m unittest discover -s scripts/tests -v
```

Full developer/native/audit recipes are in [CONTRIBUTING.md](../../CONTRIBUTING.md)
and [test quality](test-quality.md). They are commands to run, not current pass claims.

- [Release workflow](../../.github/workflows/release.yml)
- [Source authorization](../../scripts/release_source.py)
- [Metadata/acceptance verifier](../../scripts/release_gate.py)
- [Relay artifact verifier](../../scripts/verify-relay-release.py)
- [Historical release evidence](../archive/release-acceptance.md) and
  [dated CodeQL triage](../archive/codeql-triage.md)
