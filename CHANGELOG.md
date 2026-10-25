# Changelog

All notable changes to Dunlin are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Static marketing site in `web/` with a native product showcase, a download
  page, privacy and terms pages, and a generated social card.

## [0.1.0-beta.1] — 2026-10-25

The first public macOS beta. Dunlin is an early, opinionated release: local
projects are usable today, the remote relay is a beta, and several planned
surfaces are intentionally absent.

### Added

- Native Rust/GPUI workspace with a real Agent Client Protocol client.
- Harness support for OpenCode (default), Claude Code, Codex and custom ACP
  commands, with per-project overrides.
- Streaming chat with thoughts, plans, tool calls, usage updates, permission
  cards and unified diff previews.
- Prompt queue with edit, remove, reorder and clear; message copy, retry and
  edit.
- File explorer, syntax-highlighted editor, save, autosave, external-change
  detection and conflict recovery.
- Project search and source control backed by the system `git` CLI: status,
  staging, discard, commit, amend, pull, push, branches and read-only diffs.
- Agent graph and trace waterfall projected from the live transcript.
- Session history, transcript snapshots, project groups and per-project
  settings.
- Self-hosted remote relay with pairing, device tokens in Keychain, project
  grants, session leasing, takeover and transcript snapshots.
- macOS notifications, Dock badge, missing-folder recovery and a debounced
  file watcher.
- First-run onboarding, empty states and a **Show keyboard shortcuts**
  setting.
- CI and a tag-driven release workflow producing an arm64 ZIP, DMG, relay
  tarball and SHA-256 checksums.
- MIT license, third-party notices and community documentation.

### Known limitations

- macOS 15+ on Apple Silicon only.
- Builds are ad-hoc signed until an Apple Developer ID is available. Gatekeeper
  may require approval in **System Settings → Privacy & Security**.
- The iOS client is a scaffold and is not part of this release.
- No terminal, computer use, automations, skills, artifacts or visual Markdown
  editor yet.
- All relay users share the OpenCode credentials installed on the relay host.
- The relay database is not encrypted at rest; use FileVault or an encrypted
  volume.

[0.1.0-beta.1]: https://github.com/corapoid/dunlin/releases/tag/v0.1.0-beta.1
