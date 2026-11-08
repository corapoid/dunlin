# Dunlin — technical plan

This document describes the direction of Dunlin after version `0.1`: the target
service model, architectural decisions and work order. The roadmap with concrete
milestones is in [`ROADMAP.md`](ROADMAP.md).

Status after the 2026-11-01 / beta.4 review: [`REMAINING-WORK.md`](REMAINING-WORK.md).
This plan describes the target architecture, not a list of completely unimplemented features.
Already present are global skills, hierarchical groups, the recorded-data Dashboard,
session import from files, a local Git graph with branch create and some feature modules.
History import does not replace the planned billing usage scanners.
Added: local/relay Git/blame, editable Visual/Raw Markdown, inherited
runtime settings and all eight workbench v1 areas with GitHub CI/PR.
Added: remote checkpoints/compare/context/GitHub, explicit multi-root grants,
billing scanners with a dashboard, and graph/trace search+zoom, onboarding and the TCC
center. This does not close QA, full LSP impact or new subsystems;
current boundaries are in the [roadmap](ROADMAP.md) and the [workbench scope](agent-workbench-and-github.md).

## Principles

1. Dunlin remains a native macOS app in Rust/GPUI. We are not introducing
   Electron or Monaco.
2. The Markdown view should be visual, but without a webview: a block editor based on a Markdown
   AST, with the ability to switch to raw text.
3. Computer use gets a separate helper in Swift/AppKit. This way Accessibility
   and Screen Recording permissions are granted to the helper, not the whole
   app — as mature agent-first IDEs do.
4. Automations can run while Dunlin is closed: locally through
   LaunchAgent, remotely through the scheduler in relay.
5. SSH is transport to the existing remote mode: OpenSSH ControlMaster +
   `dunlin-relay` bootstrap on the host. Full parity with relay, without a second
   file backend.
6. "Save as artifact" is an explicit agent command. An artifact is a separate
   session object, not an accidental file in the repo.
7. When in doubt, we look at mature agent-first IDE solutions and implement
   independently.

## Layers

```
apps/desktop
  features/chat, projects, usage, terminal, skills, automations, artifacts
  services/watcher, notifications, permissions, scheduler, pty, ssh
crates/domain       shared models
crates/storage      SQLite, migrations, FTS5
crates/workspace    fs, git, search, tree
crates/relay        relay API, ACP bridge, scheduler
crates/usage        subscription probes + accurate provider scanners
crates/terminal     PTY, VT, scrollback
crates/skills       discovery, validation, materialization
crates/artifacts    model, storage, preview
crates/audit        Cloudflare/Codex/SARIF findings, coverage, bounded validation
crates/vault        use-only credentials, Keychain policy, HTTP/CLI broker, MCP proxy
crates/automation   scheduler, runs, precheck
crates/ssh          OpenSSH ControlMaster, relay bootstrap
crates/mcp          local tool server for harnesses
```

`DunlinApp` must not keep growing in a single file. `app.rs` is already split
into `app/overlays.rs` (UI/overlays), `app/helpers.rs` (helper functions) and
`app/tests.rs`; extracting shared services through `Services` and
the event bus remains.

## Data model

### Project and space

- `Project` — existing workspace (local, relay, SSH).
- `ProjectGroup` — existing sections.
- `ChatSpace` — chat without a project; a private scratch workspace in
  `Application Support/Dunlin/chats/<id>`.
- `ProjectSettings` — overrides: harness, model, permission mode, env, skills,
  automation defaults.
- `SshHost` — host, user, port, key/agent, jump host, relay bootstrap.

### Sessions and usage

- `SessionSummary` — unchanged, with an optional `project_id` after splitting out
  `ChatSpace`.
- `UsageEvent` — a single turn/provider: input, output, cache read/write,
  reasoning, cost, model, harness, provider session id, project.
- `UsageSession`/`UsageDaily` — aggregates for the dashboard.
- Provider scanners have their own checkpoints (mtime, size, offset, owner keys),
  so they do not read logs from scratch or duplicate forks.

### Automations and artifacts

- `Automation` — prompt, cron/RRULE, zone, project, harness, model, precheck,
  workspace mode, permission policy, `next_run_at`.
- `AutomationRun` — status, output, usage, diff, error.
- `Artifact` — session/project, type (`markdown`, `html`, `csv`, `image`, `diff`,
  `screenshot`), content/path, version, visibility, link.
- `Skill` — `SKILL.md` standard, source, providers, hash, enabled per project.
- `Toast`/`Notification` — actions and history.

## Technical decisions

### Markdown

- Implemented: source-mapped AST → editable GPUI components, Visual/Raw,
  formatting, table/list structure, selection and explicit project resources;
  canonical source and shared undo/save, without lossy AST serialization.
- Manual GUI/IME/bidi QA, complex edge cases and real relay
  acceptance remain; [details](markdown-editor.md). Do not plan the finished phase 1/2 again.
- Raw text is always available with one switch.
- No HTML/JS inside. HTML artifacts are either a native safe subset or
  opened in a browser (to be decided with artifacts).

### Computer use

- `Dunlin Computer Use.app` — separate bundle id, Swift/AppKit.
- Accessibility API for the tree, ScreenCaptureKit for screenshots, CGEvent for input.
- TCC status, opening System Settings, permission reset.
- The main app talks to the helper over local IPC with a token.
- Disabled by default; app allowlist, confirmation of destructive actions,
  visible activity indicator and global stop.

### SSH

- OpenSSH `ControlMaster`/`ControlPersist`, `ssh -G` for config resolution.
- `dunlin-relay` bootstrap on the host (upload binary, start, local
  tunnel). The existing remote backend then keeps working.
- Passphrase in Keychain; known_hosts and host-key verification mandatory.
- "Plain SSH" fallback only for terminal/browsing, without full parity.

### Usage

- ACP `usage_update` provides context and an optional session cost report with currency,
  not a complete bill. The v1 ledger uses only valid USD; missing/
  non-USD/opaque child work remains unknown, not zero or a guaranteed billing cap.
- Accurate usage from logs: Claude JSONL, Codex rollout JSONL, OpenCode SQLite.
- Pricing per model, cache and long-context; `unpriced` instead of making things up.
- Local and relay scanners; global/project/model/harness dashboard.

### Skills

- `SKILL.md` standard + YAML frontmatter.
- Roots: `~/.agents/skills`, `~/.claude/skills`, `~/.codex/skills`,
  `.agents/skills`, `.claude/skills`, `.dunlin/skills`.
- Enable per project by materializing symlinks into harness roots or
  a list in configuration.
- Adding from a folder/repo, editing, validation, hash, script warnings.

### Automations

- Precheck → worktree (optional) → ACP session → prompt → usage → notification.
- By default read-only or work on a separate worktree; no auto-push.
- Local scheduler in the app, LaunchAgent when the app is closed, relay
  for remote hosts.
- Time, concurrency and run-history limits.

## Security

- Terminal and computer use are capabilities, not enabled by default.
- Skills may contain instructions/scripts — we show the source and hash.
- Automations never push without explicit consent.
- TCC permissions have statuses and explanations in the UI.
- Usage scanners are local; sending statistics to relay is optional.
- Artifacts have an explicit publish action, expiration and revoke.

## Work order

1. Foundation: finish splitting `app.rs` (Services/event bus), models,
   migrations and design tokens.
2. M1 — daily work (queue, message actions, notifications, watcher,
   missing folder, toasts, context menus, narrow window).
3. M2 — per-project settings, groups v2, usage, onboarding, permissions center.
4. M3 — terminal, commands, Git v2, search, Markdown, standalone chat, artifacts,
   skills.
5. M4 — SSH, automations, computer use.
6. M5 — graph/trace, budgets, sharing, accessibility, polish.
