# CI/CD execution and caches

## Pull requests

The CI workflow runs two independent macOS jobs:

- `macos-checks`: formatting, unused Rust dependencies (cargo-machete 0.9.2),
  workflow lint, release-tooling regression tests,
  vendor and metadata verification, Clippy, all workspace tests, and both relay
  smoke suites, followed by every registered native UI fixture. Native failures
  (including a missing interactive desktop) fail the required aggregate gate;
  logs and the machine-readable QA report are uploaded for diagnosis.
- `macos-build`: the production-profile app/helper/relay compilation without
  desktop packaging or app uploads. The standalone relay is copied from that same build, not compiled
  again with a different feature-unification context.

The required `test` check is an aggregate gate. It runs even if a prerequisite
fails or is skipped, and succeeds only when **both** macOS jobs succeed. Existing
branch protection and release source authorization continue to require `test`;
no repository settings need to change.

Linux relay tests/builds, the web checks, dependency review, dependency auditing,
and all four CodeQL analyses remain enabled. No source paths are excluded from
testing or security analysis. PR relay artifacts expire after three days;
release artifacts retain their existing retention policy.

## Reusable Rust caches

The separate [Coverage workflow](../.github/workflows/coverage.yml) instruments
the entire workspace, including desktop, and uploads JSON/HTML plus per-package
line/function/region counts. Stable PR runs do not claim branch coverage; manual
dispatch can request nightly branch instrumentation. Coverage is not yet a
configured required check. Its initial regression floors are 39% workspace and
23% desktop line coverage, derived from the measured local baseline (not a claim
of adequate coverage). See [test quality](test-quality.md)
for commands, scope, and the UI coverage ledger.

`Warm Rust caches` writes dependency caches from `main` only. It runs after
dependency/build-configuration changes, weekly (to pick up stable toolchain
updates), or manually on `main`. It compiles test binaries but does not execute
tests, smoke suites, packaging, signing, or publishing.

Two shared keys separate debug/check artifacts (`macos-checks-v1`) from optimized
artifacts (`macos-release-v1`). PR builds and tagged relay releases both restore the
latter. Rust-cache still adds the OS, architecture, compiler and manifest hashes
to these keys. Workspace code is rebuilt; cached artifacts are not treated as
verified release binaries.

Consumers do not save PR/tag-local caches: those cannot seed unrelated PRs or
future releases. No pull-request code is used to write a main-branch cache, and
the warmer uses read-only repository permissions without persisted credentials.

After this workflow first lands on `main`, wait for `Warm Rust caches` to finish
before measuring warm-cache performance. The first PR introducing the workflow
can still have cold caches. To refresh manually, select `Warm Rust caches` in
GitHub Actions, choose **Run workflow**, and select `main`.

## Releases

The source-authorization job verifies the signed mainline source, trusted green
PR checks and security alerts, then validates version metadata, acceptance policy
and the existing tag. macOS and both Linux builds start in parallel only after
that gate succeeds. Publishing still waits for every platform, revalidates the
transferred artifacts and publishes only the verified tag. Releases distribute
only relay archives, bootstrap scripts and `RELAY-SHA256SUMS`; signing,
notarization and provenance attestations apply to the macOS relay, not the desktop app.

Desktop DMG/PKG/ZIP generation, app artifact uploads and OCI/GHCR publication are
paused until Apple Developer ID signing and notarization are available. The beta
desktop app should be built from a clone of the repository using
`./scripts/bundle-macos.sh --build` locally.

## Local checks

```sh
actionlint
python3 -m unittest discover -s scripts/tests -v
bash -n scripts/bundle-relay.sh
git diff --check
```

Workflow contract tests exercise aggregate-gate failure/cancellation/skip cases,
cache trust boundaries, parallel release dependencies, and relay packaging
without invoking Cargo. Actual runner speed and cache hits must be measured in
GitHub Actions; parallel jobs can reduce elapsed time without reducing total
runner minutes by the same proportion.
