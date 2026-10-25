export const site = {
  name: "Dunlin",
  tagline: "A native, agent-first IDE for macOS.",
  description:
    "Dunlin connects OpenCode, Claude Code, Codex and any ACP harness to one fast Rust and GPUI workspace. Chat, permissions, diffs, Git and traces, without Electron.",
  url: "https://dunlin.sh",
  github: "https://github.com/corapoid/dunlin",
  releases: "https://github.com/corapoid/dunlin/releases",
  latestRelease: "https://github.com/corapoid/dunlin/releases/latest",
  roadmap: "https://github.com/corapoid/dunlin/blob/main/docs/ROADMAP.md",
  security: "https://github.com/corapoid/dunlin/security/advisories/new",
  license: "https://github.com/corapoid/dunlin/blob/main/LICENSE",
  version: "0.1.0-beta.1",
  downloadDmg:
    "https://github.com/corapoid/dunlin/releases/download/v0.1.0-beta.1/Dunlin-0.1.0-beta.1-macos-arm64.dmg",
  downloadZip:
    "https://github.com/corapoid/dunlin/releases/download/v0.1.0-beta.1/Dunlin-0.1.0-beta.1-macos-arm64.zip",
  downloadRelay:
    "https://github.com/corapoid/dunlin/releases/download/v0.1.0-beta.1/dunlin-relay-0.1.0-beta.1-macos-arm64.tar.gz",
  checksums:
    "https://github.com/corapoid/dunlin/releases/download/v0.1.0-beta.1/SHA256SUMS",
} as const;

export const navLinks = [
  { label: "Features", href: "/#features" },
  { label: "Native", href: "/#native" },
  { label: "Remote", href: "/#remote" },
  { label: "FAQ", href: "/#faq" },
] as const;
