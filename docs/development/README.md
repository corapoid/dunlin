# Development

The contribution entry point is [CONTRIBUTING.md](../../CONTRIBUTING.md): supported
toolchain, build commands, code style and pull-request policy. Do not duplicate
its check list here.

1. Read the [repository map and data flow](../architecture/README.md).
2. Run `cargo run -p dunlin` on a supported Mac. Use a separate
   [data directory](../reference/configuration.md) for development.
3. Use [shared controls](ui-controls.md), [surface rules](ui-surfaces.md) and
   [text motion](text-motion.md) when changing native UI.
4. Run the relevant [quality checks](../quality/README.md), then the contribution
   checks before requesting review. Fixtures do not replace physical input,
   VoiceOver, TCC or deployed-host acceptance.
5. Update the [documentation](documentation.md) with the behavior change.

## Specialized work

Headless services and CLI can be exercised without a desktop session:

```sh
cargo test --workspace --exclude dunlin --exclude dunlin-ui --locked
cargo build -p dunlin-cli -p dunlin-agent --bin dunlin-cli --bin mock_acp_agent --locked
python3 scripts/test_headless.py
python3 scripts/check_architecture.py
```

The headless runner uses only a deterministic mock ACP process, never a provider
account. Missing fixture binaries fail rather than silently skip acceptance.
CLI artifacts are uploaded separately by CI; this does not enable previously
paused desktop distribution or automatically publish a new CLI release channel.

- [Full Workbench UI proposal](ui-refresh-proposal.md) — feature-preserving layout,
  parity matrix and interactive study; replaces the rejected chat-only direction,
  not the shipped native interface.
- [Plugin manifests, WASM host and knowledge MCP](plugins.md)
- [CI/CD execution, caches and release builds](ci-cd.md)
- [Release process](../quality/release-process.md)
- [Marketing site](../../web/README.md) — static Astro site, separate from the native app.

[Documentation home](../README.md)
