# Installation

Dunlin is beta software. **Build the desktop app from source.** Prebuilt desktop
downloads, Homebrew distribution and OCI/GHCR publication are paused until Apple
Developer ID signing and notarization are available. Relay archives remain a
separate release channel; see [server setup](../administration/relay-setup.md).

## Requirements

- macOS 15 or newer, Apple Silicon. Intel and Linux desktop builds are not supported.
- Rust 1.92 or newer, preferably installed with `rustup`.
- `git` on `PATH` and the macOS development tools needed by Cargo's native dependencies.
- For local agent work: a separately installed ACP adapter and its provider account.
  A remote workspace uses the adapter installed on its relay instead.

## Build a local application

```sh
git clone https://github.com/corapoid/dunlin.git
cd dunlin
./scripts/bundle-macos.sh --build
open dist/Dunlin.app
```

The bundle is **ad-hoc signed, not Apple-notarized**. No Apple Developer ID
certificate is required for this local build. Signing with a Developer ID does
not by itself notarize the app; do not disable Gatekeeper as an installation step.

For development without bundling:

```sh
cargo run -p dunlin
```

The desktop data directory defaults to
`~/Library/Application Support/Dunlin`. A development run can use a separate
directory through `DUNLIN_DATA_DIR`; this is not a replacement for a separate
macOS account when testing Keychain or TCC. See [configuration](../reference/configuration.md).

## Update

Review the [changelog](../../CHANGELOG.md), stop active agents and terminals, close
Dunlin and [back up your data](../administration/data-and-recovery.md) before
building a newer checkout. Database migrations run when storage opens. Downgrading
a migrated database is not a supported recovery procedure.

Next: [your first session](first-session.md). Build failures: [troubleshooting](../administration/troubleshooting.md).
