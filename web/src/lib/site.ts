// Only advance this after verifying the published assets, not merely a tag push.
const version = "0.1.0-beta.6";
const github = "https://github.com/corapoid/dunlin";
const releaseAssets = `${github}/releases/download/v${version}`;

export const site = {
  name: "Dunlin",
  tagline: "A native, agent-first IDE for macOS.",
  description:
    "Dunlin connects OpenCode, Claude Code, Codex and any ACP harness to one fast Rust and GPUI workspace. Chat, permissions, diffs, Git and traces, without Electron.",
  url: "https://dunlin.sh",
  github,
  releases: "https://github.com/corapoid/dunlin/releases",
  latestRelease: `${github}/releases/tag/v${version}`,
  roadmap: "https://github.com/corapoid/dunlin/blob/main/docs/ROADMAP.md",
  security: "https://github.com/corapoid/dunlin/security/advisories/new",
  license: "https://github.com/corapoid/dunlin/blob/main/LICENSE",
  version,
  downloadDmg: `${releaseAssets}/Dunlin-${version}-macos-arm64.dmg`,
  downloadZip: `${releaseAssets}/Dunlin-${version}-macos-arm64.zip`,
  downloadRelay: `${releaseAssets}/dunlin-relay-${version}-macos-arm64.tar.gz`,
  checksums: `${releaseAssets}/SHA256SUMS`,
} as const;

export const navLinks = [
  { label: "Features", href: "/#features" },
  { label: "Native", href: "/#native" },
  { label: "Remote", href: "/#remote" },
  { label: "FAQ", href: "/#faq" },
] as const;
