<p align="center">
  <img src="assets/dunlin-icon.svg" alt="Dunlin" width="88" />
</p>

<h1 align="center">Dunlin</h1>

<p align="center">
  <b>A native, agent-first IDE for macOS.</b><br/>
  Rust + GPUI. No Electron, no browser runtime, no cloud account.
</p>

<p align="center">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-2ea44f" />
  <img alt="Platform: macOS 15+ Apple Silicon" src="https://img.shields.io/badge/macOS%2015%2B-Apple%20Silicon-111111" />
  <img alt="Status: public beta" src="https://img.shields.io/badge/status-public%20beta-f0a020" />
  <a href="https://github.com/corapoid/dunlin/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/corapoid/dunlin/actions/workflows/ci.yml/badge.svg" /></a>
  <a href="https://github.com/corapoid/dunlin/releases"><img alt="Latest release" src="https://img.shields.io/github/v/release/corapoid/dunlin?include_prereleases&label=release" /></a>
  <a href="https://github.com/corapoid/dunlin/releases"><img alt="Last release date" src="https://img.shields.io/github/release-date/corapoid/dunlin?include_prereleases&label=released" /></a>
</p>

<p align="center">
  <img src="docs/screenshots/chat.png" alt="Dunlin conversation with native reasoning, tool and file cards" width="100%" />
</p>

Dunlin is a calm, keyboard-driven workspace for coding with agents. It speaks
the **Agent Client Protocol** directly, so tool calls, plans, diffs and permission
requests are native UI — not terminal text to parse. Projects, sessions and
history stay on your Mac or your explicitly selected self-hosted server.

**Public beta** · macOS 15+ · Apple Silicon desktop
· macOS ARM64 / Linux x86_64 and ARM64 relay

**Dunlin is in beta. We recommend cloning this repository and building from
source.** Prebuilt app downloads and Homebrew installation are paused until
Apple Developer ID signing and notarization are available.

## What you won't find combined anywhere else

- **One native ACP surface for every agent.** OpenCode, Claude Code, Codex,
  Gemini, Copilot, Cursor, custom ACP adapters — with structured tool cards,
  permission gates, plans, diffs and subagent context in one UI.
- **A real agent workspace, not a chat box.** Queue prompts while a turn runs,
  edit or retry messages, resume history, and keep per-project/group harness,
  model and environment defaults.
- **Agents that know their project peers.** Built-in MCP tools expose observed
  runs, bounded result reports and inbox messages. Human-approved deferred turns
  wait for fixed run/task dependencies without model polling, locally or on a
  paired relay, preserving mode, permissions and budgets.
- **Live Graph and file Timeline.** Follow agents and subagents, then inspect
  animated file-edit history and recorded diffs from the actual transcript.
- **Your history comes with you.** Import Claude Code, Codex and OpenCode
  sessions locally, including per-turn tokens and file-diff counts, with no
  cloud upload.
- **Stored and use-only credentials.** Name/key pairs stay in the macOS Keychain
  without agent access; explicitly configured short-lived policies let
  agents request an allowlisted HTTPS call, a named policy operation or a
  fixed CLI recipe without receiving the value. Auto-approved only within the
  key's host/method/grant scope, with optional per-use approval and a separate
  relay-side vault.
- **Intent, concepts and decisions as files.** A generic knowledge spine
  indexes Markdown frontmatter, validates relations and builds a bounded intent
  envelope for agents. The optional sandboxed WASM governance pack adds the
  schema; agent writes stay pending proposals until a human accepts them.
- **Evidence you can inspect.** Capture the clipboard or current file as a
  content-addressed artifact, preview text and images locally or over a relay,
  link it as evidence and share it through revocable, expiring capability links.
- **A native Git workbench.** Graph/history, stash, worktrees, merge/rebase/
  cherry-pick, inline blame, read-only GitHub/GitLab repository links and
  AI-assisted Conventional Commits. PR/Actions integration has been removed.
- **Visual Markdown that is still a real editor.** Editable blocks, tables,
  code fences, raw mode, undo and save — no embedded browser or Monaco.
- **A self-hosted server workspace.** Manage a named macOS/Linux relay without
  a local agent, install and authorize OpenCode/Codex/Claude Code remotely,
  clone repositories or copy a reviewed project snapshot with optional Git history.
  Pair with a one-use, 15-minute code; harnesses use macOS seatbelt or Linux
  bubblewrap, with server-side credential storage. Remote connections use private
  Tailscale Serve HTTPS only; SSH bootstrap, tunnels and terminals are removed.
  Tagged Linear issues arrive through native MCP and can be explicitly approved
  for a durable, serial queue on a chosen host, with review-status delivery.
- **Portable project organization.** Local collections and GitHub/GitLab imports,
  plus settings/project-structure backup to JSON or an existing repository.
  Imported catalog entries download only when explicitly requested.
- **A flexible native workbench.** Two document panes, independent conversation
  drafts, shared reading zoom, project search and recoverable generated-folder
  cleanup, with refreshed light/dark chrome and keyboard controls.
- **A terminal that goes the distance.** Local PTY and relay transports,
  draggable splits, mouse selection, OSC 8 links and zsh/bash shell integration
  with command/output actions.
- **Two-step onboarding and a macOS permission center.** No account, no telemetry,
  no bundled model, and no pretending that TCC permissions were granted.

Individually some of these exist elsewhere; the combination in one native
Rust/GPUI app is the unusual part.

## Fast, native, lean

Measured on an Apple M1 / 16 GB / macOS 27.0.1 with the release build, no agent
connected, median of three warm launches:

| Metric | Measured |
| --- | ---: |
| First window, empty onboarding | **0.31 s** |
| First window, local project | **0.30 s** |
| First window, 1,748-file project | **0.32 s** |
| Idle memory, empty | **99 MiB** |
| Idle memory, project workspace | **103–106 MiB** |
| Idle CPU | **0.4%** |
| App executable | **42.4 MiB** |

First launch after install was 1.07 s on the same machine because of cold
filesystem and Gatekeeper work. There is no Electron, Chromium, Node runtime or
Monaco in the running app; the UI is GPUI and the data layer is local SQLite.
Long transcripts open as a bounded 120-item window, and file, Git, language and
icon scans run off the UI thread with debounced refreshes.
The exact method and caveats are in [performance measurements](docs/quality/performance.md).

## Start

Clone the repository and build the beta locally:

```sh
git clone https://github.com/corapoid/dunlin.git
cd dunlin
./scripts/bundle-macos.sh --build
open dist/Dunlin.app
```

Source-build requirements: macOS 15+, Apple Silicon and Rust 1.92+ (`rustup`).
Local agent work needs an ACP harness such as `opencode acp`; a remote workspace
uses the harness installed on its server instead. Development runs with
`cargo run -p dunlin`.

Local app bundles are ad-hoc signed, not Apple-notarized. An Apple Developer ID
certificate is not required to build from source.

## Set up a remote server

The interactive terminal installer collects the server name,
optional Tailscale, autostart, Git tools and harness choices before installation.
The latest release includes the bootstrap script, macOS/Linux relay archives and
checksums.

Download `install-relay.sh` from the
[releases page](https://github.com/corapoid/dunlin/releases), then run:

```sh
bash install-relay.sh
```

Review the script before running it as your regular service user, not root.
For source builds, see [relay setup](docs/administration/relay-setup.md#build-and-register-from-source).

Pair in **Settings → Remote · Tailscale**, then choose **Manage** or use the title-bar host
switcher. Server tool settings and authorization stay remote; appearance stays
local. Copying is a one-time, non-overwriting snapshot, not synchronization.
Review files and history for secrets before uploading.

Administration, cloning and copying require a relay administrator. Host Git
authorization requires an administrator-only relay; separate OS accounts/services
are needed for untrusted users. Existing macOS relays retain their agent homes
until explicit `setup`; opting in may require sign-in and new conversations.

See [Server setup and remote workspace](docs/administration/relay-setup.md) for commands,
Linux keyring/reboot requirements, transfer limits and acceptance caveats.

## A quick look

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/screenshots/onboarding.png" alt="Two-step Dunlin onboarding" />
      <br/><sub><b>Open a folder.</b> No demo projects or hidden workspace.</sub>
    </td>
    <td width="50%" valign="top">
      <img src="docs/screenshots/settings.png" alt="Dunlin settings" />
      <br/><sub><b>Make it yours.</b> Flat settings, themes, permissions and TCC.</sub>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/screenshots/graph.png" alt="Dunlin agent graph" />
      <br/><sub><b>See the plan.</b> Agents, tools and subagents as a live graph.</sub>
    </td>
    <td width="50%" valign="top">
      <b>Follow the changes.</b><br/>
      <sub>Timeline shows animated additions, removals and per-file edit history.</sub>
    </td>
  </tr>
</table>

<p align="center"><sub><a href="docs/user-guide/screenshots.md">Open the screenshot gallery — light and dark, plus feature close-ups →</a></sub></p>

## Learn more

- [Getting started](docs/getting-started/README.md) · [User guide](docs/user-guide/README.md).
- [Roadmap](docs/project/roadmap.md) — the single remaining-work backlog.
- [Documentation index](docs/README.md) — guides, references and acceptance notes grouped by task.
- [Architecture](docs/architecture/README.md) · [Configuration reference](docs/reference/README.md).
- [Changelog](CHANGELOG.md) · [Performance methodology](docs/quality/performance.md).
- [Project import](docs/user-guide/project-import.md) · [Session import](docs/user-guide/session-import.md) · [Markdown editor](docs/user-guide/markdown-editor.md) · [Server setup](docs/administration/relay-setup.md).
- [Settings and project backup](docs/administration/settings-project-backup.md) — JSON or GitHub/GitLab, with deferred repository downloads.
- [R11 manual QA matrix](docs/quality/manual-acceptance.md) · [Security](SECURITY.md) · [Contributing](CONTRIBUTING.md).

## Status

Dunlin is an early public beta. Intel Macs are not supported in the current beta. See the
[roadmap](docs/project/roadmap.md) for what is still open and the
[documentation index](docs/README.md) for the detailed references.

## License

MIT — see [LICENSE](LICENSE). Bundled fonts and dependencies are listed in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
