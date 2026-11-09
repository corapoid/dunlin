# Command interfaces

Run examples from the repository root. Use built binaries' `--help` and the
source references below when an interface changes. This is not a general
`dunlin <folder>` launcher: positional file/folder arguments are rejected.

## Desktop

```sh
cargo run -p dunlin
target/debug/dunlin --help
target/debug/dunlin --version
target/debug/dunlin --syntax-notices
```

`--mcp-vault` and `--mcp-coordination` are app-managed stdio server entry points,
not GUI launch options. Do not start them expecting an interactive chat client.
`--vault-keychain-smoke` creates/deletes a real temporary Keychain item and is not
a routine read-only diagnostic.

### Debug-only native checks

```sh
cargo build -p dunlin -p dunlin-agent --bin dunlin --bin mock_acp_agent --locked
target/debug/dunlin --list-native-smokes
python3 scripts/release_qa.py --native-only --tmp-root "$TMPDIR"
python3 scripts/release_qa.py --native-only --native-filter 'gui-*' --tmp-root "$TMPDIR"
```

The JSON registry is authoritative for flags, fixture prefixes, optional
`--light-narrow` support and timeouts. Scenarios generally require a **new absolute
directory** with a scenario-specific `dunlin-…-smoke-` prefix. They need a visible
desktop; headless absence is not a silent pass. Release builds reject these flags.
`--github-smoke` has been removed. See [GUI scenarios](../quality/gui-scenarios.md)
and [test quality](../quality/test-quality.md), not old archive command lists.

Sources: [validation/registry](../../apps/desktop/src/cli.rs),
[dispatch](../../apps/desktop/src/main.rs), [runner](../../scripts/release_qa.py).

## Relay

```sh
cargo build -p dunlin-relay --locked
target/debug/dunlin-relay --help
```

| Command | Purpose / caution |
| --- | --- |
| `init [--db PATH]` | Create the database/admin and an initial pairing code. Do not use an existing database as a fresh fixture. |
| `setup [--name NAME] [--db PATH]` | Explicit managed server/tool-home setup; may require fresh provider sign-in on migration. |
| `user add NAME [--admin]`, `user list` | Manage relay roles; host Git accounts prohibit adding non-admin members. |
| `project add NAME PATH [--user USER]`, `project list` | Register a canonical server directory and explicit grants. |
| `grant PROJECT USER` | Grant access; not OS isolation. |
| `pair USER [--ttl MINUTES]` | Issue a one-use code, clamped to 1–15 minutes. Treat as a credential. |
| `devices USER`, `revoke DEVICE_ID` | Inspect/revoke paired devices. |
| `serve [--config PATH] [--db PATH] [--listen ADDR] [--harness CMD]` | Start HTTP/WebSocket server; environment overrides these serve flags. |
| `vault add/list/grant/revoke/remove` | Manage the relay's separate credential vault; values are read interactively, not pasted in argv. |

Use `--db PATH` explicitly for administration of a non-default database. The
commands do not uniformly inherit `serve --config`; see [configuration](configuration.md).
[Relay setup](../administration/relay-setup.md) covers installers, Linux keyrings,
authorization, transfers and deployed-host acceptance.

Source: [relay main](../../crates/relay/src/main.rs).

## Knowledge MCP and pack tooling

```sh
./scripts/build-governance-plugin.sh
cargo run -p dunlin -- --governance-smoke dist/governance
cargo run -p dunlin-knowledge-mcp -- --project . --plugin dist/governance
```

The MCP command serves JSON-RPC on stdin/stdout; it is not an automatic test pass.
The build command writes build output. The optional install script writes to the
user's plugin directory. Read [plugin development](../development/plugins.md)
before installing/enabling a pack.
