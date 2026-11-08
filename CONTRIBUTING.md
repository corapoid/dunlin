# Contributing to Dunlin

Thanks for taking the time to contribute. Dunlin is an early native macOS IDE;
small, focused changes are easiest to review and land.

## Before you start

- Search existing issues and discussions before opening a new one.
- For a large feature or an architectural change, open an issue first so we can
  agree on the direction.
- Report security issues privately as described in [`SECURITY.md`](SECURITY.md),
  never in a public issue.

## Development setup

Requirements:

- macOS 15 or newer on Apple Silicon
- Rust 1.92 or newer (`rustup` recommended)
- `git` on `PATH`
- An ACP harness for manual testing, for example `opencode acp`

Build and run:

```sh
cargo run -p dunlin
```

The first launch opens an onboarding screen. Use **Open folder…** to add a
project. Test data can be redirected with `DUNLIN_DATA_DIR`:

```sh
DUNLIN_DATA_DIR=/tmp/dunlin-dev cargo run -p dunlin
```

## Checks

Run the same checks as CI before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
python3 -m unittest discover -s scripts/tests -v
./scripts/relay-smoke.sh
python3 scripts/git-operations-smoke.py
```

To build a local app bundle:

```sh
./scripts/bundle-macos.sh --build
open dist/Dunlin.app
```

The bundle is ad-hoc signed by default. Set `DUNLIN_CODESIGN_IDENTITY` to use a
Developer ID certificate.

## Code style

- Follow the existing structure and naming. The UI is GPUI, not a webview;
  keep rendering work off blocking threads.
- Prefer the shared types in `crates/domain` and the persistence layer in
  `crates/storage` over ad-hoc models.
- Keep ACP protocol I/O on the agent worker and send events to the UI over
  channels.
- Add tests for behavior that can regress silently, especially storage
  migrations, workspace operations and relay authorization.
- Do not add a dependency for something the standard library or an existing
  crate already does well. New dependencies need a reason in the pull request.

## Dependency updates

Dependency updates are maintained manually; Dependabot version/security update
pull requests and bot auto-merge are disabled. Review updates carefully because
many Rust dependencies are still on `0.x`, where a minor bump can be breaking.

Security alerts, dependency review and the CI/security gates remain enabled.
Weekly and manually dispatched Security runs check the shipped dependency graph
for newly disclosed vulnerabilities. Treat security fixes as urgent and submit
them through the same protected pull-request process as other changes.

## Commits and pull requests

- Keep commits focused and write imperative subject lines, for example
  `Add project search filters`.
- Describe the user-visible change, how you tested it, and any trade-offs in
  the pull request.
- Include screenshots or a short recording for UI changes.
- Update `docs/ROADMAP.md` or `CHANGELOG.md` when a change affects the release
  scope.
- By contributing, you agree that your contribution is licensed under the MIT
  License.

## Release process

Releases are cut from an existing `v*` tag matching the workspace, lockfile and
changelog, or manually dispatched against that same tagged commit. CI and Security
run on pull requests, not again on pushes to `main`. Security also runs weekly
and on manual dispatch to detect newly disclosed vulnerabilities on `main`.

Release authorizes the GitHub-signed squash merge, requires its tree to match
the reviewed PR head exactly, and checks the PR's successful CI/security results,
including both Linux architectures, plus open CodeQL alerts. It does not rerun
formatting, lint, unit/integration tests or security scans. It builds the release
on macOS and Linux, signs/packages/verifies the artifacts, attests their provenance
and publishes only after those jobs pass. Artifact verification and release-binary
smokes remain in CD. Merging alone does not publish a release.

Beta tags produce a prerelease, not a draft. Missing
Apple credentials produce explicitly ad-hoc builds; partial credentials fail.
Developer ID signing alone must never be described as notarization.

Release artifacts include a ZIP, a DMG, an unsigned `.pkg` installer for
`/Applications` and the relay tarball. The Homebrew beta cask lives in
[`corapoid/homebrew-tap`](https://github.com/corapoid/homebrew-tap) and must be
advanced to the same verified DMG and SHA-256 after each beta publication.
`./scripts/bundle-pkg.sh` builds the installer from the already-signed app;
`DUNLIN_PKG_SIGN_IDENTITY` enables Developer ID Installer signing when
available. The release workflow also calls `.github/workflows/package.yml`,
which verifies the published assets and pushes them to GHCR as
`ghcr.io/corapoid/dunlin:<version>` plus the `beta` (or `latest`) tag.

For isolated automatic QA use `python3 scripts/release_qa.py --tmp-root "$TMPDIR"
--native --audit` after installing cargo-deny 0.20.2. Native mode needs a desktop;
it never substitutes for manual GUI/TCC/deployed-host acceptance. The current
dependency and repository-policy audits have unresolved findings; see
[release acceptance](docs/release-acceptance.md) before cutting another tag.

## Code of conduct

Be direct, be kind, and assume good intent. Harassment, discrimination and
personal attacks are not welcome. Maintainers may remove comments or close
threads that make the project worse for other contributors.
