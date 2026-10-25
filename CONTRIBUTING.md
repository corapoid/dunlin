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
./scripts/relay-smoke.sh
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

Dependabot is enabled for Cargo and GitHub Actions and runs weekly on Monday at
06:00 Europe/Warsaw. Patch updates are grouped by area; minor and major updates
stay as separate pull requests because many Rust dependencies are still on
`0.x`, where a minor bump can be breaking.

`test` and `dependency-review` are required checks on `main`. Dependabot
auto-merges GitHub Actions minor/patch updates and Cargo patch updates once
those checks pass. Security updates are enabled independently; treat them as
urgent and review them even when CI is green.

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

Releases are cut from a `v*` tag. The workflow builds on macOS, packages the
app and relay, and creates a draft GitHub Release. The first beta is unsigned.
Once an Apple Developer ID is configured, add the signing and notarization
secrets described in `.github/workflows/release.yml`.

## Code of conduct

Be direct, be kind, and assume good intent. Harassment, discrimination and
personal attacks are not welcome. Maintainers may remove comments or close
threads that make the project worse for other contributors.
