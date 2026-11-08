# Terminal and Run — implementation and boundaries

Status: **local and relay PTY, SSH, tabs and splits, mouse selection,
scrollback, shell integration (OSC 133/7), OSC 8 and Run from manifests.**

Manual GUI/IME/bidi acceptance, a real relay/SSH host and unusual prompts
remain a separate QA matrix.

## Terminal

- `crates/terminal` starts a local PTY through `portable-pty` and maintains
  `vt100::Parser` on the UI side. The reader and waiter run on separate threads;
  the output channel is bounded, so a fast process cannot blow up memory.
- The default shell comes from `$SHELL`; when unavailable, the app tries
  `/bin/zsh`, `/bin/bash`, then `/bin/sh`. It starts as a login shell in the
  project directory. `TERM=xterm-256color`, `COLORTERM=truecolor`.
- The app removes internal `DUNLIN_*` variables and the relay bearer
  token from the terminal process. The rest of the user environment is inherited;
  project env is passed only to an explicitly started Run.
- Terminal and Run are additional panels in a single dock, not main
  views. `⌘J` shows/hides Terminal, `⌘⇧J` Run; a panel can also be closed
  with `×`, and state is remembered. The dock can be pinned to the bottom, left or
  right edge with header buttons or by dragging the `⠿` handle toward the chosen
  edge. The panel's inner edge is a resize handle:
  dragging changes height (bottom) or width (left/right), with limits
  relative to the window. The handle is 9 px wide and is a separate layout strip, so
  it does not compete with content scrolling; double-click restores the default size.
  Terminal and Run switch in the same dock without killing sessions.
- Tabs are independent sessions. Closing a tab sends kill to the child and removes
  the PTY; closing the app does the same through `Drop`. No automatic
  resume/replay after restart.
- Scrollback is limited to 5,000 lines by default (hard ceiling 50,000).
  The mouse wheel and PageUp/PageDown scroll the view; typing a character returns to
  the live viewport.
- PTY size is matched to the window during render (SIGWINCH through
  `MasterPty::resize`), with minimum dimensions of 3×20.
- `⌘F` or **Find** opens search of the visible buffer; matching fragments
  are highlighted, and the counter shows hits in the visible viewport.
- Supported input: Unicode from `key_char`, Enter/Tab/Backspace/Escape,
  arrows/Home/End/Delete/Insert, Ctrl+C/D/Z/L/A/E/U/K/W, Cmd+A (select all
  retained scrollback), Cmd+C (copy selection or buffer), Cmd+V (bracketed
  paste, 1 MiB limit), Cmd+W (close active tab), Cmd+D (split down),
  Cmd+Shift+D (split right).
- Rendering supports ANSI 16/256/RGB, bold, inverse and a block cursor.

### Mouse selection and links

- Left-button dragging creates a selection in absolute buffer lines,
  so it does not disappear when scrolling or resizing the window. Double-click
  selects a word/path, triple-click the whole line, Shift-click extends the existing
  selection. Dragging beyond the top/bottom edge scrolls by one line.
- Cmd+A selects the entire retained scrollback, Cmd+C and **Copy** copy
  the selection; without a selection they copy the entire visible buffer. Empty cells are
  preserved as spaces, trailing line spaces are trimmed, and the clipboard limit is
  1 MiB while preserving UTF-8 boundaries.
- Selection is local: terminal applications do not receive mouse events
  (no mouse reporting), so there is no conflict with text selection.
- `http://`, `https://` and `mailto:` links in text are clickable. OSC 8 is
  parsed incrementally even when the sequence is split across
  chunks; the link is assigned to cells and rendered in blue. Only explicitly
  allowed schemes qualify for opening; `javascript:`,
  `file:` and other URIs are ignored.

### Splits

- The `◫`/`⬓` buttons or `⌘⇧D`/`⌘D` split the active pane right/down and
  start an independent shell in the new pane (local, relay or SSH — according
  to the project). A split does not duplicate the session: the old tab stays in the left/top
  pane, the new one in the right/bottom.
- The split is a tree with a 0.5 ratio, bounded to 0.15–0.85. The split
  handle is 6 px and can be dragged; each PTY size is calculated
  separately and sent as SIGWINCH/`resize`.
- The active pane has a stronger border. Clicking a pane sets the active tab, and
  the tab bar lets you switch sessions; a tab already visible is only
  activated, a hidden one replaces the active pane. Closing a tab collapses exactly
  one split-tree leaf. Closing the last tab removes the tree.
- A row height of 15 px, cell width of 7.1 px and padding of 8 px are shared
  between rendering and hit-testing. The canvas registers pane bounds, so
  selection also works after changing the split and dock.

### Shell integration

- Enabled by default (Settings → Editor → Terminal → **Shell integration**).
  The change applies to new tabs; existing sessions keep their hooks.
  The app creates a private `0700` directory in the temporary directory and starts
  zsh through `ZDOTDIR` or bash through `--rcfile`; it does not modify the user's
  dotfiles. The wrapper first sources the original `.zshenv`/`.zprofile`/
  `.zshrc` (zsh) or profile/`.bashrc` (bash), then adds the hooks.
- Supported markers: OSC 133 `A` (prompt), `B` (start of command line),
  `C` (execution start), `D;<exit>` (end), OSC 7 `file://host/path` and
  OSC 1337 `CurrentDir=`/`RemoteHost=`. The OSC 633 variant is also accepted.
  Percent-encoding of the path is decoded.
- The UI shows the current directory from OSC 7, a green `OSC 133` marker, prompt-line
  tint and actions **Cmd** (copy last command) and **Out** (copy output
  of the last finished command). Nothing is automatically run
  or sent to the shell; actions only copy text.
- Blocks are limited to 512 entries; output is cut from the current
  buffer/scrollback, not stored as a hidden copy. An unknown shell
  (e.g. fish) works without integration and without an error; the user can disable it
  if their own prompt uses `ZDOTDIR`/`PROMPT_COMMAND` in an unusual way.
- Integration is not installed in the remote SSH shell; if the host emits
  OSC 133/7 itself, the UI shows them.

### Relay transport

- A relay project uses `GET /v1/terminal?project=...&rows=...&cols=...` with
  a bearer token. Relay checks the device and project grant, then opens
  a PTY in the project directory on the relay host. The client does not start a local
  shell as a fallback.
- WebSocket protocol: binary frames are raw PTY bytes in both directions;
  text frames are JSON `ready`, `exit`, `error`, `resize`, `close`, `ping`.
  The client sends `resize` after a pane change and `close` when closing a tab. Relay
  sends `exit` with code/signal and closes the socket. Heartbeat is 30 s.
- Limits: 8 simultaneous terminal sessions per relay, input frame up to
  2 MiB, `rows` 4–400, `cols` 20–500, output/input channels are bounded.
  The connection requires HTTPS/WSS outside loopback; the token is not passed
  to the shell.
- **Relay PTY is not a sandbox.** A user with a project grant gets a
  full shell on the relay host with the relay process's privileges. The project grant
  is the capability boundary; do not grant it to people who cannot
  execute code on that host.

### SSH transport v1

- The **SSH** button in the terminal toolbar opens a `user@host` bar (alias
  from `~/.ssh/config`) and an optional remote path. `Connect` starts the local
  `/usr/bin/ssh -tt` in a PTY; there is no own cryptography client or key
  copying.
- Used: `ControlMaster=auto`, a private `ControlPath` in a `0700` directory,
  `ControlPersist=60` and `ConnectTimeout=15`. Authentication, agent, keys and
  `known_hosts` are handled by OpenSSH; a new host key requires explicit
  acceptance in the terminal. The target is validated (no options/spaces), and the remote
  path is single-quoted before `cd`.
- Project env and Dunlin tokens are not sent to the host. SSH does not yet
  bootstrap relay, manage Keychain or build tunnels —
  that remains part of the SSH XL stage.

### Security and remoteness

The local terminal runs project code with user privileges. It is not a
sandbox. The project must be trusted. For a relay project the UI uses only the
relay grant; SSH requires an explicit target and user decision. Closing the window
or tab ends the process; there is no hidden replay.

## Run

- `dunlin_workspace::run::detect` reads statically, without evaluating manifests:
  `package.json` (scripts + npm/pnpm/yarn/bun), `Cargo.toml` (run/test/check/
  build/clippy, `[[bin]]`, `[[example]]`, workspace), `Makefile`, `justfile`,
  `pyproject.toml` (pytest/ruff/mypy), `go.mod`, `deno.json`, `composer.json`,
  `Rakefile`, `mix.exs`, `gradlew`, `mvnw` and `scripts/`/`bin/` for
  `.sh/.bash/.zsh/.py/.js/.mjs/.cjs/.rb/.pl`.
- The scan is limited to 40 directories (depth 2), 80 candidates, 1 MiB per
  manifest; it skips `.git`, `node_modules`, `target`, build/dist, venv and hidden
  directories. Symlinks are not followed.
- A candidate has a stable ID, exact `argv`, program, relative cwd and source.
  Execution does not go through a shell string; project env is shown
  as **names**, without values, and passed to the PTY.
- Every run requires native confirmation with an argv/cwd/env preview.
  The command starts in a new terminal tab; exit code/signal goes into history.
- Run is local. A relay project can use the relay terminal, but detection and
  running Run on the relay host remain a separate scope.
- History is local, limited to 50 records, with explicit status
  `running/succeeded/terminated/...`. Closing the tab marks the run as interrupted
  by the user. After restart, historical `running` becomes
  `interrupted by app restart`; nothing is automatically resumed.
- No automatic retry, auto-run, executing text from logs or detecting
  commands through an LLM.

## Validation

- PTY tests spawn a real `/bin/sh`, check output and exit code.
- VT tests check ANSI, scrollback, OSC 8/133/7, selection, text
  extraction, split ratio and safe links.
- Shell integration tests start real `/bin/zsh` and `/bin/bash`
  when available, and wait for a finished OSC 133 block with command and exit code.
- Relay tests check 401/403 on WebSocket, `ready` and a real PTY stream
  finished with `exit`.
- Terminal/relay/desktop suite: **380 tests OK / 0 skipped**
  (`cargo test -p dunlin-terminal -p dunlin-relay -p dunlin`). `cargo fmt
  --all --check`, Clippy `-D warnings` for changed packages and
  `git diff --check` pass.
- Manual GUI, IME/bidi, mouse drag in splits, fast output, SIGINT,
  a real relay/SSH host and unusual prompts remain to be done.
