# License texts

Dunlin itself is MIT-licensed; the root [`LICENSE`](../LICENSE) is the source
license. This folder keeps the full license texts that must ship with third-party
components whose notices are summarized in
[`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md).

- `Apache-2.0.txt` — full Apache License 2.0 text used by GPUI/GPUI Kit,
  agent-client-protocol and other Apache-2.0 dependencies. The macOS bundle
  copies it to `Dunlin.app/Contents/Resources/Apache-2.0.txt`.
- Font and icon licenses live next to their assets: `assets/fonts/OFL.txt`,
  `assets/fonts/Nerd-Fonts-LICENSE.txt`, `assets/Simple-Icons-LICENSE.md` and
  `assets/harnesses/LICENSE.md`.

Keep this folder in sync with the dependency graph; do not replace the full
Apache-2.0 text with a short notice.
