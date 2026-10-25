<p align="center">
  <img src="assets/dunlin-icon.svg" alt="Dunlin" width="84" />
</p>

<h1 align="center">Dunlin</h1>

<p align="center">
  A native, agent-first IDE for macOS.<br/>
  Built in Rust and GPUI. No Electron, no webview, no Monaco.
</p>

<p align="center">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-2ea44f" />
  <img alt="Platform: macOS 15+ Apple Silicon" src="https://img.shields.io/badge/macOS%2015%2B-Apple%20Silicon-111111" />
  <img alt="Status: public beta" src="https://img.shields.io/badge/status-public%20beta-f0a020" />
  <a href="https://github.com/corapoid/dunlin/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/corapoid/dunlin/actions/workflows/ci.yml/badge.svg" /></a>
</p>

<p align="center">
  <img src="docs/screenshots/chat.png" alt="A permission request inside a Dunlin conversation" width="100%" />
</p>

Dunlin pairs a calm, keyboard-driven workspace with a real
[Agent Client Protocol](https://agentclientprotocol.com) client. One app can
drive OpenCode, Claude Code, Codex or any custom ACP harness, with streaming
messages, plans, tool calls, diffs and permission requests rendered as native
UI instead of a terminal transcript.

The first public beta is intentionally small: **macOS 15+, Apple Silicon, local
projects and a self-hosted relay beta**. The iOS client is a scaffold and is not
part of this release.

## Why Dunlin

- **Truly native.** The entire interface is GPUI. The SQLite database is local,
  source control uses the system `git`, and there is no browser runtime in the
  loop.
- **ACP first.** Dunlin speaks the structured Agent Client Protocol directly.
  Tool calls and permission requests become real cards and diffs, not ANSI
  text to parse.
- **Local-first.** Projects, sessions and transcripts stay on your Mac. There
  is no telemetry, no bundled model and no account.
- **Built for follow-ups.** Queue prompts while a turn is running, edit or
  retry a message, copy the agent's answer, and revisit the transcript later.
- **Self-hosted remote.** Pair another Mac or iPhone with your own relay.
  Sessions keep running on the relay, and another device can take over.

## A tour

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/screenshots/onboarding.png" alt="Dunlin onboarding" />
      <br/><sub><b>Open a folder.</b> No demo projects, no hidden workspace. A folder is all Dunlin needs.</sub>
    </td>
    <td width="50%" valign="top">
      <img src="docs/screenshots/settings.png" alt="Dunlin settings" />
      <br/><sub><b>Make it yours.</b> Theme, harness, permissions, notifications and keyboard hints.</sub>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/screenshots/graph.png" alt="Dunlin agent graph" />
      <br/><sub><b>See the plan.</b> Plan steps and tool calls become a live agent graph.</sub>
    </td>
    <td width="50%" valign="top">
      <img src="docs/screenshots/trace.png" alt="Dunlin trace waterfall" />
      <br/><sub><b>See the time.</b> Every turn, model call, tool and permission becomes a trace span.</sub>
    </td>
  </tr>
</table>

## Features

### Chat and harnesses

- Streaming assistant output, reasoning, plans, tool calls and usage updates.
- Permission cards with allow-once, always-allow and reject choices.
- Unified diffs previewed inline before or after a tool writes a file.
- Prompt queue with edit, reorder, remove and clear while a turn is running.
- Message actions: copy, retry and edit. Agent responses can be copied as
  Markdown.
- Session history with persisted transcripts and interruption recovery.
- OpenCode (default), Claude Code, Codex and custom ACP commands. Each project
  can override the harness and permission mode.

### Workspace

- File explorer with a debounced watcher and missing-folder recovery.
- Syntax-highlighted editor with dirty tracking, `⌘S`, optional autosave,
  external-change detection and conflict recovery.
- Project search that skips build, VCS and secret files.
- Source control backed by the system `git`: status, stage, unstage, discard,
  commit, amend, pull, push, branches and read-only diffs.
- Agent graph and trace waterfall projected from the live transcript.
- Project groups, pinned ordering and per-project settings.

### Remote relay

- Self-hosted `dunlin-relay` with accounts, devices and project grants.
- One-time pairing codes; device tokens live in the macOS Keychain.
- ACP over WebSocket with session leases and explicit takeover.
- Transcript snapshots so another device can show history.
- macOS seatbelt sandbox for the harness, denying other projects and common
  credential stores.

## Install

### Download the beta

The current beta is ad-hoc signed because there is no Apple Developer ID yet.
macOS Gatekeeper may ask you to approve the app in
**System Settings → Privacy & Security**.

Release artifacts are published on the
[releases page](https://github.com/corapoid/dunlin/releases):

- `Dunlin-<version>-macos-arm64.dmg`
- `Dunlin-<version>-macos-arm64.zip`
- `dunlin-relay-<version>-macos-arm64.tar.gz`
- `SHA256SUMS`

The app requires macOS 15 or newer on Apple Silicon. Intel Macs are not
supported in 0.1.

### Build from source

Requirements:

- macOS 15 or newer on Apple Silicon
- Rust 1.92 or newer (`rustup` recommended)
- `git` on `PATH`
- An ACP harness for real agent work, for example `opencode acp`

```sh
git clone https://github.com/corapoid/dunlin.git
cd dunlin
cargo run -p dunlin
```

The first launch creates `~/Library/Application Support/Dunlin/dunlin.db` and
shows onboarding. Set `DUNLIN_DATA_DIR` to use a different data directory.

To build a local app bundle:

```sh
./scripts/bundle-macos.sh --build
open dist/Dunlin.app
```

The script renders `assets/dunlin-icon.svg`, writes `Info.plist`, copies the
license notices into the bundle and applies an ad-hoc signature. To sign with a
Developer ID:

```sh
DUNLIN_CODESIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" \
  ./scripts/bundle-macos.sh --build
```

## Harnesses

Open **Settings → Harnesses** (`⌘,`) to choose the default harness and edit the
custom ACP command. Dunlin checks whether each command is installed and
connects when a project opens.

| Harness | Command |
| --- | --- |
| OpenCode | `opencode acp` |
| Claude Code | `claude-agent-acp` |
| OpenAI Codex | `codex-acp` |
| Custom ACP | Any command that speaks ACP over stdio |

The default is OpenCode. Permission modes are **Ask every time**, **Read-only**
and **Auto-approve**. ACP permissions are never auto-approved by the relay.

## Remote relay

`dunlin-relay` is a small self-hosted server. It stores accounts, devices,
project grants, a session index and, by default, transcript snapshots. OpenCode
remains the source of truth for live sessions; the relay's snapshot lets another
device show history and provides a backup.

Build and initialise it on the machine that has the repositories:

```sh
cargo build -p dunlin-relay
./target/debug/dunlin-relay init
./target/debug/dunlin-relay user add marek
./target/debug/dunlin-relay project add dunlin /Users/me/git/dunlin --user marek
./target/debug/dunlin-relay pair marek --ttl 10
./target/debug/dunlin-relay serve
```

Optional `~/.config/dunlin/relay.toml`:

```toml
listen = "127.0.0.1:8787"
harness = "opencode acp"
linger_secs = 900       # keep an in-flight turn alive after the client leaves
sandbox = true          # macOS seatbelt: deny other projects and credentials
store_transcripts = true
```

`serve --config ~/.config/dunlin/relay.toml` uses it. CLI flags and
`DUNLIN_RELAY_*` environment variables override the file.

`serve` listens on `127.0.0.1:8787` by default. For a phone or another Mac,
expose it over Tailscale:

```sh
tailscale serve --bg --https=443 http://127.0.0.1:8787
```

Then enter `https://<machine>.<tailnet>.ts.net` and the one-time pairing code in
**Settings → Remote** on macOS, or in the iOS app. A paired device receives its
own token; revoke it with `dunlin-relay revoke <device-id>`.

Remote projects support the full macOS surface: explorer, editor with save,
search, source control, diffs, chat, permissions, graph and trace. A session can
be opened on one device at a time; if another device is active, use **Take over
session** in the session menu or the **Take over** button on iOS. Composer
attachments are uploaded to `.dunlin/attachments` on the relay and kept out of
`git status` through `.git/info/exclude`.

If the client disconnects while a turn is running, the relay keeps the harness
alive for `linger_secs` (15 minutes by default) so the turn can finish and the
result is persisted. Permission requests raised during that window are
auto-cancelled. Set `linger_secs` or `DUNLIN_RELAY_LINGER_SECS` to change the
window.

Users are isolated: each account sees only granted projects and its own
sessions. All relay users currently share the OpenCode credentials installed on
the server. The macOS relay runs each harness under a seatbelt profile that
denies other project roots and common credential stores (`.ssh`, `.gnupg`,
`.aws`, browser profiles, Documents/Desktop/Downloads). This is a best-effort
privacy boundary, not a container; use separate OS accounts or containers for
hard isolation.

The relay database is not encrypted at rest. Set `store_transcripts = false` to
keep only metadata, and use FileVault or an encrypted volume on the relay host.

## How Dunlin compares with Orca

[Orca](https://github.com/stablyai/orca) is an excellent, much more mature
AI orchestrator and the closest point of reference for this project. It is also
MIT-licensed. The comparison below is about fit, not quality.

| | **Dunlin 0.1 beta** | **Orca** |
| --- | --- | --- |
| Foundation | Rust + GPUI; no browser runtime | Electron + Chromium |
| Agent integration | Structured ACP client | CLI agents in terminal sessions |
| Parallel worktrees | Planned | Yes, a core feature |
| Terminal | Planned | Ghostty-class terminal with splits |
| Git | Status, commit, push, branches, diffs | Worktrees, PRs, Linear, diff annotation |
| Agent graph and trace | Live projections of the transcript | Session and task views |
| Remote | Self-hosted relay with session takeover | SSH worktrees, mobile companion |
| Mobile | iOS scaffold; not in 0.1 | Shipped iOS and Android apps |
| Platforms | macOS 15+, Apple Silicon | macOS, Windows, Linux |
| Maturity | Early public beta | Mature, shipping daily |
| License | MIT | MIT |

**Choose Orca** if you want terminal-native agents, parallel worktrees, a mature
mobile companion, SSH hosts or Windows/Linux support today.

**Dunlin may fit** if you want a native ACP client that treats tool calls and
permissions as structured UI, a small local-first workspace, and a self-hosted
relay you can run yourself.

If Dunlin is missing something you need, open an issue. The roadmap is public
and intentionally honest about what is not built yet.

## What is not in 0.1

- iOS build and sync (the SwiftUI client is a scaffold).
- Terminal and shell integration.
- Computer use.
- Automations, skills and artifacts.
- Visual Markdown editor.
- Parallel worktrees and multi-agent orchestration.
- Universal or Intel builds.

See [`docs/ROADMAP.md`](docs/ROADMAP.md) for the release gate and the next
milestones, and [`docs/PLAN.md`](docs/PLAN.md) for the architecture.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo test --workspace
./scripts/relay-smoke.sh
```

The agent crate includes deterministic smoke tools:

```sh
# End-to-end ACP turn against a real harness
cargo run -p dunlin-agent --bin harness_smoke -- "opencode acp" "Reply with: HARNESS OK"

# Permission round-trip against the bundled mock agent
cargo build -p dunlin-agent --bins
./target/debug/permission_smoke allow
./target/debug/permission_smoke reject

# Relay end-to-end: pairing, ACP-over-WS, session isolation and grants
./scripts/relay-smoke.sh
```

Useful diagnostics:

- `DUNLIN_TRACE_HARNESS=1` logs every harness event to stderr.
- `DUNLIN_HARNESS_COMMAND="..."` overrides the configured harness command for one run.
- `DUNLIN_DATA_DIR=/path` redirects the SQLite database.
- `DUNLIN_LOG_FILE=/path` redirects the lifecycle log (default `~/Library/Logs/Dunlin/dunlin.log`).
- `DUNLIN_CODEX_BIN=/path/to/codex` overrides Codex detection for live usage probes.
- `DUNLIN_AUTOPROMPT="..."` sends a prompt once the harness connects.
- `DUNLIN_RELAY_DB=/path` and `DUNLIN_RELAY_LISTEN=addr` configure the relay.
- `DUNLIN_RELAY_HARNESS="opencode acp"` sets the command the relay spawns.
- `DUNLIN_RELAY_LINGER_SECS=900` keeps an in-flight turn alive after disconnect.
- `DUNLIN_RELAY_SANDBOX=0` disables the macOS seatbelt profile.
- `DUNLIN_RELAY_STORE_TRANSCRIPTS=0` keeps only relay metadata.
- `DUNLIN_RELAY_DEBUG=1` prints the sandbox profile and forwards harness stderr.
- `DUNLIN_REMOTE_TOKEN=...` is a test-only fallback for the macOS remote token.

## Architecture

```
apps/desktop        GPUI application, views, git, search and local state
apps/ios            SwiftUI remote client (Xcode project generated from project.yml)
crates/ui           Design system on top of GPUI Kit
crates/domain       Shared project, session, usage, graph and trace types
crates/workspace    Tree, search, git and filesystem operations
crates/agent        Reusable ACP client on a dedicated worker thread
crates/relay        Self-hosted relay: accounts, pairing, grants, ACP-over-WS
crates/storage      SQLite persistence (WAL, foreign keys, migrations)
```

The ACP worker owns all protocol I/O. The UI sends commands and receives events
over channels, so a slow model or child process never blocks the render thread.
Permission handlers respond from a spawned task to keep the ACP dispatch loop
moving.

Data lives in SQLite with WAL enabled. Projects and sessions are upserted as
they change; transcripts are saved when a turn finishes, when a permission is
resolved, when switching sessions and on shutdown. Settings are stored as JSON
in the same database, so new options can be added without a schema migration.

## Security

Please report vulnerabilities privately as described in
[`SECURITY.md`](SECURITY.md). Do not open a public issue for a security problem.

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md). The project is early; small, focused
pull requests are easiest to review.

## License

Dunlin is released under the [MIT License](LICENSE). Bundled fonts and
dependencies are listed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
