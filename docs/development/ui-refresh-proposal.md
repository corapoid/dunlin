# Full Workbench — feature-preserving UI refresh

**Design proposal, revision 2 — not the shipped native interface.** Prepared
2026-11-09. No production layout, native menu or application capability is changed
by this document or its browser prototype.

## Correction to the first direction

The first “Quiet Workspace” study was too narrow. It presented a conversation
application with three destinations rather than the full Dunlin IDE. Large empty
reading areas and a small context panel did not account for the actual product's
projects/groups, independent document panes, repository tools, specifications,
knowledge, security, coordination and server administration.

**That reduction is rejected.** Simpler presentation must not mean a simpler
product, fewer tools, a chat-first replacement or hiding every advanced workflow
inside a palette. Revision 2 starts from the existing functional scope, without
copying or using the current menu arrangement as its information architecture.

## Open the full-workbench prototype

```sh
open docs/development/ui-concepts/quiet-workspace.html
```

[Interactive study](ui-concepts/quiet-workspace.html) ·
[Machine-readable preservation inventory](ui-concepts/workbench-capabilities.js)

The original filename is retained so existing links open the corrected study.
The default view now shows **project/group navigation, an editor next to an agent,
independent document tabs and a terminal dock** — not a chat landing page.

All content is illustrative. No agent, terminal, Git command, remote connection,
filesystem write, credential access or provider request is performed. A workflow
preview demonstrates placement and boundaries, not an implemented operation. The
actual application already implements the capabilities described in the linked
user guides; this study does not reimplement their backends in JavaScript.

### Light and dark previews

Browser composition studies, **not native application screenshots**:

![Full-workbench light study: project/group navigator, Rust editor, agent conversation and tools dock](ui-concepts/quiet-workspace-chromium-light.png)

![Full-workbench dark study with the same capabilities and document hierarchy](ui-concepts/quiet-workspace-chromium-dark.png)

## Non-negotiable design contract

1. **Dunlin stays a native agent IDE.** Rust/GPUI and the existing AppKit bridges;
   no browser runtime and no SwiftUI rewrite.
2. **Do not delete capabilities to make the screenshot calmer.** Every family in
   the preservation matrix below needs a concrete, discoverable home.
3. **Keep independent work visible together.** Two document panes, agent sessions,
   files and the tools dock remain separate surfaces with retained state.
4. **Do not force the whole product into Work / Changes / Activity.** Chat, Graph
   and Trace are explicit conversation lenses. Git, Knowledge, Spec and Security
   remain workspaces/documents in their own right.
5. **Preserve scope and safety contracts.** Local/relay/SSH, group/project/session,
   saved/unsaved and evidence/claim must stay distinguishable.
6. **Refresh presentation, not semantics.** Existing data, actions, shortcuts,
   defaults, drafts, histories, confirmations and capability checks migrate before
   replacing any old surface. A design proposal is not permission to remove them.

## New window composition

```text
┌ macOS title bar · project ▾ · branch · quick open · named host ▾ · inspector ┐
│ Navigator           │ Document pane A          │ Document pane B           │
│ Projects / Files /  │ workbench.rs · notes.md  │ agent session tabs        │
│ Sessions            ├─────────────────────────┤ Chat / Graph / Trace      │
│                     │                         │ Context Review Checks     │
│ Groups & projects   │ Editor / Markdown /     │ Budget Compare Impact     │
│ Local & remote      │ Git / Knowledge / Spec  │                           │
│ Worktrees           │ Security / Artifacts    │ Transcript / tool cards   │
│ Separate chat spaces│                         │ approvals / plans / queue │
│                     │                         │                           │
│ Project workspaces  │                         │ Agent · model · mode      │
│ Git · Knowledge     │                         │ Composer · permissions    │
│ Spec · Artifacts    ├─────────────────────────┴───────────────────────────┤
│ Security            │ Tools dock: Terminal / Run / Agents / Checks       │
│ Settings · Servers  │ Session tabs · splits · output · reports / inbox    │
└─────────────────────┴─────────────────────────────────────────────────────┘
```

The diagram shows a useful starting arrangement, not fixed content types for the
two panes. The native implementation must keep the current general-purpose pane
model: files, conversations and supported document tabs can be opened, moved and
closed independently. The browser study illustrates an editor/agent pairing; it
does not implement the native tab-drag/buffer/PTY lifecycle.

### 1. Unified native toolbar

Keep genuine macOS traffic lights, window dragging, keyboard focus and menus.
Expose project, branch/worktree, quick open and **named execution host** without
turning the title bar into a row of every application tool. Opening a host manager
does not retarget a live conversation. Selecting a remote project does not convert
local preferences or the Mac's Keychain into remote settings.

### 2. One navigator with three explicit lenses

- **Projects:** local/remote projects, collections, nested groups, worktrees and
  separate scratch chat spaces. Pin/sort/group/trust/default actions remain
  available, with an explicit action button as well as contextual menus.
- **Files:** the selected project's tree, filter/search, file operations and
  preview entry points. This is a useful first-class explorer, not an attachment
  list in the agent inspector.
- **Sessions:** history in the selected scope, new conversations, search,
  supported import and explicit standalone-chat creation. General, project and
  scratch histories are not merged to simplify the list.

A compact **Project workspaces** area gives labelled, persistent access to Source
Control, Knowledge, Intent & Specifications, Artifacts & Evidence and Security
Audits. Optional features respect their existing enablement settings; hidden
contributions never delete their stored documents or evidence.

### 3. Real document workbench

Use restrained per-pane tab strips and one-pixel pane boundaries. Keep canonical
file buffers, undo and dirty indicators independent of conversation runtimes and
drafts. A split is explicit and reversible; closing one pane does not close an
unrelated session, discard a buffer or stop a terminal. Preserve reading zoom,
preview/save behavior, two-pane navigation and existing close/save guards.

Git and other rich workspaces open as **document surfaces**, not tiny inspector
cards. A user can give one the full document area, or keep an agent beside it.
Do not replace the native Git graph, Spec cockpit or Knowledge relations with a
short summary just because the summary fits a narrow panel.

### 4. Full agent workspace

Keep **Chat / Graph / Trace** visibly named and directly switchable. Preserve
reasoning, tool evidence, approval cards, plans, retries, queued prompts,
background/parked sessions, model/configuration options and session history.

The existing **Context / Review / Checks / Budget / Compare / Impact** tool family
is directly reachable above the conversation, not removed or reduced to a single
generic information card. Context and limits can use a contextual inspector;
review, comparison and impact need appropriately sized workbench documents.

The composer keeps agent, advertised model/mode options, permissions, resources,
conversation skills and advertised MCP controls. Unsupported configuration stays
unsupported; the redesign does not invent modes or change execution policy.

### 5. Tools dock, not disposable console output

**Terminal / Run / Agents / Checks** have labelled dock tabs. Terminal sessions,
splits, search, selection, shell integration, local/relay/SSH origins and dock
placement/resizing survive presentation changes. Closing the dock hides its view;
closing a terminal session retains its existing process-lifecycle semantics.

Run retains static manifest discovery and exact-command confirmation. Agents
retains observed peers, queue distinctions, reports, inbox, tasks and exact
deferred-turn approval. It is not a new autonomous agent factory or task-DAG
executor. Check output, diagnostics, exit/failure/cancellation and freshness stay
inspectable rather than becoming a green status dot.

### 6. Inspector for context, not a place to hide the application

Use an on-demand 276–320 pt context surface for scope, effective sources,
decision notes, selected items and budget details. It is **additional** to the
workbench, not a replacement for Explorer, Git, Spec, Knowledge, audits or server
administration. Project/group/session settings display their inheritance sources.

### Menus and discoverability

Keep a complete **native system menu bar**, without making it the only route to
specialized functionality. Proposed responsibilities, not a copy of today's menu:

| Menu | Responsibility |
| --- | --- |
| Dunlin | About, Settings, Services, Hide and Quit |
| File | Open/add project, new document/conversation, save and guarded close |
| Edit | Undo/redo, selection, clipboard and Find |
| View | Navigator lenses, document panes, Chat/Graph/Trace, inspector, dock and reading zoom |
| Project | Project/group defaults and trust; repository changes/history/branches/worktrees and explicit Git operations |
| Conversation | Verified agent/session options, context/resources, queue, stop/resume and history |
| Tools | Terminal, Run, Agents, Checks; Knowledge/Spec/Security/Artifacts workspaces; named server administration |
| Window / Help | Standard window behavior, documentation and searchable commands |

The labelled navigator, conversation tool row and dock expose frequent commands
directly; the palette accelerates them but is not their only entry point. Item
context menus contain item actions, not unrelated global tools. New-document
choices have an explicit menu button, not a required long press. Native shortcut
and localization audits must preserve existing access and resolve conflicts
before changing bindings. Disabled commands name their missing prerequisite and
selected scope; simplified wording never skips a confirmation or merges distinct
operations such as creating a branch and checking it out.

## Preservation matrix

This is a map of **existing capability families**, not a new feature backlog.
The [prototype inventory](ui-concepts/workbench-capabilities.js) drives its
searchable **Capabilities** map; each record opens an identified route and links
to a current guide. Mapped does not mean a native redesign is implemented or that
every leaf command is already represented by a fully working browser control.

| Existing capability | Concrete home in the refreshed UI | Current contract |
| --- | --- | --- |
| Dashboard, activity, recent project visits | Overview document | [Workspace](../user-guide/workspace.md) |
| Local/remote projects, collection/import, watched folders | Projects navigator + Add/import | [Project import](../user-guide/project-import.md) |
| Nested groups, pin/sort, trust and inherited runtime defaults | Projects + explicit project/group settings | [Workspace](../user-guide/workspace.md), [runtime defaults](../reference/project-runtime-settings.md) |
| Recoverable generated-folder cleanup | Projects → Cleanup preview | [Workspace](../user-guide/workspace.md) |
| Standalone chats and scratch roots | Distinct Chat spaces section + Sessions | [Standalone chats](../user-guide/standalone-chat-and-worktrees.md) |
| Explorer, file/folder mutations, filter/search | Files navigator + Project Search document | [Workspace](../user-guide/workspace.md) |
| Editing, save/undo, two panes, independent tab state, zoom | Document panes and per-pane controls | [Tab lifecycle](../user-guide/session-tabs-and-ui-preferences.md) |
| Code, minimap/blame and image/PDF previews | Editor / preview documents | [Editor](../user-guide/editor.md) |
| Visual/raw Markdown, blocks/tables/fences | Markdown document, same backing buffer | [Markdown](../user-guide/markdown-editor.md) |
| Conversation history and supported imports | Sessions + Agents settings | [Session import](../user-guide/session-import.md) |
| ACP readiness, accounts, default/visible adapters | Agents settings; scope-aware composer | [Agents](../user-guide/agents-and-models.md) |
| Model/mode/configuration, reasoning/tools, queue/retry/park | Conversation pane and composer | [Conversations](../user-guide/conversations.md) |
| Graph and Trace, recorded session usage | Explicit conversation lenses | [Conversations](../user-guide/conversations.md), [usage](../user-guide/usage.md) |
| Conversation resources, skills, MCP exclusions | Composer context menu + Context | [Skills](../user-guide/skills.md), [conversations](../user-guide/conversations.md) |
| Context envelope and revisioned decision memory | Context inspector | [Agent workbench](../user-guide/agent-workbench.md) |
| Checkpoints, saved-file baseline and guarded restore | Task Review document | [Agent workbench](../user-guide/agent-workbench.md) |
| Exact-command checks and diagnostics | Checks dock + reviewed command preview | [Agent workbench](../user-guide/agent-workbench.md) |
| Task request/time/USD gates; stop/release/forget | Budget inspector | [Agent workbench](../user-guide/agent-workbench.md) |
| A/B detached worktrees and separate agent approvals | Comparison document | [Agent workbench](../user-guide/agent-workbench.md) |
| Rust reference/dependent/candidate-test impact | Impact document | [Agent workbench](../user-guide/agent-workbench.md) |
| Observed peers, reports, messages, tasks, dependencies | Agents dock and scoped details | [Coordination](../user-guide/agent-coordination.md) |
| Exact deferred-turn review/approve/cancel, inbox acknowledgment | Agents dock → specific review | [Coordination](../user-guide/agent-coordination.md) |
| Stage/unstage, diff, commit/amend, pull/push, AI commits | Source Control → Changes | [Git](../user-guide/git.md), [AI commits](../user-guide/skills.md#ai-assisted-commits) |
| Branch graph/history/parents, refs and patches | Source Control → Graph & history | [Git history](../user-guide/git-history.md) |
| Stash, worktrees, conflict/sequencer operations | Source Control → Worktrees / State | [Git operations](../user-guide/git-operations-and-blame.md) |
| Registered subtask worktrees, create/open/remove | Source Control → Worktrees; Projects links | [Worktrees](../user-guide/standalone-chat-and-worktrees.md) |
| Knowledge objects, relations, canonical Markdown | Knowledge document | [Knowledge](../user-guide/knowledge.md) |
| Pending knowledge writes and accept/reject | Knowledge → Proposals | [Knowledge](../user-guide/knowledge.md) |
| Intent inbox/capture, Spec phases, questions, gates/drift | Intent & Specifications document | [Spec](../user-guide/spec-workspace.md) |
| Requirement/task/worktree/session evidence and completion | Specifications → Traceability / Tasks | [Spec](../user-guide/spec-workspace.md) |
| Agent contract, hash-bound decisions and heuristic audit | Specifications → Agent contract | [Spec](../user-guide/spec-workspace.md) |
| Artifact capture, local/remote preview/export/share/revoke | Artifacts & Evidence document | [Artifacts](../user-guide/artifacts.md) |
| Audit readers, findings, coverage, triage and exports | Security Audits document | [Security audits](../user-guide/security-audits.md) |
| Separately confirmed sandboxed local check | Security Audits → explicit workflow | [Security audits](../user-guide/security-audits.md) |
| Stored/use-only keys, policy/HTTP/CLI recipes/activity | Credentials settings and bounded approval | [Credentials](../user-guide/credentials.md) |
| Agent approval boundaries and macOS TCC | Permissions settings; inline exact approvals | [Settings](../user-guide/settings.md) |
| Local/relay/SSH terminal, tabs/splits/shell integration | Terminal dock (and supported document tabs) | [Terminal](../user-guide/terminal-and-run.md) |
| Static tasks, reviewed argv/cwd/env, output/cancel | Run dock | [Run](../user-guide/terminal-and-run.md) |
| Relay pairing/grants/leases, host accounts/tools, clone/copy | Named host → Server Administration | [Relay](../administration/relay-setup.md) |
| SSH hosts, passphrases, host keys, bootstrap/tunnels | Server Administration + Remote & SSH | [SSH](../administration/ssh.md) |
| Session usage, account limits, attribution/billing, sync | Trace, status entry and Usage & Limits | [Usage](../user-guide/usage.md) |
| Shared skill installation, companions, updates and scopes | Skills settings + conversation attachments | [Skills](../user-guide/skills.md) |
| WASM packs, capabilities, explicit install/enable, builder | Extensions settings + contributed workspaces | [Plugins](plugins.md) |
| Settings/project export/import, recovery, diagnostics/logs | Data & Backup / Diagnostics & About | [Settings](../user-guide/settings.md) |

**All 15 settings pages remain accounted for:** General, Appearance, Workspace,
Chat, Editor & Terminal, Agents, Skills, Extensions, Permissions, Credentials,
Security Audits, Remote & SSH, Usage & Limits, Data & Backup, Diagnostics & About.
The prototype searches and opens all 15. They are not collapsed into six
nonfunctional category descriptions. Search aliases, source labels, Save/Cancel,
dirty-form guards and immediate-save preferences need native parity before a
rollout, even if the final grouping changes.

Do **not** add removed PR/Actions integration, autonomous worker orchestration,
scheduled automations, global transcript FTS or generic native plugin UI to make
the prototype appear comprehensive. These are not current supported capabilities;
see [product boundaries](../project/status.md).

## Visual refresh: lighter, not emptier

- Preserve a productive information density: 12–13 pt navigation, 13–15 pt
  conversation reading, readable system-monospace code. Avoid the first study's
  oversized promotional heading and large empty chat canvas.
- Use a 4 pt spacing rhythm, 8–12 pt related-control gaps and 16–24 pt section
  separation. Differentiate document, navigation and transient surfaces without
  surrounding every line with a card.
- Give both themes deliberate semantic tokens. Reserve system blue for focus,
  selection and the next meaningful action. Preserve text/shape alongside status
  colors. Unknown data remains explicit.
- Keep tab titles, filenames, roots and agent identity stable. Do not animate or
  morph technical text while a user is reading/editing it.

## Apple materials and components

Reuse [the AppKit effects bridge](../../apps/desktop/src/effects/backdrop.rs):
runtime-checked `NSGlassEffectView` on macOS 26+, `NSVisualEffectView` fallback on
the supported macOS 15 baseline. Keep native title bars, menus, dialogs, open/save
panels, clipboard and text conventions; supported Apple symbol APIs can coexist
with bundled-icon fallbacks. Do not redistribute SF Symbols fonts/artwork.

**Material budget:** title bar/navigation and transient overlays. Code, diffs,
terminal text, transcripts, controls and approval details stay opaque. No
wallpaper shining through code or multiple stacked blur layers. A browser blur is
only a composition approximation; it does not demonstrate native Liquid Glass.

Respect the [platform Reduce Transparency preference](../../apps/desktop/src/platform.rs)
with opaque semantic surfaces. Inactive-window material, bright/dark wallpapers,
increased contrast and actual native hit-testing need platform acceptance.

## Motion, adaptive layout and accessibility

Use 90–120 ms hover/press transitions, 140 ms document entrances, 160 ms popovers
and 180–220 ms pane reveals. Preserve scroll anchors, selected files, active
sessions and keyboard focus. No perpetual pulse, animated code glyphs, streaming
paragraph reflow or blur-radius animation. Start at rest and stop requesting
frames after a transition settles.

System **OR** application Reduce Motion disables nonessential movement. Reduce
Transparency is independent. Both work in both themes. Text, not animation, must
communicate pending approvals, stale evidence and execution state.

At wide sizes, show the editor/agent pairing and dock. Below useful split widths,
switch between the retained panes rather than squeezing two unusable columns;
keep an explicit Agent action and document tabs. At narrow sizes, navigation and
inspector become dismissible drawers. All major workspaces and dock tools remain
reachable. The demo supports smaller browser sizes for review; it does not change
the native minimum window size.

Require keyboard access, labelled controls, roving tab focus, correct hidden/inert
panes, focus restoration, reading zoom, long localized names and VoiceOver.
Native parity also includes IME, file/terminal selections, per-pane focus and
command shortcut conflicts. Browser checks cannot establish those native results.

## Evaluation and implementation boundary

The inventory is a **parity gate**, not a competing roadmap. If the visual
direction is accepted, implementation belongs in the existing
[roadmap](../project/roadmap.md), in reviewable native slices:

1. Retain the current state/action models; test a presentation shell behind a flag.
2. Migrate navigation, panes and dock without dropping a row of the matrix.
3. Migrate rich workspaces and all settings forms with their exact scope/guards.
4. Apply AppKit materials and bounded shared motion only after functional parity.
5. Compare native light/dark, motion/transparency variants, small/wide windows,
   keyboard/VoiceOver, remote loss/reconnect, large histories and existing
   [performance baselines](../quality/performance.md).

### Browser verification

`node scripts/ui-concept-smoke.mjs` (also `--webkit`) uses an externally installed
Playwright, not an application dependency. `PLAYWRIGHT_MODULE` can identify its
absolute `index.mjs`; `PLAYWRIGHT_EXECUTABLE_PATH` can select an existing browser.
Optional `DUNLIN_UI_CONCEPT_EVIDENCE` names a capture directory.

The smoke validates the prototype's route inventory, all settings pages,
project/session drafts, retained demo editor buffers, conversation lenses, dock,
Git/Knowledge/Spec routes, local/remote scope labels, safe message rendering,
keyboard/dialog behavior, preference toggles and adaptive layout. It does not
execute or validate the native application's operational workflows. Native
materials, drag/drop, PTYs, real agents/servers, TCC and physical-input/VoiceOver
acceptance remain unverified by this proposal.

Revision 2 browser runs passed locally on 2026-11-09 in Chromium and WebKit,
including all mapped routes and 15 settings pages at six viewport sizes. Both
runs completed without external requests or browser console errors. This is
prototype evidence only, not a fresh native application acceptance run.

## Apple references

[Materials](https://developer.apple.com/design/human-interface-guidelines/materials) ·
[Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars) ·
[Toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars) ·
[Motion](https://developer.apple.com/design/human-interface-guidelines/motion) ·
[Accessibility](https://developer.apple.com/design/human-interface-guidelines/accessibility)

[Development index](README.md)
