# Skills and AI-assisted commits

## Available and installed skills

Use **Settings → Skills** to maintain a persisted instruction library, discover
existing skills, import a folder or fetch a public GitHub `SKILL.md`. Review the
instructions before installation: an agent may execute tools they describe.

**Installed skills** lists skills with global or project installations.
**Available skills** combines bundled skills with saved or imported instructions
that are not installed yet. Use **Install globally** directly; there is no separate
“Add to library” step. After uninstalling, a skill returns to Available skills.
Creation, folder scanning and imports are grouped below these two sections.

Folder import supports bounded multi-file UTF-8 bundles and never follows
symlinks. GitHub-link import fetches only `SKILL.md`; use a folder for scripts,
references and other supporting files. Installation is explicit and does not
install an agent, sign in or send a prompt.

Use **Install globally** once, regardless of the selected harness. The canonical
bundle lives in `~/.agents/skills/dunlin-<id>/`. Other supported harnesses discover
that same bundle through directory links, so updates include all companion files
without storing separate copies.

| Harness | Global discovery directory |
| --- | --- |
| OpenCode | `$XDG_CONFIG_HOME/opencode/skills` or `~/.config/opencode/skills` |
| Claude Code | `$CLAUDE_CONFIG_DIR/skills` or `~/.claude/skills` |
| Codex | `~/.agents/skills` |

Library edits do not silently update installed files; use **Update global
installation**. Unchanged Dunlin-managed legacy global copies are replaced with
discovery links during this explicit update. Unmanaged files, unrelated links and
external edits are preserved; conflicts are reported, not overwritten. Removal
deletes only recorded discovery links and hash-checked managed files, never an
entire folder recursively. Existing project copies remain untouched and may
override global definitions; **Remove project copies** explicitly removes only
their unchanged managed files. A running agent may need a new session to discover
newly installed skills. Custom harnesses must be configured to discover the shared
directory themselves; installation does not claim that an agent loaded a skill.

## Conversation skills

The composer's **+ → Skills** menu attaches reviewed
bundled or imported instructions to the current conversation without a global
installation. Selections are saved per conversation on this Mac. Other
conversations do not inherit them.

Each subsequent message includes the selected `SKILL.md` and its supporting text
files. Up to eight skills and 256 KiB of combined instruction text are allowed;
missing or oversized bundles stop sending with an explanation. A queued message
keeps the instruction snapshot selected when it was submitted. Removing a skill
affects future messages, not instructions already in the agent's context.
No scripts are executed, files installed or permission grants changed by attaching
a skill. Instructions sent to a remote conversation cross to that selected relay
as prompt context; they are not installed on the server.

## AI-assisted commits

When local authentication and the last dispatched harness/model can be verified,
Source Control offers **Commit on AI**. Otherwise use the manual commit form;
Dunlin does not silently select another model/provider.

After confirmation of paid-token/commit permission, a separate conversation uses
the bundled editable **Conventional commits** skill. It reviews repository policy,
recent history and coherent staged scope; it must not push, rewrite history,
commit secrets or bypass hooks. Its instructions are injected without implicitly
installing global files.

The flow requires a trusted local repository, saved buffers, changes and no
conflicting Git/agent operation. Normal approval gates remain. Competing Dunlin
mutations are blocked, but external editors/Git clients are not sandboxed. Remote
projects and unsupported/custom authentication retain manual commits. Restart
does not resume a pending AI commit request.

For audit-specific skill bundles, see [security audits](security-audits.md).

Sources: [skill management](../../apps/desktop/src/skills.rs),
[AI commits](../../apps/desktop/src/ai_commit.rs),
[bundled skills](../../assets/skills).
