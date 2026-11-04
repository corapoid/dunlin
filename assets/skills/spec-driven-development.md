---
name: dunlin-spec-driven-development
description: Use an existing GitHub Spec Kit workspace (.specify/ and specs/) without inventing process state or self-approving reviewer artifacts.
---

# Spec-driven development

Use this skill when the repository contains `.specify/` or `specs/`.

## Ground rules

- Files are the source of truth. Read `.specify/feature.json`, the active
  feature directory, `spec.md`, `plan.md`, `tasks.md`, `research.md`,
  `data-model.md`, contracts and checklists before changing code.
- Do not rewrite another tool's artifacts to make them match your work.
  `converge` is append-only; `analyze` is read-only.
- Reviewer-owned checklist items (`[x]`) are never yours to approve. Report
  open items and stop if they gate implementation.
- Do not claim a phase is complete because a file exists. Report observed
  evidence, committed code, checks and unresolved questions separately.
- If the required `/speckit-*` skill is unavailable, say so. Do not imitate
  Spec Kit commands or fabricate their output.

## Working the phases

1. **Intent** — read the approved intent. If it is missing or ambiguous, ask
   before writing a specification.
2. **Constitution** — follow the project principles; report conflicts.
3. **Specify / clarify** — keep requirements and success criteria explicit.
   Ask targeted questions instead of guessing.
4. **Plan** — respect the planned architecture and constraints.
5. **Checklist** — read reviewer checklists as gates; never check them.
6. **Tasks** — execute in dependency order and honor parallel markers.
7. **Implement** — make the smallest coherent change. Do not start unrelated
   refactors.
8. **Analyze** — report cross-artifact conflicts; do not edit artifacts.
9. **Converge** — append only missing work to `tasks.md`; otherwise leave it
   byte-for-byte unchanged and say `Converged` only when true.

## Evidence

For every completed task, state:

- files changed,
- commands/checks actually run and their result,
- anything still unknown or unverified.

Never turn an unknown into a pass. Never push, merge, publish or discard work
without explicit approval.
