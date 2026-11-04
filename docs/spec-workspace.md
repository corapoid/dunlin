# Spec workspace, governing artifacts and plugin packs

Dunlin treats intent, spec-kit documents and the agent contract as file-first
governing artifacts. Files remain canonical; SQLite is a rebuildable index and
evidence store, never the authority for content.

## What is indexed

Discovery is read-only and bounded (512 files, 512 KiB per file, 16 MiB per
pass, no symlink following). Local projects use the filesystem directly; relay
projects use the existing bounded file-tree and file-read endpoints in a
background task, so the window shows a loading state instead of blocking.

- `intent/**/*.md` — originator-owned intent documents.
- `.dunlin/agent-contract.md` — project behaviour contract.
- `.specify/memory/constitution.md` and `.specify/constitution.md`.
- `specs/*/`, `.specify/specs/*/`, `.specify/features/*/`:
  `spec.md`, `plan.md`, `tasks.md`, `research.md`, `data-model.md`,
  `quickstart.md` and `checklists/*.md`.
- `.specify/feature.json` / `.specify/feature` for the active feature.
- `.specify/extensions.yml` and `.specify/extensions/*/extension.yml` for
  detected Spec Kit extensions and their command names (local projects; remote
  extension metadata is not indexed yet).

External files are never rewritten. The cockpit shows `spec.md` open questions,
`FR-*` / `SC-*` / `US*` identifiers, `T###` tasks and checkbox progress. Unknown
or unparsable content stays visible as prose rather than being guessed.

The derived artifact index is stored in SQLite (`artifact_index`, schema v11).
It is replaced on every refresh and can be deleted/rebuilt without touching
files.

## Global switch

**Settings → General → Spec & intent tools** disables the Spec workspace,
Intent Inbox, agent contract, the Spec tab and the bundled Spec Kit /
agent-contract pack commands. The switch is stored in `StoredSettings` and
defaults to on for existing installations. Disabling it never deletes or edits
files on disk.

## Intent inbox

**Capture Intent…** creates
`intent/YYYY-MM-DD-<slug>.md` from the current composer text and opens it in the
Visual Markdown editor. The template contains problem, proposed outcome,
affected users/systems, constraints and open questions.

**Interview with agent** appends a prompt that asks the agent to interview the
originator one question at a time and only then write the intent draft.

The header has an **Intent inbox / Workflow** toggle. The inbox filters the list
to intents and the metrics bar shows total intents, accepted/total, open review,
average intent age and decision count. Acceptance rate is derived from the
effective hash-bound decision, not from a separate mutable flag.

## Decisions, notes and gates

The cockpit can record a local approve/reject decision with an optional note. It
is bound to the SHA-256 of the exact bytes and to the HEAD commit observed at
decision time. If the file changes, the cockpit shows `review needed · content
changed since the decision` and the old decision is no longer current.

`Copy decisions` copies the local JSON decision log to the clipboard. It is
local evidence only; signed attestations, PR/merge linkage and team review are
later phases.

The implementation gate is advisory at the composer boundary, not a sandbox:

- open clarification questions in `spec.md`;
- unchecked reviewer checklist items (agents must not self-approve them);
- no generated tasks.

`Next step` refuses to add an implement prompt while the gate is closed.

## Spec Kit phases

The cockpit derives ten phases:

1. Intent
2. Constitution
3. Specify
4. Clarify
5. Plan
6. Checklist
7. Tasks
8. Analyze
9. Implement
10. Converge

Optional phases (missing checklist, analyze report) are shown but do not block
`Next step`. Implement is blocked by open questions, reviewer checklists and
missing tasks. Converge is `needs evidence` until `tasks.md` contains a
convergence report; even then the UI asks the human to verify it says
`Converged` and that no appended tasks remain.

Task details expose **Create worktree for T###** (creates through the existing
confirmation flow and records `task_id` in the worktree registry), **Attach
HEAD**, **Attach GitHub** (selected CI run/PR), **Complete…** through the merge
gate, and links to **Checks**, **Task review** and **GitHub CI / PR**.

Completing a task from the cockpit requires a linked worktree record. The
completion action rewrites only that one `- [ ] T###` checkbox after a fresh
mtime check and records commit/worktree evidence. It never merges or pushes the
worktree branch.

The bundled `spec-kit` pack contributes the phase model and canonical
`/speckit-*` prompts. Dunlin does not require Python or the `specify` CLI. The
`Install Spec Kit skill` button adds the bundled `spec-driven-development` skill
to the current project for the active harness, using the existing hash-checked
skill installer.

Detected Spec Kit extensions are shown with their version and command count.
Dunlin reads them; it does not execute their Python hooks or rewrite their
manifests.

## Persistence model and active feature

The header cycles the project persistence model: `flow-back`, `flow-forward`,
`living spec`. It is stored per project and used for drift warnings only.

When more than one feature directory exists, the header can cycle between
`All features` and each feature. This is a view-local override; Dunlin does not
rewrite `.specify/feature.json`.

## Traceability

`Trace` switches the workspace to an interactive requirement → task →
worktree/evidence/session tree. Requirements and tasks expand/collapse; the
panel shows linked worktrees, commit/branch/GitHub evidence and matching
sessions. Links are indexed evidence, not proof of implementation.

## Coverage, drift and metrics

Every refresh computes:

- requirement → task coverage (`FR-*`, `SC-*`, `US*` mentioned by tasks),
- uncovered requirements,
- dirty governing files after implementation started,
- open reviewer checklists while implementation work exists,
- missing convergence evidence.

Warnings are shown in a drift bar. Requirement rows in the detail panel carry
`linked task` / `NO TASK`. The metrics bar also shows observed session cost for
the current project; it is not yet attributed to a specific intent or task.

## Agent contract

`Create agent contract` writes `.dunlin/agent-contract.md` with values,
autonomy, boundaries, escalation and disclosure sections. The cockpit shows the
contract hash and a parsed summary.

Dunlin enforces a **permission ceiling** at the client boundary and provides a
heuristic runtime audit: contract boundary keywords are matched against observed
tool titles/details in the current transcript. `AGENTS.md` is linted for
autonomy conflicts, and a manual `Run manual eval` prompt asks for PASS/FAIL/
UNKNOWN evidence per principle. None of this claims model-behaviour enforcement.

- `read-only` always wins;
- `ask` downgrades `Auto-approve` to `Ask`;
- `auto` or unknown does not loosen an existing mode.

A policy-conflict card appears when the project requests a broader mode than
the contract allows. This is not model-behaviour enforcement: the harness still
owns the model. `Ask agent to audit against contract` adds a read-only review
prompt from the bundled pack.

## Plugin packs (v1)

A pack is a directory with `plugin.json` and no executable code. Bundled packs
live in `assets/plugins/`. External packs can be dropped into
`~/Library/Application Support/Dunlin/plugins/` and enabled in
**Settings → Extensions**. Bundled ids cannot be shadowed by external packs.

Manifest fields include `id`, `name`, `version`, `api`, `description`,
`contributes` (workflows, commands, skills, views, artifact kinds, MCP server
declarations) and `permissions`. Validation rejects invalid ids, duplicate
commands/workflows, unsupported API versions and the combination of process
execution with full project write access. The UI shows a SHA-256 manifest hash;
this is local pinning evidence, not a publisher signature.

Enable/disable can be global or scoped to the current project. Missing
per-project ids inherit the global choice. Enabled pack commands appear in the
command palette and append their prompt to the composer explicitly. `Next step`
uses the project-scoped pack prompt and refuses when no enabled pack provides
it.

v1 boundaries:

- packs are metadata/prompt contributions only;
- MCP server declarations are displayed but not launched;
- no arbitrary GPUI, WASM or dylib UI;
- external packs require explicit opt-in and are not auto-updated;
- no remote catalog or signatures.

## Remaining work

- **Phase 1:** remote intent/spec/agent-contract authoring (remote indexing is
  read-only now); automatic PR/merge linkage for decisions; per-requirement
  cost attribution.
- **Phase 2:** Spec Kit extension hooks/commands execution; compare harnesses
  on the same spec; remote Spec Kit command runs.
- **Phase 3:** interactive requirement/task/commit/check graph; spec-aware
  review with acceptance criteria beside the diff; drift-driven CI checks.
- **Phase 4:** MCP connector launch and tool permissions; sidecar runtime with
  sandbox/capability grants; signatures/catalog; relay admin policy.
- **Phase 5:** automated behaviour evals and graders; relay sharing, comments
  and multi-user approvals.
