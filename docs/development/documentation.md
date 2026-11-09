# Maintaining documentation

Documentation is part of a behavior change. Update it in the same review as the
code; a recent date alone does not establish correctness.

## Where content belongs

- `getting-started/`: shortest supported path to a working first session.
- `user-guide/`: tasks, visible controls, expected results, scope and limitations.
- `administration/`: deployment, credentials/host operations, recovery and diagnostics.
- `reference/`: exact commands, precedence, formats, limits and source contracts.
- `architecture/`: responsibilities, state/data flow and trust boundaries.
- `development/`: contributor workflow, shared patterns and extension development.
- `quality/`: reproducible checks, measured baselines and explicit acceptance gaps.
- `project/`: current status and **one** roadmap; do not create a competing backlog.
- `archive/`: dated plans, handoffs and release evidence with a visible historical
  warning and links to maintained replacements.

Keep public docs in English, lower-kebab-case filenames and `README.md` section
indexes. Use repository-relative links, not machine-specific absolute paths.
Commands run from the repository root unless stated otherwise. Link source files
instead of duplicating large schemas/registries that will drift.

## Claims and evidence

Distinguish **implemented**, **automatically verified**, **manually accepted**,
**planned**, **removed** and **historical**. Describe the test scope: mock ACP,
loopback relay, real provider and deployed host are not interchangeable.
Date measurements/results and retain failures/unknowns. Never turn a test recipe
into a claim that it passed, or a completed storage slice into a fully delivered
product workflow.

For a changed interface, verify code/tests first: UI labels, CLI validation,
config precedence/defaults, filesystem/credential ownership and local/remote
boundaries. One canonical page owns a detailed contract; other pages summarize
and link to it. Keep roadmap items only for still-intended work. Do not revive a
removed feature because a historical proposal mentioned it.

## Navigation and migration

Every active page must be reachable from [docs/README.md](../README.md), normally
through its section index. Link new pages there. When moving a page, update
inbound links, heading fragments, screenshots, root policies, website source URLs
and test/tool contracts. A versioned GitHub URL in historical evidence is not a
mutable link to rewrite to a different commit. Avoid permanent redirect stubs
unless a public compatibility requirement justifies them; Git history retains
old paths.

## Check locally and in CI

```sh
python3 scripts/check_docs.py
python3 -m unittest discover -s scripts/tests -v
git diff --check
```

The standard-library checker validates local inline/reference Markdown links,
HTML `href`/`src`, Markdown heading fragments/explicit HTML anchors, required
section indexes and reachability of maintained pages. It scans `docs/`, root
Markdown, `todo/`, `.github/` Markdown and `web/README.md`; vendor/generated
content is excluded. External URLs are not fetched. Fenced/inline code examples
are not interpreted as links; bare prose/code paths still require review.

Archived files retain local path checks but may contain obsolete commands/claims;
their archive warning is essential. The checker cannot prove semantic currency,
external availability, visual screenshot accuracy or real-world acceptance.
CI runs it as a separate required-check step, with parser/navigation regression
tests in `scripts/tests`.
