# Kestrel

A fast, native, agent-first IDE written in Rust. Kestrel pairs a calm, keyboard-driven workspace with a real [Agent Client Protocol](https://agentclientprotocol.com) client, so the same app can drive OpenCode, Claude Code, Codex, or any custom ACP harness.

No Electron. No webview. No Monaco. The entire UI is GPUI, the data layer is SQLite, and source control shells out to the system `git`.

## Status

Kestrel is an early MVP that runs natively on macOS. The chat surface is wired to live ACP sessions, including streaming messages, thoughts, tool calls, plans, usage updates and permission prompts. Files open in a lightweight syntax-highlighted editor with dirty tracking, `⌘S` save, optional autosave, an unsaved-changes prompt and external-change detection. New files are created through the native save panel. Transcripts can be copied as Markdown from the session menu. Projects, sessions and transcripts persist across launches; sessions and projects can be removed with a confirmation step, and settings, theme, the last open project and session, and the window frame are restored on startup.

Prompt queueing keeps follow-up instructions when the current turn is still running; entries can be edited, reordered or removed before dispatch. A debounced project watcher refreshes the explorer and source control after external edits. Turns and permission requests raise a macOS notification when the window is in the background, and waiting sessions update the Dock badge. Missing project folders are flagged in the sidebar with a **Locate folder…** recovery action.

The living plan is in [`docs/PLAN.md`](docs/PLAN.md); the implementation order and current state are in [`docs/ROADMAP.md`](docs/ROADMAP.md).

Projects can be organised into user-defined groups such as **Private** and **Work** from the Projects sidebar. Groups collapse, persist across launches, and can be renamed or deleted without deleting their projects. Use the `…` action on a project to move it between groups, or **New group…** to add a section.

Kestrel can also connect a project to a self-hosted relay. The relay owns the working tree, git, search and the ACP harness, while the macOS client keeps the full explorer, editor, source control, search, chat, graph and trace surfaces. Remote sessions live on the server, so they keep running when the Mac is closed and can be continued from another paired device. A native SwiftUI iOS client is included as a scaffold in `apps/ios`; it needs Xcode, which is not installed on the current machine.

The agent graph and trace waterfall are projections of the live transcript. Plan steps or recent tool calls become graph nodes; timestamps and tool durations become trace spans. Empty sessions show a labelled preview of the surfaces until the first prompt arrives.

Source control reads the system `git` CLI: branch, staged and unstaged entries, per-file line counts, stage/unstage, discard, stage all, commits, amend, fast-forward pull, push and branch switching. Every change opens a read-only unified diff. Source control refreshes when an agent turn finishes. The explorer rescans too, and an open editor with no unsaved edits reloads from disk. Project search runs on GPUI's background executor and skips build, VCS and secret files. Codex subscription windows are read live from the local Codex app-server; Claude Code reports when it needs a `user:profile` login, and OpenCode stays explicitly estimated. Local projects keep all of this on the machine; remote projects use the self-hosted relay described below.

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

## Remote relay

`kestrel-relay` is a small self-hosted server. It stores accounts, devices, project grants, a session index and — by default — transcript snapshots. OpenCode remains the source of truth for live sessions; the relay's snapshot lets another device show history and provides a backup. No prompt or tool payload is written to the relay log.

The relay spawns one ACP harness per connection and exposes it over WebSocket, so the macOS app and the iPhone share the same remote sessions.

Build and initialise it on the machine that has the repositories:

```sh
cargo build -p kestrel-relay
./target/debug/kestrel-relay init
./target/debug/kestrel-relay user add marek
./target/debug/kestrel-relay project add kestrel /Users/me/git/kestrel --user marek
./target/debug/kestrel-relay pair marek --ttl 10
./target/debug/kestrel-relay serve
```

For a standalone release binary:

```sh
./scripts/bundle-relay.sh
./dist/kestrel-relay serve
```

Optional `~/.config/kestrel/relay.toml`:

```toml
listen = "127.0.0.1:8787"
harness = "opencode acp"
linger_secs = 900       # keep an in-flight turn alive after the client leaves
sandbox = true          # macOS seatbelt: deny other projects and credentials
store_transcripts = true
```

`serve --config ~/.config/kestrel/relay.toml` uses it. CLI flags and `KESTREL_RELAY_*` environment variables override the file.

`serve` listens on `127.0.0.1:8787` by default. For a phone or another Mac, expose it over Tailscale:

```sh
tailscale serve --bg --https=443 http://127.0.0.1:8787
```

Then enter `https://<machine>.<tailnet>.ts.net` and the one-time pairing code in **Settings → Remote** on macOS, or in the iOS app. A paired device receives its own token; revoke it with `kestrel-relay revoke <device-id>`.

Remote projects support the full macOS surface: explorer, editor with save, search, source control, diffs, chat, permissions, graph and trace. The only difference from a local project is that files and the session live on the relay. A session can be opened on one device at a time; if another device is active, use **Take over session** in the session menu or the **Take over** button on iOS. ACP permissions are never auto-approved by the relay. Composer attachments are uploaded to `.kestrel/attachments` on the relay and kept out of `git status` through `.git/info/exclude`.

Saves use an optimistic modification-time check on both local and remote files. If a file changed since it was opened, Kestrel blocks the save and offers **Reload** (take the newer version) or **Overwrite** (force the local buffer).

If the client disconnects while a turn is running, the relay keeps the harness alive for `linger_secs` (15 minutes by default) so the turn can finish and the result is persisted by OpenCode. Permission requests raised during that window are auto-cancelled. Set `linger_secs` in the relay config or `KESTREL_RELAY_LINGER_SECS` to change the window.

Users are isolated: each account sees only granted projects and its own sessions. All relay users currently share the OpenCode credentials installed on the server. On macOS the relay runs each harness under a seatbelt profile that denies other project roots and common credential stores (`.ssh`, `.gnupg`, `.aws`, browser profiles, Documents/Desktop/Downloads). This is a best-effort privacy boundary, not a container; disable it with `sandbox = false` if a harness needs broader filesystem access, and use separate OS accounts or containers for hard isolation.

Transcript snapshots are stored in the relay's SQLite database. Set `store_transcripts = false` in the relay config (or `KESTREL_RELAY_STORE_TRANSCRIPTS=0`) to keep only metadata. The database is not encrypted at rest; use FileVault or an encrypted volume on the relay host.

## iOS

The SwiftUI client lives in `apps/ios`. It supports pairing, projects, sessions, chat, permissions and takeover. Generate the Xcode project with XcodeGen:

```sh
brew install xcodegen
cd apps/ios && xcodegen && open Kestrel.xcodeproj
```

See `apps/ios/README.md` for simulator and Tailscale setup. Xcode is required and is not installed on the current machine, so the iOS target has only been syntax-checked.

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
apps/ios            SwiftUI remote client (Xcode project generated from project.yml)
crates/ui           Kobra-themed design system on top of GPUI Kit
crates/domain       Shared project, session, usage, graph and trace types
crates/workspace    Tree, search, git and filesystem operations shared by local and remote
crates/agent        Reusable ACP client running on a dedicated worker thread
crates/relay        Self-hosted relay: accounts, pairing, grants, ACP-over-WS and workspace APIs
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

# Relay end-to-end: pairing, ACP-over-WS, session isolation and grants
./scripts/relay-smoke.sh
```

Useful diagnostics:

- `KESTREL_TRACE_HARNESS=1` logs every harness event to stderr.
- `KESTREL_HARNESS_COMMAND="..."` overrides the configured harness command for one run.
- `KESTREL_DATA_DIR=/path` redirects the SQLite database.
- `KESTREL_LOG_FILE=/path` redirects the lifecycle log (default `~/Library/Logs/Kestrel/kestrel.log`).
- `KESTREL_CODEX_BIN=/path/to/codex` overrides Codex detection for live usage probes.
- `KESTREL_AUTOPROMPT="..."` sends a prompt once the harness connects; useful for smoke tests.
- `KESTREL_RELAY_DB=/path` and `KESTREL_RELAY_LISTEN=addr` configure the relay.
- `KESTREL_RELAY_HARNESS="opencode acp"` sets the command the relay spawns.
- `KESTREL_RELAY_LINGER_SECS=900` keeps an in-flight turn alive after disconnect.
- `KESTREL_RELAY_SANDBOX=0` disables the macOS seatbelt profile.
- `KESTREL_RELAY_STORE_TRANSCRIPTS=0` keeps only relay metadata.
- `KESTREL_RELAY_DEBUG=1` prints the sandbox profile and forwards harness stderr.
- `KESTREL_REMOTE_TOKEN=...` is a test-only fallback for the macOS remote token.

## License

Apache-2.0.
