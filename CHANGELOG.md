# Changelog

All notable changes to Dunlin are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- OpenCode provider/account controls, including OpenCode Go inside the OpenCode
  harness, official Terminal sign-in, and provider/model refresh without a new
  conversation. Credentials stay in OpenCode.
- Provider-filtered enabled models and three persistent, deduplicated recent
  models per provider/harness/environment. Prevent sending through a different
  billing provider than the selected filter, preserving the composer text.
- VS Code-inspired chat composer with inline environment, harness, mode/approval
  and model dropdowns; centered layout for a new conversation.
- ACP agent-mode discovery and switching through config options or
  `session/set_mode`, including live mode updates.
- Standalone-chat repository-context picker specified in the roadmap (not yet
  implemented and intentionally separate from project sessions).
- Model picker populated by the connected ACP harness, supporting session
  config options and the legacy model-selection extension.
- A remove action in each project's sidebar menu, including the last project.
- Static marketing site in `web/` with a native product showcase, a download
  page, privacy and terms pages, and a generated social card.

### Fixed

- Fix a trace-bar rounding panic at the end of the timeline, including the
  startup crash when restoring the Trace view. Keep existing settings/history.
- Send chat messages with Enter; Shift+Enter inserts a newline. Keep `⌘Enter`
  as an alternative send shortcut.
- Ask to save unsaved edits before switching projects or branches, removing the
  active project, closing the window or quitting with the app menu / `⌘Q`.
- Resume existing local ACP sessions instead of displaying old history over a
  fresh agent context. Preserve history and report unsupported or failed loads.
- Keep session identifiers synchronized across project switches and restore the
  selected conversation before connecting on startup.
- Surface local database read/write failures in a persistent warning instead of
  silently treating them as successful saves or empty history.

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
