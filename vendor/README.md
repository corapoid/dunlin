# Reviewed dependency migrations (no advisory exceptions)

These are **Dunlin-maintained local forks**, not unmodified upstream packages.
Their upstream versions remain in the directory names for provenance; patches
are explicit in the workspace `[patch.crates-io]` table. They are not published
to crates.io. Upstream licenses, notices and sources are retained.

`provenance.json` records the SHA-256 of each original crates.io archive from
the beta.3 lockfile, every modified file's before/after hash, and the complete
patched tree digest. `python3 scripts/verify_vendor.py` fails on unreviewed tree
changes, unregistered patches, symlinks or reintroduction of any of the five old
packages anywhere in the lockfile. It does not exempt forks from cargo-deny.

| Local fork | Change |
| --- | --- |
| gpui-base 0.8.0 | `instant` → existing maintained `web-time`; same clock/duration API; opt-out `scrollable` flag so embedded visual editors forward wheel input to the document |
| gpui-component 0.8.0 | `instant` → `web-time`, `paste` → `pastey`; Windows SVG dependency → resvg 0.49 |
| gpui-pre 0.3.9 | resvg/usvg 0.49 (harfrust/fontdb 0.24); glyph coverage through skrifa, not ttf-parser; re-export the test attribute as `gpui_test` so it cannot shadow the built-in `#[test]` |
| metal 0.33.0 / pulp 0.22.3 / rav1e 0.8.1 | Macro dependency `paste` → maintained `pastey` |
| rav1e 0.8.1 C API | Fail-closed null-context/frame/buffer guards, checked nullable test views and bounded rate-control decoding; five isolated C API tests pass (this feature is not enabled in Dunlin) |
| cosmic-text 0.19.0 | fontdb 0.24, preventing the non-macOS graph from retaining old ttf-parser |
| syntect 5.3.0 | Replace bincode dumps with a versioned MessagePack envelope (`DLSYNP01`), bounded reader and explicit compression finalization |
| two-face 0.4.5 | Migrate bundled syntax, nested lazy-context/theme and license dumps to the new codec |

The application's own resvg is also 0.49. All five packages (`bincode`, `instant`,
`paste`, `rustybuzz`, `ttf-parser`) are absent from the entire application lockfile
and explicitly banned in `deny.toml`. `advisories.ignore` remains empty.

## Asset conversion and compatibility

Legacy binary dump files are **not accepted** by the maintained runtime codec.
Dunlin did not persist user syntax-set dumps in its settings/database; project
source files, sessions, editor buffers and undo are not converted or discarded.

The one-time `scripts/migrate_syntax_assets.rs` converter reads only the original
trusted assets pinned by `asset-sources.json`, using upstream syntect 5.3.0 and
two-face 0.4.5 in a **separate temporary Cargo project**. It converts both outer
containers and nested lazy payloads. Its legacy reader is not a workspace member,
normal build dependency, CI step or part of the shipped app/relay.

To reproduce, create an isolated manifest with `[workspace]`, a binary path to
that converter, original `syntect = { version = "=5.3.0", default-features = false,
features = ["default-fancy", "metadata"] }`, original `two-face = { version =
"=0.4.5", default-features = false, features = ["syntect-fancy"] }`, serde/derive,
serde_json, flate2, rmp-serde 1.3.1 and sha2 0.10.9. Use a separate output root
with `vendor/syntect-5.3.0/assets` and `vendor/two-face-0.4.5/generated` directories,
then pass the original registry source directory and that output root as arguments.
Do not run the converter against arbitrary/untrusted binary dumps.

All ten resulting assets were reproduced byte-for-byte. The syntax corpus counts
are preserved (75 defaults, 199 fancy extras, 205 onig extras per newline variant).
Runtime tests traverse every lazy context in the active fancy/default sets, load
all lazy themes, round-trip dumps, reject legacy/malformed data, highlight Unicode
without changing source text, and verify the **exact original license Markdown
SHA-256**. A separate cosmic-text library probe also compiles with fontdb 0.24;
that is not a claim of a tested Linux GUI port.

Mirrored upstream CI templates under `vendor/**/.github` cannot execute from that
location and are excluded from workflow analysis. Patched Rust sources remain
in CodeQL scope. Upstream whitespace is preserved; first-party formatting rules
and the complete-tree integrity check remain active.
