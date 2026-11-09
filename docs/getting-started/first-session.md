# Your first session

## 1. Open a folder

Launch Dunlin and choose **Open folder…**. Select a repository or an ordinary local
folder. A repository is not created automatically. Only trust code you intend to
allow an agent to access; untrusted projects cannot start an agent runtime.

For several repositories, use [project import](../user-guide/project-import.md).
Groups organize projects without moving their files.

## 2. Configure an agent

Open **Settings → Agents** with **⌘,**. Detection distinguishes a CLI from an ACP
adapter: being signed into a CLI does not install its adapter. Follow **Set up…**
for a missing adapter, then **Scan again** and **Test connection**.

The connection test opens an empty ACP session; it does not send a test prompt or
sign you into a provider. Start with the agent's normal approval prompts enabled.
Choose a model advertised by the connected adapter. See
[agents and accounts](../user-guide/agents-and-models.md) for commands and model boundaries.

For a remote project, pair in **Settings → Remote & SSH** and manage the named
server. Local agent/account settings do not configure its harness.

## 3. Send a prompt

Describe one task in the composer. **Enter** or **⌘Enter** sends; **Shift+Enter**
inserts a newline. Agent mode and tool-permission mode are different controls.
Review permission requests before approving tool use. Local agents run with your
OS permissions; a project trust flag is not an operating-system sandbox.

While a response runs, you can queue a follow-up. **Graph** and **Trace** show
observed transcript events, not estimated or demo work. Completion means the turn
ended, not that its code is correct; inspect the diff and run appropriate checks.

## 4. Return to your work

Conversation history is persisted. Closing a tab hides it rather than deleting
the conversation. Drafts and pane layout have different lifetimes from saved
history; see [workspace](../user-guide/workspace.md) and
[session tabs](../user-guide/session-tabs-and-ui-preferences.md).

Use the [user guide](../user-guide/README.md) for editing, Git, terminals,
credentials, knowledge and agent coordination.
