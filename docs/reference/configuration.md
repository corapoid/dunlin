# Configuration

Reviewed against the source tree on **2026-11-09**. Paths below are defaults,
not a claim that all state follows `DUNLIN_DATA_DIR`.

## Desktop

Persistent preferences are configured in [Settings](../user-guide/settings.md).
Project/group model and environment inheritance is documented in
[runtime defaults](project-runtime-settings.md).

| Environment variable | Effect | Source |
| --- | --- | --- |
| `DUNLIN_DATA_DIR` | Overrides the desktop data directory, including `dunlin.db`; default `~/Library/Application Support/Dunlin`. | [Data helpers](../../apps/desktop/src/app/helpers.rs) |
| `DUNLIN_LOG_FILE` | Lifecycle log path; default `~/Library/Logs/Dunlin/dunlin.log`. Independent of the data-directory override. | [Logging](../../apps/desktop/src/logging.rs) |
| `DUNLIN_HARNESS_COMMAND` | Local harness command override used by activation/fixtures; not a remote server setting. | [Readiness](../../apps/desktop/src/harness/readiness.rs) |
| `DUNLIN_TRACE_HARNESS` | Presence enables raw harness event diagnostics on stderr. Can expose sensitive content; not the redacted log-export contract. | [Harness](../../apps/desktop/src/harness.rs) |
| `DUNLIN_CODEX_BIN` | Codex executable override for account-usage probes. | [Usage](../../crates/usage/src/lib.rs) |
| `DUNLIN_AUTOPROMPT` | Automatically submits a prompt after connection; developer/fixture use, potentially paid work. Do not set for ordinary isolated startup. | [App](../../apps/desktop/src/app.rs) |
| `DUNLIN_REMOTE_TOKEN` | Test fallback for remote authentication. Normal paired tokens belong in Keychain. Never put a real token into shared shell history. | [Remote client](../../apps/desktop/src/remote.rs) |

For an isolated development database/log (not isolated Keychain or TCC):

```sh
DUNLIN_DATA_DIR="$TMPDIR/dunlin-dev" \
  DUNLIN_LOG_FILE="$TMPDIR/dunlin-dev.log" cargo run -p dunlin
```

Choose a disposable path that does not contain data you need. QA scenarios have
stricter fresh-directory naming rules; use their runner rather than repurposing
your normal data directory.

## Relay TOML

The file is loaded only when passed to `serve --config PATH`; a file named
`~/.config/dunlin/relay.toml` is not implicitly discovered. Current precedence for
**serve** is **defaults → explicit TOML → CLI flags → environment**. In particular,
an exported `DUNLIN_RELAY_DB` overrides `serve --db`, not the other way around.

```toml
name = "Studio server"
listen = "127.0.0.1:8787"
harness = "opencode acp"
max_frame_bytes = 4194304
idle_timeout_secs = 900
lease_secs = 120
linger_secs = 900
sandbox = true
store_transcripts = true
```

```sh
dunlin-relay serve --config "$HOME/.config/dunlin/relay.toml"
```

| TOML field | Default / constraints |
| --- | --- |
| `name` | `Dunlin server`. |
| `listen` | `127.0.0.1:8787`; expose through a reviewed private HTTPS/tunnel setup. |
| `db` | macOS: `~/Library/Application Support/Dunlin/relay.db`; Linux: `~/.local/share/dunlin/relay.db`. Relative paths resolve from the service working directory. |
| `harness` | `opencode acp`. Server-owned executable/authentication. |
| `max_frame_bytes` | 4 MiB; file values clamp to 64 KiB–32 MiB. |
| `idle_timeout_secs` | 900; file values clamp to 30–86,400. |
| `lease_secs` | 120; file values clamp to 15–3,600. |
| `linger_secs` | 900; file/environment values clamp to 30–86,400. |
| `sandbox` | Enabled by default on macOS/Linux; missing required platform support fails closed. |
| `store_transcripts` | `true`; disabling snapshot storage is not database encryption. |

Environment overrides: `DUNLIN_RELAY_LISTEN`, `DUNLIN_RELAY_DB`,
`DUNLIN_RELAY_HARNESS`, `DUNLIN_RELAY_LINGER_SECS`, `DUNLIN_RELAY_SANDBOX`,
`DUNLIN_RELAY_STORE_TRANSCRIPTS`. Boolean true values are `1`, `true`, `yes`, `on`
(case-insensitive, trimmed); other values are false. Presence of
`DUNLIN_RELAY_DEBUG` forwards extra harness/sandbox diagnostics and can expose
sensitive details.

Non-serve commands do not uniformly load TOML or environment. `user`, `project`,
`grant`, `pair`, `devices` and `revoke` use `--db` before `DUNLIN_RELAY_DB`;
`init` and `vault` select their database from `--db` or platform default. Pass the
intended database explicitly for administration; check the implementation before
assuming that a service's `--config` affects another command.

Sources: [Config](../../apps/server/src/config.rs),
[command dispatch/serve](../../apps/server/src/main.rs).
