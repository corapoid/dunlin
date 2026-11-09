# Conversations, Graph and Trace

## Messages and history

Send with **Enter / ⌘Enter**; use **Shift+Enter** for a newline. While a turn runs,
queue, edit, reorder or remove follow-ups. Message controls support copy, retry
and edit. Queued drafts are not a durable scheduler; use
[approved deferred turns](agent-coordination.md) for fixed run/task dependencies.

Conversation history is saved locally or on the selected relay. The latest part
of a long transcript is rendered first; **Show earlier messages** expands it.
Scrolling upward pauses automatic following; **Jump to latest** or sending returns
to the live edge. Closing a tab does not delete its saved history.

**Settings → Chat → Final answer only** hides intermediate content in Chat,
without erasing the transcript or permission/error gates. **Stream responses** is
independent: an adapter that supplies text only at completion cannot be forced
to stream by the UI. Response metadata uses recorded dispatch-time model/harness
information; missing historical data stays unknown.

The composer **+** menu contains **Add photos & files…**, **Show reasoning**,
**MCP servers**, **Skills** and **Chat settings…**. Reasoning disclosure is a Chat
option, not a title-bar control; it does not change the model's reasoning budget.
Additional model options (such as reasoning effort or variants) appear in the
composer footer, with the selected value always visible, only
when the connected harness advertises their exact choices through ACP. Changes
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

## Graph and Trace

Both are projections of actual transcript events. Empty sessions show an empty
state, never demo agents, timing or cost. Graph distinguishes agents/delegated
sessions from plans/tools; disclosure, search, Fit, minimap and SVG export aid
navigation. Trace groups events under requests, supports disclosure and filters,
and keeps unknown/idle gaps separate from observed active spans.

Tab focuses a tree. Arrow keys, Home/End and PageUp/PageDown move selection;
Left/Right collapse/expand or move through the tree; Enter/Space toggles disclosure;
Escape clears selection. Filtering normalizes hidden selection. Virtualization
keeps rendering bounded. Full VoiceOver, narrow-window and both-theme acceptance
remain manual work; see [Graph/Trace QA](../quality/graph-trace-virtualization.md).

Reported context occupancy is not consumed tokens. Unknown costs/timing/usage must
not be inferred from graph shape. See [usage](usage.md).

Implementation: [chat](../../apps/desktop/src/views/chat.rs),
[transcript projections](../../apps/desktop/src/projection.rs),
[agent worker](../../crates/agent/src).
