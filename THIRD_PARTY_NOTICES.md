# Third-party notices

Harness response marks (Claude, OpenAI/Codex, GitHub Copilot and Google Gemini)
are from Simple Icons (CC0); the OpenCode mark is derived from its MIT-licensed
favicon. Sources, modifications and the MIT notice are in
`assets/harnesses/LICENSE.md` (bundled as `Harness-Marks-LICENSE.md`). These marks
identify the selected harness and do not imply endorsement.

## Maintained dependency forks

The release-hardening branch maintains local migrations of GPUI, syntect,
two-face, rav1e and cosmic-text to replace unmaintained dependencies,
and block to fix Rust compatibility warnings while preserving its C ABI.
Original licenses/notices are retained under `vendor/`; exact archive provenance
and reviewed file changes are in `vendor/provenance.json`. See
[`vendor/README.md`](vendor/README.md) for the changes and asset-format boundary.
The embedded syntax/theme license Markdown is verified unchanged against its
upstream SHA-256. The historical asset converter is not shipped or used in normal builds.

Dunlin is distributed under the MIT License. It is built on open-source
software and bundles open-source fonts. This file lists the licenses of the
software that is compiled into Dunlin and the Dunlin relay.

Full license texts for Apache-2.0 and the SIL Open Font License are included
in [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) and
[`assets/fonts/OFL.txt`](assets/fonts/OFL.txt).

## Bundled fonts

Group logos `assets/aws-lambda.svg` and `assets/kubernetes.svg` come from
Simple Icons 11.15.0 (`https://github.com/simple-icons/simple-icons/tree/11.15.0/icons`),
under CC0 1.0 Universal; full text: `assets/Simple-Icons-LICENSE.md`.
AWS and Kubernetes trademarks remain the property of their respective owners;
their use identifies user-selected groups and does not imply endorsement.

| Component | Copyright | License |
| --- | --- | --- |
| Geist Sans | Copyright 2024 The Geist Project Authors | SIL Open Font License 1.1 |
| Geist Mono | Copyright 2024 The Geist Project Authors | SIL Open Font License 1.1 |
| Symbols Nerd Font Mono 3.4.0 | Copyright (c) 2014 Ryan L McIntyre | MIT; `assets/fonts/Nerd-Fonts-LICENSE.txt` |

Geist is created by Vercel in collaboration with basement.studio. See
<https://vercel.com/font>.

Terminal symbol fallback: <https://github.com/ryanoasis/nerd-fonts>, tag `v3.4.0`,
`patched-fonts/NerdFontsSymbolsOnly/SymbolsNerdFontMono-Regular.ttf`.
SHA-256: `f0f624d9b474bea1662cf7e862d44aebe1ae1f6c7f9cb7a0ca5d0e5ac9561c60`.

## Rust dependencies

| Component | License |
| --- | --- |
| GPUI, GPUI Kit, GPUI Component | Apache-2.0 |
| syntect, two-face, bincode, bit-set, bit-vec, fancy-regex | MIT or Apache-2.0 |
| onig | MIT |
| onig_sys / Oniguruma | MIT / BSD-2-Clause |
| image | MIT or Apache-2.0 |
| raw-window-handle | MIT or Apache-2.0 or Zlib |
| agent-client-protocol | Apache-2.0 |
| similar | Apache-2.0 |
| rusqlite, fontdb, hyper, tower | MIT |
| axum, tokio, tokio-tungstenite | MIT |
| notify | CC0-1.0 |
| tiny-skia | BSD-3-Clause |
| webpki-roots | CDLA-Permissive-2.0 |
| rustls, rustls-native-certs, rustls-pemfile | Apache-2.0 OR ISC OR MIT |
| objc2, objc2-foundation | MIT |
| objc2-app-kit, objc2-quartz-core, objc2-core-graphics | Zlib OR Apache-2.0 OR MIT |
| resvg, usvg | Apache-2.0 OR MIT |
| serde, serde_json, thiserror, base64, http, uuid | MIT OR Apache-2.0 |
| notify-rust, rand, sha2, toml, futures-util, async-channel, async-io, ttf-parser | MIT OR Apache-2.0 |
| ureq | MIT OR Apache-2.0 |
| hex | MIT OR Apache-2.0 |
| security-framework | MIT OR Apache-2.0 |
| zeroize, percent-encoding | MIT OR Apache-2.0 |
| url | MIT OR Apache-2.0 |

The exact dependency graph and resolved versions are recorded in
`Cargo.lock`. License texts for each crate are available from the crate
sources in the Cargo registry and from <https://crates.io>.

Syntax definitions supplied by two-face/bat carry additional upstream notices.
The app bundle includes their complete generated acknowledgements in
`Contents/Resources/Syntax-Notices.md`; `Dunlin --syntax-notices` prints the same
version-matched text without launching the UI or connecting a harness.

## Bundled skills

| Component | Copyright | License |
| --- | --- | --- |
| Cloudflare `security-audit-skill` (pinned commit `c1c8a8c`) | Copyright 2025-2026 Cloudflare, Inc. | MIT |

The complete skill bundle is embedded under `assets/skills/security-audit`,
including its MIT license, companion playbooks, JSON schema and validators.
Dunlin installs these files as text; it does not execute the bundled scripts.

## Runtime components

Dunlin launches external ACP harnesses such as OpenCode, Claude Code and
Codex. Those programs are not bundled with Dunlin, are installed by the user,
and are governed by their own licenses and terms. Codex Security is an external
plugin and is not bundled or reimplemented by Dunlin.

The macOS application uses Apple system frameworks under the SDK license that
ships with Xcode.

## Website

The marketing site in `web/` is built with Astro, React, Tailwind CSS, Motion
and Phosphor Icons. It bundles Manrope, Instrument Serif and Geist Mono. The
license texts are in `web/public/licenses/`.
