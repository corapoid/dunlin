# Troubleshooting

Preserve data and collect scoped evidence before reinstalling, resetting or
changing credentials. Do not publish raw tokens, prompts or provider logs.

| Symptom | Check / safe next step |
| --- | --- |
| Build fails | Confirm supported macOS/Apple Silicon and Rust ≥1.92; use the first actual compiler error, not a downstream failure. See [installation](../getting-started/installation.md). |
| Agent CLI found but no connection | A CLI and ACP adapter are different programs. Open Settings → Agents → Set up…, scan again, then test connection. Authenticate in the agent's supported client. |
| Model/default rejected | Refresh advertised models and review project/group inheritance. Do not silently substitute another paid model. See [runtime defaults](../reference/project-runtime-settings.md). |
| Resume fails | Keep saved history; start a new session when prompted rather than claiming provider context was restored. |
| MCP tools absent | Verify the feature is enabled/granted and the harness received its config. Credential-broker changes may need a harness reconnect; do not broaden permissions blindly. |
| Remote operation unavailable | Check named host, device role, project grant and relay version. Missing APIs do not authorize a local fallback. |
| Usage unavailable or stale | Check environment/harness/provider scope and cached labels. Local OpenCode Go/Console limits need a running 2.2+ service. Missing accounting is unknown, not zero. |
| File/Git operation blocked | Save/close dirty buffers, finish active agents/operations and re-preview current state. Do not force-reset or discard files to bypass a guard. |
| macOS permission unchanged | Use Settings → Permissions → Recheck and macOS System Settings. Dunlin cannot self-grant TCC. |
| Native QA cannot open a window | Run on an interactive desktop; headless failure is not an accepted skip. Use the [QA runner](../quality/test-quality.md). |

## Collect a report

**Settings → Diagnostics & About → Troubleshooting** produces a copyable report
including data directory, schema/quick-check, project/session counts, harness
discovery, macOS permissions and remote hosts. **Agent logs → Export ZIP…**
includes current/rotated lifecycle diagnostics. Protocol payloads are omitted and
known credentials are redacted, but review adapter stderr, local paths and
personal data before sharing.

State the app version/commit, OS/architecture, local versus relay scope, agent
and model, exact steps, expected/actual result and whether the issue reproduces
with a separate data directory. Add named GUI scenario evidence if applicable.

Raw `DUNLIN_TRACE_HARNESS`/`DUNLIN_RELAY_DEBUG` output has a different privacy
boundary from the redacted export; enable only when needed and inspect locally.
See [configuration](../reference/configuration.md).

Do not use [factory reset](factory-reset.md) until you understand its irreversible
scope and have a [backup](data-and-recovery.md). Report vulnerabilities privately
through [SECURITY.md](../../SECURITY.md).
