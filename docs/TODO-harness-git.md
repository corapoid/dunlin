# Session import, Git history and macOS integration

> Historical record of a completed scope, not a current backlog. The numbers and
> the note about no commit/push below refer to that validation. Sources were published as
> beta.3; currently 592 tests OK / 0 skipped. Manual acceptance is still
> separate from native smoke. Current state: [ROADMAP.md](ROADMAP.md).

- [x] Session file import in harness settings: adapters, preview, project matching, deduplication and atomic save.
- [x] Graphical repository history and branch creation from the selected commit without checkout.
- [x] Custom icon in Project settings, with the ability to return to automatic detection.
- [x] Greeting based on local system time.
- [x] Native macOS menu: Settings, files, navigation and Git.
- [x] Tests, Clippy, formatting, a new macOS package and signature verification.
- [x] Review of ROADMAP/PLAN and the current list of unimplemented items.

Import does not start agents or modify sources. Ambiguous or
unmatched paths require explicit project selection. Existing conversations are not
overwritten. Git history is local, bounded and does not fetch.

Validation: 301 tests passed, 4 optional integration tests
skipped; Clippy `-D warnings`, fmt and diff check correct. `dist/Dunlin.app` was built;
`codesign --verify --deep --strict` correct (ad-hoc signature).
No visual GUI test because window control was unavailable. No paid prompts
were used and the working repository was not modified with test Git commands;
mutating operations were tested only on isolated fixtures.
Changes remain without commit/push. Remaining backlog: `REMAINING-WORK.md`.
