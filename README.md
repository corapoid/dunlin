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
</p>

<p align="center">
  <img src="docs/screenshots/chat.png" alt="Dunlin conversation with native reasoning, tool and file cards" width="100%" />
</p>

Dunlin is a calm, keyboard-driven workspace for coding with agents. It speaks
the **Agent Client Protocol** directly, so tool calls, plans, diffs and permission
requests are native UI — not terminal text to parse. Projects, sessions and
history stay on your Mac or your explicitly selected self-hosted server.

**0.1.0-beta.10** · macOS 15+ · Apple Silicon desktop
· macOS ARM64 / Linux x86_64 and ARM64 relay

The latest published release is [0.1.0-beta.10](https://github.com/corapoid/dunlin/releases/tag/v0.1.0-beta.10).
Published downloads have passed independent checksum, bundle and architecture checks.
Beta.9 was tagged but its Linux test gate blocked publication; the tag is retained.

## What you won't find combined anywhere else

- **One native ACP surface for every agent.** OpenCode, Claude Code, Codex,
  Gemini, Copilot, Cursor, custom ACP adapters — with structured tool cards,
  permission gates, plans, diffs and subagent context in one UI.
- **A real agent workspace, not a chat box.** Queue prompts while a turn runs,
  edit or retry messages, resume history, and keep per-project/group harness,
  model and environment defaults.
- **Live Graph and Trace.** Every request, model call, tool and permission is
  projected from the actual transcript — not a mock, estimate or demo view.
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
  bubblewrap, with server-side credential storage. SSH hosts reuse system
  OpenSSH with ControlMaster, Keychain passphrases, relay bootstrap and
  loopback tunnels instead of a second crypto stack.
- **Portable project organization.** Local collections and GitHub/GitLab imports,
  plus settings/project-structure backup to JSON or an existing repository.
  Imported catalog entries download only when explicitly requested.
- **A flexible native workbench.** Two document panes, independent conversation
  drafts, shared reading zoom, project search and recoverable generated-folder
  cleanup, with refreshed light/dark chrome and keyboard controls.
- **A terminal that goes the distance.** Local PTY, relay and SSH transports,
  draggable splits, mouse selection, OSC 8 links and zsh/bash shell integration
  with command/output actions.
- **Two-step onboarding and a macOS permission center.** No account, no telemetry,
  no bundled model, and no pretending that TCC permissions were granted.

Individually some of these exist elsewhere; the combination in one native
Rust/GPUI app is the unusual part.

## Fast, native, lean

Measured on an Apple M1 / 16 GB / macOS 27.0.1 with the release build, no agent
connected, median of three warm launches:

| | |
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
The exact method and caveats are in [`docs/performance.md`](docs/performance.md).

## Start

Install the current beta with Homebrew:

```sh
brew trust corapoid/tap
brew install --cask corapoid/tap/dunlin@beta
```

The macOS packages are also published as an OCI artifact in
[GitHub Packages](https://github.com/corapoid/dunlin/pkgs/container/dunlin):

```sh
oras pull ghcr.io/corapoid/dunlin:beta -o dunlin-beta
```

Or download the DMG, `.pkg` installer or ZIP from the
[releases page](https://github.com/corapoid/dunlin/releases), or build it:

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

The current beta is ad-hoc signed and not Apple-notarized; the `.pkg` installer
is unsigned. macOS may ask you to approve the app in
**System Settings → Privacy & Security**.

## Set up a remote server

The interactive terminal installer collects the server name,
optional Tailscale, autostart, Git tools and harness choices before installation.
Beta.10 includes the bootstrap scripts, macOS/Linux relay archives and checksums.

```sh
bash <(curl -fsSL https://github.com/corapoid/dunlin/releases/download/v0.1.0-beta.10/install-relay.sh)
```

Review the script before running it as your regular service user, not root.
For source builds, see the [full guide](docs/guide.md#remote-relay).

Pair in **Settings → Remote**, then choose **Manage** or use the title-bar host
switcher. Server tool settings and authorization stay remote; appearance stays
local. Copying is a one-time, non-overwriting snapshot, not synchronization.
Review files and history for secrets before uploading.

Administration, cloning and copying require a relay administrator. Host Git
authorization requires an administrator-only relay; separate OS accounts/services
are needed for untrusted users. Existing macOS relays retain their agent homes
until explicit `setup`; opting in may require sign-in and new conversations.

See [Server setup and remote workspace](docs/relay-setup.md) for commands,
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
      <img src="docs/screenshots/trace.png" alt="Dunlin trace waterfall" />
      <br/><sub><b>See the time.</b> Requests, tools and permissions as trace spans.</sub>
    </td>
  </tr>
</table>

<p align="center"><sub><a href="docs/screenshots.md">Open the screenshot gallery — light and dark, plus feature close-ups →</a></sub></p>

## Learn more

- [Full guide](docs/guide.md) — the detailed feature and operations reference.
- [Roadmap](docs/ROADMAP.md) and [remaining work](docs/REMAINING-WORK.md).
- [Changelog](CHANGELOG.md) · [Performance methodology](docs/performance.md).
- [Project import](docs/project-import.md) · [Session import](docs/session-import.md) · [Markdown editor](docs/markdown-editor.md) · [Server setup](docs/relay-setup.md).
- [Settings and project backup](docs/settings-project-backup.md) — JSON or GitHub/GitLab, with deferred repository downloads.
- [R11 manual QA matrix](docs/r11-qa.md) · [Security](SECURITY.md) · [Contributing](CONTRIBUTING.md).

## Status

Dunlin is an early public beta. The iOS client is a scaffold, not part of this
release. Intel Macs are not supported in 0.1. See the
[roadmap](docs/ROADMAP.md) for what is built and what is still open.

## License

MIT — see [LICENSE](LICENSE). Bundled fonts and dependencies are listed in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
