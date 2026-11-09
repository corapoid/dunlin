# Project runtime defaults

Local projects expose **Project settings → Model & environment…**. Groups expose
**right-click group → Group settings…** (rename remains a separate action). Saving these
defaults never sends a prompt, changes a running session's model, or restarts an
agent. Projects and existing conversations are not reassigned to another harness.

The project dialog explicitly separates appearance/agent/approval choices, which
save immediately, from the model/environment form, which requires **Save model &
environment**. **Cancel edit** discards only that form. Closing the dialog with
the form open asks before discarding; a pending relay save cannot be dismissed.
Group settings keeps all choices in one draft until **Save group settings**;
**Cancel** discards them. Project reset requires confirmation and clears all
project overrides, not repository files or inherited group values.

## Inheritance and sources

Effective defaults resolve in order: global harness/client permissions → root group
→ nested groups → project. Harness and permissions use the nearest explicit value;
models merge per harness and environment merges per name. The settings UI shows the
source of each effective value. Icons and trust are project-only, never inherited.
Environment summaries show names and sources, not inherited values.

Editors contain **only explicit overrides**, not copies of ancestor values. Choose
**Inherit** for harness/permissions, blank a model or omit an environment name to
remove that local override. If there is no ancestor override, the harness's own
model or inherited OS/process environment remains the fallback. An explicit empty
`NAME=` overrides an ancestor with an empty value; deleting an inherited variable
from the process entirely is not supported yet.

**Reset to inherited** in Group settings resets the draft; Save confirms it.
**Reset overrides** in Project settings clears all project overrides, including its
icon, without changing the group's preferences. Ancestor changes do not materialize
new project rows or rewrite existing conversations.

Moving projects/groups, deleting a group (promotion) or Undo immediately recomputes
inheritance. This can change **client approval policy for following requests**;
group settings are not just visual organization. Existing conversation harnesses and
models stay unchanged, and running processes keep their environment. Group agents
always use the strictest effective policy of their frozen member roots; future
members are not added. Missing/cyclic ancestry blocks local connection and treats
permission policy as read-only rather than accepting partial, broader defaults.

## Model

- A model is an **ACP wire ID**, not its display name. Use the current session's
  model button to copy a known ID when that session uses the same harness.
- Defaults are stored separately for each harness. Switching the default harness
  does not reuse another harness's model ID or discard its saved preference.
- A blank model removes this override and inherits the ancestor's model (or the
  harness's default if no ancestor specifies one) for future conversations.
- Before activating a new ACP session, Dunlin checks the advertised models and
  awaits the model setter's acknowledgement. Unsupported, unavailable or rejected
  defaults block activation; no prompt is silently sent using a fallback model.
- Loading an existing ACP session never applies the project default. The harness
  remains responsible for restoring that session's model. Change its current model
  explicitly using the existing composer selector (ACP, no restart).
- For the active local project, **Apply saved model to current session** explicitly
  applies the effective default for that conversation's harness through ACP. It
  waits for an idle, connected session, respects the advertised models and current
  provider/model filter, and reports rejection without changing the saved defaults.
  It sends no prompt, restarts no process and does not change parked conversations.

## Environment

```text
LANG=pl_PL.UTF-8
MY_PROJECT_MODE=development
EMPTY_VALUE=
```

One `NAME=value` override per line, with POSIX-style names. Blank lines and CRLF input are
accepted. Values are literal: spaces, quotes, `#`, `$` and extra `=` are not
interpreted. No `export`, comment syntax, shell expansion or dotenv loading.
Duplicate names, NUL, multiline values and oversized input are rejected. Limits:
128 names and 64 KiB total, including the merged environment. Group saves validate
all affected descendant groups and local projects. Errors omit submitted values.

Values override the inherited environment of the **next local ACP process**,
including its child tools. An explicit `PATH` overrides the discovered child PATH;
adapter executable discovery still uses Dunlin's existing discovery mechanism.
This does not configure other app subprocesses (for example Git/provider discovery).
Already connected foreground/background processes keep their original environment.
An ordinary new local conversation starts its own runtime and receives current
defaults; reconnecting an old one also receives current environment overrides.

**Not a secrets store:** values are saved unencrypted in the local SQLite database
(and may appear in its WAL/backups). Use harness authentication/Keychain for
credentials, not this editor. Local values are not attached to prompts,
transcripts or Git files. For a remote project, a relay administrator can store
project runtime defaults on the relay through the same editor; the relay applies
the validated environment to the next harness process. Non-admin members see the
values but cannot save them. Relay settings are stored unencrypted in the relay
database and are never read from this Mac's local account catalog.

The desktop refreshes relay model/environment defaults when entering a remote
project and when opening its settings. Project navigation reads them before
importing the session list and automatic connection. Refreshes preserve local
appearance, harness overrides and client approval choices; an open runtime form
or a newer runtime save is not overwritten by a delayed read. This is best-effort
refresh, not revisioned cross-device synchronization: failures are logged, there
is no server CAS yet, and an open draft is not refreshed automatically. See
[the remaining sync plan](../../todo/21-relay-ustawienia-i-sync-grup.md).

Project fields use serde defaults in the existing settings JSON column. Database
schema v7 introduced a group-settings JSON column with empty defaults in a transaction;
existing groups, project settings, memberships and transcripts are preserved.

## Manual QA checklist (not yet executed)

- Fresh data directory and a disposable trusted local folder; custom ACP command
  pointing at `target/debug/mock_acp_agent`, no real provider account needed.
- Save `mock-b` and `DUNLIN_MOCK_STREAM_CHUNKS=2`; start a new conversation:
  model shows Mock B and a synthetic prompt returns two chunks without tools.
- Change defaults while connected: model/process stays unchanged. New local
  conversation uses new defaults; reopening an old session never sets its model.
- Duplicate/invalid env shows an inline error without modifying saved settings;
  Cancel/Done/Esc do not save the runtime draft. Reopen confirms prior values.
- Empty fields clear overrides; changing harness preserves other harness models,
  icon and permission mode. Reset overrides clears all project preferences.
- Unknown default model blocks activation and prompts; clearing it allows recovery.
- Save a different allowed model while connected, then explicitly Apply saved
  model: current selector updates through ACP without a new session or prompt.
  Busy, unsupported/filtered and rejected selections must not force a fallback.
- Tab/Shift-Tab reach input/editor/save/cancel; Esc closes the modal and returns
  focus. Check scrolling at minimum window height in light and dark themes.
- Remote project as an admin: save model/env, reconnect, and verify the relay
  harness receives the env while no local Mac env is used. As a member, verify
  the values are visible but saving is refused. Check that reserved `DUNLIN_*`,
  `DYLD_*`, `LD_*`, `PATH`, `BASH_ENV`, `ENV` and `ZDOTDIR` keys are not applied.
- Create parent/child groups, set different harness/model/env defaults, then move
  a project between them. Check per-value sources and that editors stay override-only.
- Set Read-only in one member's effective defaults: its existing group conversation
  must remain Read-only even if another member uses Auto-approve.
- Delete and Undo a group with defaults: promoted projects temporarily inherit the
  parent's settings, then regain eligible original ancestry. Later child settings,
  memberships and existing conversations must not be overwritten.
