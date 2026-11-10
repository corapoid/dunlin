# Reviewed dependency migrations (no advisory exceptions)

These are **Dunlin-maintained local forks**, not unmodified upstream packages.
Their upstream versions remain in the directory names for provenance; patches
are explicit in the workspace `[patch.crates-io]` table. They are not published
to crates.io. Upstream licenses, notices and sources are retained.

`provenance.json` records the SHA-256 of each original crates.io archive from
the application lockfile, every modified file's before/after hash, and the complete
patched tree digest. `python3 scripts/verify_vendor.py` fails on unreviewed tree
changes, unregistered patches, symlinks or reintroduction of any of the five old
packages anywhere in the lockfile. It does not exempt forks from cargo-deny.

| Local fork | Change |
| --- | --- |
| block 0.1.6 | Use inhabited `c_void` for the opaque runtime class instead of an empty enum and spell out the existing C ABI; fixes Rust's `uninhabited_static` future incompatibility and `missing_abi` warnings without changing the ABI |
| gpui-kit / gpui-kit-assets 0.9.0 | Align exact GPUI pre dependency pins with 0.3.12 throughout the graph |
| gpui-base 0.9.0 | `instant` → existing maintained `web-time`; same clock/duration API; opt-out `scrollable` flag so embedded visual editors forward wheel input to the document; GPUI pre pins → 0.3.12 |
| gpui-component 0.9.0 | `instant` → `web-time`, `paste` → `pastey`; Windows SVG dependency → resvg 0.49; transparent folder strips, spring-animated selection fill (text stays still), reduced-motion opt-out and compact trailing close-control padding; Syntect Oniguruma backend restores JSX/PowerShell; YAML/Jinja capture overlays and named-capture API; GPUI pre pins → 0.3.12 and two-face → 0.5.2 |
| gpui-pre 0.3.12 | resvg/usvg 0.49 (harfrust/fontdb 0.24); glyph coverage through skrifa, not ttf-parser; re-export the test attribute as `gpui_test` so it cannot shadow the built-in `#[test]`; typed active-drag query keeps unrelated project/tab drop affordances isolated; expose existing painted-quad inspection in debug builds for native corner regression checks |
| rav1e 0.8.1 | Macro dependency `paste` → maintained `pastey` |
| rav1e 0.8.1 C API | Fail-closed null-context/frame/buffer guards, checked nullable test views and bounded rate-control decoding; five isolated C API tests pass (this feature is not enabled in Dunlin) |
| cosmic-text 0.19.0 | fontdb 0.24, preventing the non-macOS graph from retaining old ttf-parser |
| syntect 5.3.0 | Replace bincode dumps with a versioned MessagePack envelope (`DLSYNP01`), bounded reader and explicit compression finalization; compile metadata file loaders only with `yaml-load` or tests |
| two-face 0.5.2+bat-0.26.1 | Migrate updated bundled syntax, nested lazy-context/theme and license dumps to the new codec |

Additional native-control patches preserve accessible button labels while allowing
an opt-in animated label renderer and raised/semantic-outline action treatments
(`gpui-component`). HTML preformatted content becomes a real code block and
spanless paragraphs retain distinct element identities (`gpui-base`). Native
visual fixtures can present a manually drawn frame in debug builds (`gpui-pre`).
Their original-file hashes were checked against the checksum-verified registry
archives; the complete patched trees are recorded in `provenance.json`.

The selectable plain-text extension is recorded as well: four `gpui-base` files
add a `Plain` format that appends source directly to one paragraph, preserves
whitespace, and selects the existing plain-text copy behavior; the
`gpui-component` compatibility constructor forwards to it. It does not parse
HTML/Markdown, fetch resources, change dependencies or introduce unsafe code.
The November 2026 reconciliation compared **all files** in both forks against
their SHA-256-verified crates.io archives. Only those five files were absent from
the earlier per-file change records; all other differences matched those records.
This is a focused source/provenance review, not a new comprehensive security audit.

The application's own resvg is also 0.49. All five packages (`bincode`, `instant`,
`paste`, `rustybuzz`, `ttf-parser`) are absent from the entire application lockfile
and explicitly banned in `deny.toml`. `advisories.ignore` remains empty.

The GPUI 0.9 dependency graph no longer contains metal 0.33.0 or pulp 0.22.3.
Their unused patches and vendored source trees were removed; the verifier also
rejects unused patches recorded in `Cargo.lock`.

## Asset conversion and compatibility

Legacy binary dump files are **not accepted** by the maintained runtime codec.
Dunlin did not persist user syntax-set dumps in its settings/database; project
source files, sessions, editor buffers and undo are not converted or discarded.

The one-time `scripts/migrate_syntax_assets.rs` converter reads only the original
trusted assets pinned by `asset-sources.json`, using upstream syntect 5.3.0 and
two-face 0.5.2+bat-0.26.1 in a **separate temporary Cargo project**. It converts both outer
containers and nested lazy payloads. Its legacy reader is not a workspace member,
normal build dependency, CI step or part of the shipped app/relay.

To reproduce, create an isolated manifest with `[workspace]`, a binary path to
that converter, original `syntect = { version = "=5.3.0", default-features = false,
features = ["default-fancy", "metadata"] }`, original `two-face = { version =
"=0.5.2", default-features = false, features = ["syntect-fancy"] }`, serde/derive,
serde_json, flate2, rmp-serde 1.3.1, hex 0.4.3 and sha2 0.11.1. Use a separate output root
with `vendor/syntect-5.3.0/assets` and `vendor/two-face-0.5.2+bat-0.26.1/generated` directories,
then pass the original registry source directory and that output root as arguments.
Do not run the converter against arbitrary/untrusted binary dumps.

The four syntect assets remain byte-for-byte unchanged; all six two-face assets
were regenerated from the new archive. The syntax corpus now contains 75 defaults,
213 fancy extras and 220 onig extras per newline variant.
Runtime tests traverse every lazy context in the active onig/default sets, load
all lazy themes, round-trip dumps, reject legacy/malformed data, highlight Unicode
without changing source text, and verify the **exact original license Markdown
SHA-256**. A separate cosmic-text library probe also compiles with fontdb 0.24;
that is not a claim of a tested Linux GUI port.

Mirrored upstream CI templates under `vendor/**/.github` cannot execute from that
location and are excluded from workflow analysis. Patched Rust sources remain
in CodeQL scope. Upstream whitespace is preserved; first-party formatting rules
and the complete-tree integrity check remain active.
