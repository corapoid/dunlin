# Project catalog and harness modes

- [x] Design → Sidebar preferences: language, GitHub topics, branches, wrapped project names and hover preview; backward-compatible persistence.
- [x] Normal-weight project names, language filter populated from actual primary languages, clickable topic filters and explicit clear action, in list and tree.
- [x] Background, bounded, cached GitHub metadata and README preview; no repository writes, external images or credential extraction.
- [x] Group color, AWS Lambda/Kubernetes logo and embedded custom icon, with cancel/save and persisted settings.
- [x] Move active-project switcher into the title bar, including when the sidebar is hidden.
- [x] Unify actual agent/connection status in the title bar; remove its duplicate from the footer while preserving terminal/run/branch controls.
- [x] Harness-only dynamic mode selector, acknowledged ACP mode changes, no fabricated modes or implicit permission escalation.
- [x] Tests, isolated native acceptance, separate bundle.

## Contract discovery (confirmed)

| Claim | Evidence | Revision | Confidence | Missing link / next lookup |
| --- | --- | --- | --- | --- |
| Primary language is the largest detected source-byte share, not a guessed extension | `languages::detect`, `local_projects::language_badge`, `RepoSummary.languages` | working tree | high | none |
| GitHub identity is an exact validated HTTPS/SSH remote, never the display/project name; read-only metadata also works without HEAD | `crates/workspace/src/github.rs::{metadata_identity,metadata,repository_name}` | working tree | high | none |
| GitHub metadata includes `full_name`, `language`, `topics`; a single repo response is not paginated | GitHub REST repositories documentation, GET `/repos/{owner}/{repo}` | checked 2026-11-05 | high | none |
| Existing `gh` client pins GitHub host/repo, bounds output/time, disables prompts and sanitizes errors | `github.rs::{command, read_bytes, api}` | working tree | high | none |
| Relay GitHub access uses the relay account, so is admin-only in addition to project grants | `crates/relay/src/api.rs::{github_load,github_metadata}` | working tree | high | none; negative authorization tests passed |
| ACP modes are runtime-advertised config options or legacy availableModes; mode selection uses the corresponding setter and waits for acknowledgement | `crates/agent/src/{models.rs,lib.rs}`, `harness.rs::{select_mode,apply_modes}`, `agent/tests/sessions.rs` | agent-client-protocol 3.5.0; working tree | high | none; both setters exercised against mock ACP |
| “Auto · Ask” combined two unrelated settings; Ask is Dunlin's fallback tool-permission policy, not the selected harness mode | `views/chat_controls.rs::{mode,mode_label}`; domain PermissionMode | working tree | high | none; combined label/menu removed and regression-tested |
| Group tone is persistent; group settings JSON stores an optional non-inherited icon without a schema migration | domain ProjectGroup/GroupSettings; storage group upsert | schema 14 | high | none; color/icon roundtrip and native save/cancel tests passed |

```text
project --validated Git remote--> GitHub owner/repo --gh GET repos/owner/repo--> topics/name/language
remote project --authenticated project grant + admin check--> relay-owned gh account --> same metadata
session --ACP session response/notifications--> available modes --set_config_option or set_mode--> acknowledged current mode
```

All edges above are proven by repository contracts. Display labels do not authorize
repository access. README and external metadata are inert data, never instructions.
No new credential/token store or automatic approval bypass is introduced.

## Behavior and boundaries

- Preferences are in **Settings → Design → Sidebar**. Existing installations
  retain branch visibility; the new language/topics/preview flags default on and
  wrapping defaults off. Filters are temporary; hiding a filter feature clears its
  selection. Tree filters preserve group-agent scope/counts (a filter must never
  silently change which repositories a group agent can access).
- GitHub metadata is read with the installed, already-authenticated `gh` CLI.
  Prefer `origin`; without it require exactly one verified GitHub remote. Identity
  is checked before and after the request. Metadata responses are bounded to
  64 KiB and 20 seconds, validate repository identity and topics, and never echo
  credential-bearing stderr. No HEAD commit is needed for this read-only request.
- One background metadata worker; ten-minute in-memory cache, scoped to project
  root/trust/relay identity. Untrusted projects do not start these reads. A missing
  `gh`, failed auth, non-GitHub remote or old relay leaves ordinary project use
  working, with an unavailable-metadata explanation in the preview.
- Relay `/v1/github/metadata` keeps the existing GitHub admin + project-grant
  boundary. It never falls back to a local checkout/account. Remote primary
  language can use this metadata; access restrictions remain authoritative.
- README previews use confined file reads, at most 32,768 Unicode characters,
  and inert plain text. No external images, HTML, repository commands or agent
  instructions execute. Storage uses the whole-folder allocated disk usage,
  including generated folders and Git metadata, not GitHub's unrelated repository
  size or the Explorer's source-only inventory. The platform `du -skP` scan is
  bounded to eight seconds, does not follow symlinks, and reports Unknown on a
  failed/incomplete scan instead of inventing a partial total. Remote storage
  reads use `/v1/fs/storage` with project grants, independently of GitHub admin access.
- Group appearance is available by clicking its dot/icon or **Group settings…**
  in the context menu. Color is the existing group tone; the optional icon lives
  in group settings JSON, not inherited runtime defaults. Built-ins are Simple
  Icons 11.15.0 logos (CC0); user icons reuse the existing bounded image decoder,
  are rasterized to at most 128 px, embedded, and survive source-file deletion.
- The title-bar status follows actual runtime/connection state, not a persisted
  “Running” label. The footer keeps branch/terminal/run controls but no longer
  repeats the harness status. The project picker is a non-dimming global popover,
  so it works with a hidden sidebar and closes when a modal opens.
- The composer mode menu contains only harness-advertised choices. `Auto` means
  the acknowledged ACP mode, not a made-up combination with “Ask”. Dunlin's tool
  approval policy remains separately configurable in **Permissions**; selecting a
  harness mode never silently disables read-only, vault, budget or group guards.

## Verification — 2026-11-05

- Desktop: **373 tests passed**; storage: **33 passed**.
- Domain: **17 passed**; ACP library: **18 passed**, plus **14 mock-session
  integration tests**, including both legacy and config-option mode setters.
- GitHub workspace tests: **21 passed**, including metadata identity, topic
  validation/deduplication, HEAD-less repos, bounded requests and sanitized errors.
- Full relay suite: **47 passed** with `SHELL=/bin/sh`; the added storage-grant
  endpoint test passed separately after extending the suite to 48 tests.
- Whole-folder storage test passed, including generated files, `.git`, missing
  roots, and an external symlink that must not contribute its target's size.
- Extended `--workspace-ui-smoke`: passed. It asserts list/tree wrapped-row
  geometry, opens the real hover preview, verifies combined/hidden topic and
  primary-language filters, checks persisted group color/icon save vs cancel,
  and draws the title-bar picker while the sidebar is hidden. Previous chat,
  graph, Git-confirmation and session-navigation regressions still run.
- Native glass/input/popover smoke passed after moving the picker globally.
- `cargo build -p dunlin` and `git diff --check` passed.
- Final ad-hoc-signed bundle: `dist/project-catalog/Dunlin Project Catalog.app`.
  Its own executable passed the extended native smoke after the whole-folder
  storage change; `codesign --verify --deep --strict` passed. Earlier bundles and
  existing working-tree changes were preserved.

No real GitHub account or production relay was queried in acceptance tests; GitHub
responses use the existing mock `gh` fixture and ACP uses the mock harness. No
real-provider spending, automatic permission bypass, repository writes, manual
pixel-by-pixel review or notarized release was performed.

## Handoff

Follow-up (2026-11-05): removed the repeated `+ New session` action under every
project in both list and tree. Empty session blocks no longer render a border or
spacing; existing session rows and title-bar session creation remain available.

> Later UI follow-up: Projects now uses only Tree. The global **Group appearance…**
> dropdown was removed; group-header settings/icon and context-menu actions remain.
> Topic chips now have explicit themed fill/borders. The older List/Tree and
> dropdown acceptance notes below describe the previous delivery.

Appearance follow-up (2026-11-05): GPUI's `Style::overflow_mask` clips to a
rectangle, not the parent's corner radii. The composer's separately painted
header now has 19 px top corners inside the 20 px/1 px bordered card.
Group headers expose a visible 24 px settings action (rename stays in the context
menu), and **Group appearance…** above the projects opens every group's settings
in both List and Tree, without adding project breadcrumbs. **Color dot** explicitly
removes the image; a loaded logo/custom image replaces the dot. Color is retained
when switching back. Save/Cancel stay outside the scrollable settings content.
The panel now receives initial focus itself instead of the model input farther
down; focusing that input could scroll the appearance controls out of view on open.
Verification: 373 desktop and 33 storage tests passed again. Extended native smoke
now dispatches mouse events to the tree's settings action, Kubernetes/color
controls and Save; reopens via the List appearance menu; selects Color dot and
saves again. It checks persisted image/tone and mutually exclusive dot/image
rendering. The first run exposed the focus/scroll issue; after fixing it the native
smoke passed. Glass/input smoke passed too. Custom file decoding remains covered
by the existing bounded-image/source-removal unit test; the OS file chooser and
pixel-level corner appearance have not been manually reviewed.
The click smoke waits for the deferred detached-modal frame before looking for
its controls (an initial packaged run reached them one frame too early).
Separate bundle: `dist/group-appearance/Dunlin Group Appearance.app`; signature
verification passed. Earlier delivered application bundles were not overwritten.
The final packaged executable passed the extended native click smoke as well.

Title-bar picker follow-up (2026-11-05): project rows now show one name only,
without the repeated path tail or group breadcrumb. The picker always uses the
existing cycle-safe group hierarchy, including empty parent groups, nested
indentation, group appearance, and expand/collapse controls. It shares persisted
collapse state with the sidebar; selecting List in the sidebar does not flatten
the picker. Pinned/name project sorting remains unchanged. Projects whose saved
group no longer exists stay reachable in Ungrouped. Group toggles keep the picker
open; selecting the current project also dismisses the picker, without bypassing
dirty-editor or busy-operation guards on navigation.
Verification: 373 desktop tests passed. Extended native smoke exercises the
title-bar picker with a root containing only a subgroup, verifies nested row
indentation, clicks collapse/expand, checks that both descendants and their
projects disappear/reappear while an orphaned-group project remains reachable,
then selects the current project and verifies dismissal. This also runs with the
sidebar hidden. Build, `git diff --check`, and signature verification passed.
Separate bundle: `dist/project-picker-tree/Dunlin Project Picker Tree.app`.
The packaged executable passed the extended native smoke; glass/input/outside
dismissal smoke passed too. No manual pixel-level review was performed.

Confirmed: all retrieval and identity edges above; preferences/persistence,
scope boundaries and native rendering are covered by executable checks.
Rejected: guessing GitHub identity from a project name, treating `Auto` as
`AutoApprove`, using GitHub size/source-only bytes as whole-folder storage,
or silently broadening group-agent scope when a filter is active.
Unresolved live-only acceptance: the user's installed `gh` authentication, each
real harness's advertised modes, and production relay deployment. The next owner
is the user/operator: launch the separate bundle, verify appearance on their
display, and update the relay to enable the new metadata/storage endpoints.
