# Agents, models and accounts

An **ACP harness/adapter** connects Dunlin to an agent over stdio. A provider
account pays for or authorizes the model. Installing an adapter, signing into a
CLI and choosing an agent mode are separate operations.

## Connect locally

Use **Settings → Agents** to scan, open setup instructions, test a connection,
choose a default or configure a custom ACP command. Detection does not prove
authentication or ACP compatibility. **Test connection** opens an empty ACP
session without sending a prompt. **Show in agent picker** checkboxes affect selection menus,
not installed software or saved conversations.

Select the radio beside an agent to make it the default for new conversations.
The checked radio marks the default; unavailable or unverified agents cannot be made
the default. Existing conversations keep their agent. **Scan again** remains an
explicit, read-only discovery action.

Detected/configured agents are listed first. Expand **Set up another agent** to
browse agents that are not installed. **Import history** is available only for
supported detected CLIs/adapters; it reads local history without starting an ACP
session. A CLI-only installation can be imported even when its ACP adapter is missing.

The chat harness selector lists only visible agents with a verified, ready
connection for the current project configuration. Unverified or failed adapters
stay in Settings → Agents, not as disabled choices in chat.

Common commands from the built-in catalog:

| Agent | ACP command |
| --- | --- |
| OpenCode | `opencode acp` |
| Claude Code | `claude-agent-acp` |
| Codex | `codex-acp` |
| Gemini CLI | `gemini --acp` |
| Qwen Code | `qwen --acp` |
| GitHub Copilot | `copilot --acp` |
| Cursor | `cursor-agent acp` |
| Auggie CLI | `auggie --acp` |
| Kiro CLI | `kiro-cli acp` |
| OpenClaw | `openclaw acp` |
| Hermes Agent | `hermes acp` |
| Qoder CLI | `qodercli --acp` |

The current setup catalog in the app is authoritative for adapter installation
instructions. Other commands can be configured as Custom ACP; repository-local
executables are not automatically trusted. Remote installation/authentication
belongs to [server management](../administration/relay-setup.md).

## Models, modes and defaults

The composer offers models and agent modes advertised by the connected harness.
Not every adapter offers model switching or the same modes. Tool permissions
(Ask, Read-only, Auto-approve) are client policy, not the agent's Plan/Build mode.
Contract ceilings and budgets may further restrict permissions.

Global, ancestor-group and project defaults resolve with explicit sources.
Model defaults normally affect new conversations, not resumed sessions; applying
a saved model to an idle session is explicit. Environment changes apply on the
next connection and are unencrypted configuration, **not secret storage**.
See [runtime defaults](../reference/project-runtime-settings.md).

Saved history survives a failed resume. Dunlin does not silently reset the
harness context and pretend it resumed. Changing the local harness creates a
new conversation rather than rewriting the old one.

## OpenCode providers

OpenCode Go is a provider inside OpenCode, not a separate ACP harness. The local
provider selector filters enabled models also advertised by ACP. **Free** is an
explicit filter for discovered zero-priced models, not a fallback to paid models.
Unknown pricing is excluded from the free allowlist; limits and availability are
controlled by the provider.

The provider selector lists connected accounts reported by OpenCode for the
current project, not every provider in its integration catalog. Expired accounts
are excluded. Free appears only when zero-priced models have been discovered.
Use **Connect an integration…** to search the full installation-provided catalog,
see connected-account counts and advertised sign-in methods, and connect another
provider. Integrations requiring custom forms or external setup are identified
without guessing their configuration.

Provider-menu sign-in and **Settings → Agents → OpenCode provider accounts** use
the verified local OpenCode service. Authentication is persisted by OpenCode, not
as a Dunlin credential. Refresh providers/models after external changes. Selecting
a provider filter does not silently change the current session's model.

Other harnesses manage their accounts through their own clients. Remote projects
do not use the Mac's OpenCode account catalog. [Account limits](usage.md) are not
proof of subscription entitlement or a guaranteed spending cap.

Implementation: [catalog and discovery](../../apps/desktop/src/harness_discovery.rs),
[provider auth](../../apps/desktop/src/provider_auth.rs),
[providers](../../apps/desktop/src/providers.rs).
