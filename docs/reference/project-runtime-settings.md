# Project runtime defaults

Local projects expose **Project settings → Default model…**. Groups expose
**right-click group → Group settings…** (rename remains a separate action). Saving these
defaults never sends a prompt, changes a running session's model, or restarts an
agent. Projects and existing conversations are not reassigned to another harness.

The project dialog separates appearance/agent choices, which save immediately,
from the model form, which requires **Save model**. **Cancel edit** discards only
that form. Closing the dialog with
the form open asks before discarding; a pending relay save cannot be dismissed.
Group settings keeps all choices in one draft until **Save group settings**;
**Cancel** discards them. Both forms use a default-harness dropdown. Group colors
use a dropdown with colored dots; built-in group icon choices show their logos.
The project color picker shows a circular dot rather than a square swatch.

Neither form exposes permission/environment editors or a bulk reset-to-inherited
action. Existing stored permission and environment overrides remain intact when
appearance, harness or model settings are saved. **Clean project… / Clean group…**
opens the generated-data preview, not a settings reset or history deletion.

## Inheritance and sources

Effective defaults resolve in order: global harness/client permissions → root group
→ nested groups → project. Harness and permissions use the nearest explicit value;
models merge per harness and environment merges per name. The settings UI shows
the source of harness/model defaults. Group appearance can be inherited; project
icons can override it. Trust remains project-specific.

Editors contain **only explicit overrides**, not copies of ancestor values. Choose
**Inherit** for the harness or blank a model to remove that local override.
If there is no ancestor override, the harness's own
model or inherited OS/process environment remains the fallback. An explicit empty
`NAME=` overrides an ancestor with an empty value; deleting an inherited variable
from the process entirely is not supported yet.

Ancestor changes do not materialize new project rows or rewrite existing
conversations. Hidden legacy permission/environment defaults still participate in
inheritance; removing their controls does not relax existing approval policies.

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

The following documents the retained storage/runtime format, not a field in the
project/group settings windows. These windows no longer edit environment values.

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
credentials. Local values are not attached to prompts, transcripts or Git files.
For a remote project, relay-owned environment defaults are retained when an
administrator saves a model. The relay applies its validated environment to the
next harness process. Relay settings are stored unencrypted in the relay
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
- Save `mock-b`; start a new conversation: the model shows Mock B.
- Change defaults while connected: model/process stays unchanged. New local
  conversation uses new defaults; reopening an old session never sets its model.
- Cancel/Done/Esc do not save the model draft. Reopen confirms prior values.
- A blank model clears that model override; changing harness preserves other
  harness models, icon, environment and permission mode.
- Unknown default model blocks activation and prompts; clearing it allows recovery.
- Save a different allowed model while connected, then explicitly Apply saved
  model: current selector updates through ACP without a new session or prompt.
  Busy, unsupported/filtered and rejected selections must not force a fallback.
- Tab/Shift-Tab reach dropdown/input/save/cancel; Esc closes the modal and returns
  focus. Check scrolling at minimum window height in light and dark themes.
- Remote project as an admin: save a model, reconnect, and verify the relay's
  existing env is preserved while no local Mac env is used. As a member, verify
  saving is refused. Check that reserved `DUNLIN_*`,
  `DYLD_*`, `LD_*`, `PATH`, `BASH_ENV`, `ENV` and `ZDOTDIR` keys are not applied.
- Create parent/child groups, set different harness/model defaults, then move
  a project between them. Check per-value sources and that editors stay override-only.
- Set Read-only in one member's effective defaults: its existing group conversation
  must remain Read-only even if another member uses Auto-approve.
- Delete and Undo a group with defaults: promoted projects temporarily inherit the
  parent's settings, then regain eligible original ancestry. Later child settings,
  memberships and existing conversations must not be overwritten.
