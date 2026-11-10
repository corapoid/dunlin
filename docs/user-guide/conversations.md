# Conversations, Graph and Timeline

## Messages and history

Send with **Enter / ⌘Enter**; use **Shift+Enter** for a newline. While a turn runs,
queue, edit, reorder or remove follow-ups. Sent user messages have no **You**
caption, copy button or edit action. Selecting conversation text with the mouse
copies the selection when released; selecting composer/editor text does not.
Failed requests are highlighted red and alone show **Retry**. Retry is an explicit
new attempt (potentially billable), preserves the failed history and any new
composer draft, and never silently chooses another model. Unrelated settings
errors and deliberate cancellation do not make completed messages retryable.
Queued drafts are not a durable scheduler; use
[approved deferred turns](agent-coordination.md) for fixed run/task dependencies.

Conversation history is saved locally or on the selected relay. The latest part
of a long transcript is rendered first; **Show earlier messages** expands it.
Scrolling upward pauses automatic following; **Jump to latest** or sending returns
to the live edge. Closing a tab does not delete its saved history.

A new chat tab in the same project inherits the current conversation's confirmed
model when it uses the same agent. Its label is visible immediately; the new ACP
session must accept that exact model before it is ready. Existing conversations
are unchanged. Without a current choice, the project/agent default still applies;
models are not copied across agents or provider changes.

Provider errors wrap within the chat surface, including long URLs. A free-model
error requiring consent to training on request data must be resolved by choosing
another model or consciously reviewing that setting in the provider account.
Dunlin does not enable that consent automatically.

**Settings → Chat → Final answer only** hides intermediate content in Chat,
without erasing the transcript or permission/error gates. **Stream responses** is
independent: an adapter that supplies text only at completion cannot be forced
to stream by the UI. Response metadata uses recorded dispatch-time model/harness
information; missing historical data stays unknown.

Click **Agent work** to fold or unfold a block of reasoning, tool calls, plans
and subagent activity independently. Replies, errors and permission requests stay
visible. The working indicator rotates while a turn is active, unless Reduce
motion is enabled.

The composer **+** menu contains **Add photos & files…**, **Expand reasoning**,
**MCP servers**, **Skills** and **Chat settings…**. Reasoning disclosure is a Chat
option, not a title-bar control; it does not change the model's reasoning budget.
It expands thought cards only when the harness has shared reasoning text. Without
that text, or with Final answer only enabled, it is disabled with an explanation.
It cannot expose private model reasoning that was never sent to Dunlin.
Additional model options (such as reasoning effort or variants) appear in the
composer footer, with the selected value always visible, only
when the connected harness advertises their exact choices through ACP. Changes
use text-only selectors and menu rows, without decorative option icons, and
are applied to that session and confirmed by the harness; unsupported levels are
never guessed. An older relay may need updating to expose these options.

Use **+ → Skills** to attach instructions to this conversation.
See [conversation skills](skills.md#conversation-skills) for scope and limits.

Session runtimes, including remote connections, can be parked in the background;
follow-ups pause at background turn boundaries. A real relay disconnect has its
own bounded linger lifecycle. Neither is a guarantee of unlimited parallel
adapters or autonomous orchestration. [Session scope](session-list.md) and
[tabs](session-tabs-and-ui-preferences.md) describe ownership and persistence.

## Permissions and MCP

Review tool requests before allowing them. Client approval policies do not sandbox
a local process and do not prove the safety of a command. Budget/contract checks
can deny a permission even when a broad approval preference is enabled.

For a supported local OpenCode service, **+ → MCP servers** shows connected
servers and lets an idle session exclude a server for subsequent turns. This
does not disconnect a shared server, rewrite global/project config or erase tool
output already in context. Other harnesses/relays can show unknown state rather
than this Mac's MCP count. Built-in [credentials](credentials.md),
[coordination](agent-coordination.md) and [knowledge](knowledge.md) tools have
their own scope rules.

## Graph and Timeline

Both are projections of actual transcript events. Empty sessions show an empty
state, never demo agents, timing or cost. Graph distinguishes agents/delegated
sessions from plans/tools; disclosure, search, Fit, minimap and SVG export aid
navigation.

In Graph, Tab focuses the tree. Arrow keys, Home/End and PageUp/PageDown move selection;
Left/Right collapse/expand or move through the tree; Enter/Space toggles disclosure;
Escape clears selection. Filtering normalizes hidden selection.

**Timeline (⌘3)** replaces the former Trace view. It shows cumulative added and
removed lines, net changes, prompt boundaries and one lane per changed file for
the whole conversation. Choose **Selected prompt** and use its arrows to focus
on one request. Zoom with +/−, drag to pan, and use **Back to live** or **Fit** to
resume following. Hover an edit to preview it; click to pin its recorded diff,
open the full historical patch, or reveal its source in Chat. Focus the chart
and use Left/Right or Home/End to inspect edits; Escape clears the selection.

Counts come from successful, reported file edits, including subagents. Pending
or failed proposals do not increase the totals, and model thinking only advances
time. Re-editing a file adds activity, so totals are **not** the final Git diff.
Binary/legacy edits without patches have unknown line counts; missing timestamps
are labeled as unrecorded. History is reconstructed from the saved transcript.
Animations honor Reduce motion. Full VoiceOver and narrow-window acceptance
remain manual work.

Reported context occupancy is not consumed tokens. Unknown costs/timing/usage must
not be inferred from graph shape. See [usage](usage.md).

Implementation: [chat](../../apps/desktop/src/views/chat.rs),
[transcript projections](../../apps/desktop/src/projection.rs),
[agent worker](../../crates/agent/src).
