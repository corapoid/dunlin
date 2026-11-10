# Agents, models and accounts

An **ACP harness/adapter** connects Dunlin to an agent over stdio. A provider
account pays for or authorizes the model. Installing an adapter, signing into a
CLI and choosing an agent mode are separate operations.

## Welcome

Welcome hides the sidebar and workspace controls. Choose the **default harness**
using its radio indicator; the down arrow reveals the rest of the catalog. No
scan or separate Set default action is needed here. Detection refreshes when
returning to the app. Back from the project step is a left-arrow icon.

If Claude Code or Codex has a CLI but lacks its ACP adapter, a footer explains
that Continue authorizes installation **after choosing the first local project**.
Selecting a radio alone never installs anything; Back withdraws pending intent.
The pinned official npm adapter goes into Dunlin's private data directory, not
system-global npm directories. Node.js 22+ and npm are prerequisites; dependency
lifecycle scripts are disabled. Existing managed adapters are discovered on later
launches. Authentication/sign-in is still owned by the harness.

A dedicated setup window shows installation, then an empty ACP connection check
without a model prompt. Success remains visible for about two seconds, then the
window closes and the workspace fades in (respecting Reduced Motion). Failures
remain visible with Retry, Choose another harness and Skip setup. Interrupted
setup requires Retry instead of silently installing on restart. A remote first
project uses its relay; it never causes a local adapter install.

## Connect locally

Use **Settings → Agents** to scan, open setup instructions or choose a default.
Each agent has one action: **Install → Import
sessions → Set default → Default**. Agents without a supported history reader skip
the import step. **Set default** verifies an empty ACP session in a trusted local
project without sending a prompt, then saves the default only on success.
Detection alone does not prove authentication or ACP compatibility. Existing
conversations keep their agent. **Scan again** is read-only discovery.

Detected/configured agents are listed first. Expand **Set up another agent** to
browse agents that are not installed. **Import sessions** reads local history
without starting an ACP session. Install the ACP adapter first. After importing,
the second line shows the number imported and last import time; the refresh icon
checks for new histories and metadata without changing source files. Older imports
show an unknown import time until their first refresh.

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
instructions. Existing legacy Custom ACP configurations remain readable, but the
custom-command form is no longer shown. Repository-local executables are not
automatically trusted. Remote installation/authentication
belongs to [server management](../administration/relay-setup.md).

## Models, modes and defaults

The composer offers models and agent modes advertised by the connected harness.
Not every adapter offers model switching or the same modes. Tool permissions
(Ask, Read-only, Auto-approve) are client policy, not the agent's Plan/Build mode.

**Shift+Tab** cycles the advertised modes in the active Chat document, whether
shortcut hints are shown or hidden. It also works when focus is on the Chat
surface rather than inside the composer. Editors, terminals, settings and open
menus retain their normal key behavior. The selected mode changes only after the
harness confirms it. Model-option controls show the selected value (for example
Default); the dropdown heading gives the option name (for example Effort).
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

Choosing **OpenCode Free** opens a separate privacy window before changing the
provider filter. Read the disclosure, explicitly enable the consent switch and
choose **Save consent**; Cancel leaves the previous provider unchanged. Some free
endpoints retain request data or use it for training; policies differ by model.
The window links to the provider's current privacy policies and Console settings.

This is **local permission in Dunlin on this Mac**, not a change to or verification
of your OpenCode Console account/organization setting. If an endpoint requires
an account-level training opt-in, enable it separately in
[OpenCode Console](https://opencode.ai/console); Dunlin has no documented API to
change that setting. Saving permission sends no prompt and never changes models.
Review or revoke it in **Settings → Agents → OpenCode Free privacy**. Revocation
blocks new Free sends (including queued requests), but cannot recall shared data,
cancel an already sent request or revoke provider-owned consent. This permission
defaults off and is excluded from portable settings backups.

The provider selector lists connected accounts reported by OpenCode for the
current project, not every provider in its integration catalog. Expired accounts
are excluded. Free appears only when zero-priced models have been discovered.
Use **Connect an integration…** to search the full installation-provided catalog,
see connected-account counts and advertised sign-in methods, and connect another
provider. Integrations requiring custom forms or external setup are identified
without guessing their configuration.

Provider-menu sign-in uses the verified local OpenCode service. Provider accounts
are managed from the composer, not duplicated in Settings. Authentication is persisted by OpenCode, not
as a Dunlin credential. Refresh providers/models after external changes. Selecting
a provider filter does not silently change the current session's model.

Other harnesses manage their accounts through their own clients. Remote projects
do not use the Mac's OpenCode account catalog. [Account limits](usage.md) are not
proof of subscription entitlement or a guaranteed spending cap.

Implementation: [catalog and discovery](../../apps/desktop/src/harness_discovery.rs),
[provider auth](../../apps/desktop/src/provider_auth.rs),
[providers](../../apps/desktop/src/providers.rs).
