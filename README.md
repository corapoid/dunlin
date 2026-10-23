# Kestrel

A fast, native, agent-first IDE written in Rust. Kestrel pairs a calm, keyboard-driven workspace with a real [Agent Client Protocol](https://agentclientprotocol.com) client, so the same app can drive OpenCode, Claude Code, Codex, or any custom ACP harness.

No Electron. No webview. No Monaco. The entire UI is GPUI, the data layer is SQLite, and source control shells out to the system `git`.

## Status

Kestrel is an early MVP that runs natively on macOS. The chat surface is wired to live ACP sessions, including streaming messages, thoughts, tool calls, plans, usage updates and permission prompts. Projects, sessions and transcripts persist across launches.

The graph and trace surfaces currently render deterministic demo projections. They are the next area to connect to the real session log.

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
| `⌘↵` | Send prompt |
| `Esc` | Close palette, settings, inspector or project menu |

## Architecture

```
apps/desktop        GPUI application, views, git and search integration
crates/ui           Kobra-themed design system on top of GPUI Kit
crates/domain       Shared project, session, usage, graph and trace types
crates/agent        Reusable ACP client running on a dedicated worker thread
crates/storage      SQLite persistence (WAL, foreign keys, migrations)
```

The ACP worker owns all protocol I/O. The UI sends commands and receives events over channels, so a slow model or child process never blocks the render thread. Permission handlers respond from a spawned task to keep the ACP dispatch loop moving.

Data lives in SQLite with WAL enabled. Projects and sessions are upserted as they change; transcripts are saved when a turn finishes, when a permission is resolved, when switching sessions and on shutdown.

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

## License

Apache-2.0.
