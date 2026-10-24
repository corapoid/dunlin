# Kestrel

A fast, native, agent-first IDE written in Rust. Kestrel pairs a calm, keyboard-driven workspace with a real [Agent Client Protocol](https://agentclientprotocol.com) client, so the same app can drive OpenCode, Claude Code, Codex, or any custom ACP harness.

No Electron. No webview. No Monaco. The entire UI is GPUI, the data layer is SQLite, and source control shells out to the system `git`.

## Status

Kestrel is an early MVP that runs natively on macOS. The chat surface is wired to live ACP sessions, including streaming messages, thoughts, tool calls, plans, usage updates and permission prompts. Files open in a lightweight syntax-highlighted editor with dirty tracking, `⌘S` save, optional autosave, an unsaved-changes prompt and external-change detection. New files are created through the native save panel. Transcripts can be copied as Markdown from the session menu. Projects, sessions and transcripts persist across launches; sessions and projects can be removed with a confirmation step, and settings, theme, the last open project and session, and the window frame are restored on startup.

The agent graph and trace waterfall are projections of the live transcript. Plan steps or recent tool calls become graph nodes; timestamps and tool durations become trace spans. Empty sessions show a labelled preview of the surfaces until the first prompt arrives.

Source control reads the system `git` CLI: branch, staged and unstaged entries, per-file line counts, stage/unstage, discard, stage all, commits, amend, fast-forward pull, push and branch switching. Every change opens a read-only unified diff. Source control refreshes when an agent turn finishes. The explorer rescans too, and an open editor with no unsaved edits reloads from disk. Project search runs on GPUI's background executor and skips build, VCS and secret files. Codex subscription windows are read live from the local Codex app-server; Claude Code reports when it needs a `user:profile` login, and OpenCode stays explicitly estimated. Nothing leaves the machine.

## Requirements

- macOS 15 or newer
- Rust 1.92 or newer (`rustup` recommended)
- A harness that speaks ACP over stdio. OpenCode is the default: `opencode acp`.
- `git` on `PATH` for source control

## Build and run

```sh
cargo run -p kestrel
```

The first launch creates `~/Library/Application Support/Kestrel/kestrel.db` and seeds the current workspace as a project. Set `KESTREL_DATA_DIR` to use a different data directory.

For a release build:

```sh
cargo build --release -p kestrel
```

## macOS app bundle

```sh
./scripts/bundle-macos.sh --build
open dist/Kestrel.app
```

The script copies the release binary, renders `assets/kestrel-icon.svg` into an `.icns`, writes `Info.plist`, and applies an ad-hoc signature. For distribution, sign with a Developer ID and notarize:

```sh
KESTREL_CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
  ./scripts/bundle-macos.sh --build
ditto -c -k --keepParent dist/Kestrel.app dist/Kestrel.app.zip
xcrun notarytool submit dist/Kestrel.app.zip --keychain-profile KESTREL --wait
xcrun stapler staple dist/Kestrel.app
```

Notarization packaging is intentionally manual for now.

## Harnesses

Open **Settings → Harnesses** (`⌘,`) to choose the default harness and edit the custom ACP command. Kestrel checks whether each command is installed and connects automatically when a harness is selected.

| Harness | Command |
| --- | --- |
| OpenCode | `opencode acp` |
| Claude Code | `claude-agent-acp` |
| OpenAI Codex | `codex-acp` |
| Custom ACP | Any command that speaks ACP over stdio |

The default is OpenCode. Selecting a different harness restarts the ACP process when auto-connect is enabled.

Permission requests are surfaced as cards in the chat. The permissions tab controls whether Kestrel asks, auto-approves, or runs read-only.

## Keyboard

| Shortcut | Action |
| --- | --- |
| `⌘K` | Command palette |
| `⌘1` / `⌘2` / `⌘3` | Chat / agent graph / trace |
| `⌘,` | Settings |
| `⌘O` | Add a project folder |
| `⌘N` | New session |
| `⌘⇧F` | Project search |
| `⌘⇧L` | Toggle light and dark theme |
| `⌘S` | Save the open file |
| `⌘↵` | Send prompt |
| `Esc` | Close palette, settings, diff, inspector or project menu |

## Architecture

```
apps/desktop        GPUI application, views, git and search integration
crates/ui           Kobra-themed design system on top of GPUI Kit
crates/domain       Shared project, session, usage, graph and trace types
crates/agent        Reusable ACP client running on a dedicated worker thread
crates/storage      SQLite persistence (WAL, foreign keys, migrations)
```

The ACP worker owns all protocol I/O. The UI sends commands and receives events over channels, so a slow model or child process never blocks the render thread. Permission handlers respond from a spawned task to keep the ACP dispatch loop moving.

Data lives in SQLite with WAL enabled. Projects and sessions are upserted as they change; transcripts are saved when a turn finishes, when a permission is resolved, when switching sessions and on shutdown. Settings are stored as JSON in the same database, so new options can be added without a schema migration. Settings → Advanced can clear every conversation while keeping projects and settings.

## Development

Run the full test suite:

```sh
cargo test --workspace
```

The agent crate includes deterministic smoke tools:

```sh
# End-to-end ACP turn against a real harness
cargo run -p kestrel-agent --bin harness_smoke -- "opencode acp" "Reply with: HARNESS OK"

# Permission round-trip against the bundled mock agent
cargo build -p kestrel-agent --bins
./target/debug/permission_smoke allow
./target/debug/permission_smoke reject
```

Useful diagnostics:

- `KESTREL_TRACE_HARNESS=1` logs every harness event to stderr.
- `KESTREL_HARNESS_COMMAND="..."` overrides the configured harness command for one run.
- `KESTREL_DATA_DIR=/path` redirects the SQLite database.
- `KESTREL_LOG_FILE=/path` redirects the lifecycle log (default `~/Library/Logs/Kestrel/kestrel.log`).
- `KESTREL_CODEX_BIN=/path/to/codex` overrides Codex detection for live usage probes.
- `KESTREL_AUTOPROMPT="..."` sends a prompt once the harness connects; useful for smoke tests.

## License

Apache-2.0.
