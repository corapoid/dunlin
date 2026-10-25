# Third-party notices

Dunlin is distributed under the MIT License. It is built on open-source
software and bundles open-source fonts. This file lists the licenses of the
software that is compiled into Dunlin and the Dunlin relay.

Full license texts for Apache-2.0 and the SIL Open Font License are included
in [`licenses/Apache-2.0.txt`](licenses/Apache-2.0.txt) and
[`assets/fonts/OFL.txt`](assets/fonts/OFL.txt).

## Bundled fonts

| Component | Copyright | License |
| --- | --- | --- |
| Geist Sans | Copyright 2024 The Geist Project Authors | SIL Open Font License 1.1 |
| Geist Mono | Copyright 2024 The Geist Project Authors | SIL Open Font License 1.1 |

Geist is created by Vercel in collaboration with basement.studio. See
<https://vercel.com/font>.

## Rust dependencies

| Component | License |
| --- | --- |
| GPUI, GPUI Kit, GPUI Component | Apache-2.0 |
| agent-client-protocol | Apache-2.0 |
| similar | Apache-2.0 |
| rusqlite, fontdb, hyper, tower | MIT |
| axum, tokio, tokio-tungstenite | MIT |
| notify | CC0-1.0 |
| tiny-skia | BSD-3-Clause |
| webpki-roots | CDLA-Permissive-2.0 |
| rustls, rustls-native-certs, rustls-pemfile | Apache-2.0 OR ISC OR MIT |
| objc2, objc2-foundation | MIT |
| objc2-app-kit | Zlib OR Apache-2.0 OR MIT |
| resvg, usvg | Apache-2.0 OR MIT |
| serde, serde_json, thiserror, base64, http, uuid | MIT OR Apache-2.0 |
| notify-rust, rand, sha2, toml, futures-util, async-channel, async-io, ttf-parser | MIT OR Apache-2.0 |
| ureq | MIT OR Apache-2.0 |

The exact dependency graph and resolved versions are recorded in
`Cargo.lock`. License texts for each crate are available from the crate
sources in the Cargo registry and from <https://crates.io>.

## Runtime components

Dunlin launches external ACP harnesses such as OpenCode, Claude Code and
Codex. Those programs are not bundled with Dunlin, are installed by the user,
and are governed by their own licenses and terms.

The macOS application uses Apple system frameworks under the SDK license that
ships with Xcode.

## Website

The marketing site in `web/` is built with Astro, React, Tailwind CSS, Motion
and Phosphor Icons. It bundles Manrope, Instrument Serif and Geist Mono. The
license texts are in `web/public/licenses/`.
