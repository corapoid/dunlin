# Skills and AI-assisted commits

## Skill library

Use **Settings → Skills** to maintain a persisted instruction library, discover
existing skills, import a folder or fetch a public GitHub `SKILL.md`. Review the
instructions before installation: an agent may execute tools they describe.

Folder import supports bounded multi-file UTF-8 bundles and never follows
symlinks. GitHub-link import fetches only `SKILL.md`; use a folder for scripts,
references and other supporting files. Installation is explicit and does not
install an agent, sign in or send a prompt.

| Harness | Default global directory |
| --- | --- |
| OpenCode | `$XDG_CONFIG_HOME/opencode/skills` or `~/.config/opencode/skills` |
| Claude Code | `$CLAUDE_CONFIG_DIR/skills` or `~/.claude/skills` |
| Codex | `~/.agents/skills` |

Project-scoped installation targets the current local project. **Other folder…**
exports for a custom harness. Library edits do not silently update installed
copies. Hash-checked updates/removals preserve unmanaged files, external edits and
symlinks; uninstall is not recursive directory deletion. A running agent may
need a new session to discover newly installed skills.

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
