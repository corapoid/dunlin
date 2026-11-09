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

- [Plugin manifests, WASM host and knowledge MCP](plugins.md)
- [CI/CD execution, caches and release builds](ci-cd.md)
- [Release process](../quality/release-process.md)
- [Marketing site](../../web/README.md) — static Astro site, separate from the native app.

[Documentation home](../README.md)
