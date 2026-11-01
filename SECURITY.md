# Security policy

## Supported versions

Dunlin is pre-1.0. Security fixes land on the latest release line.

| Version | Supported |
| --- | --- |
| 0.1.x beta | Yes |
| Older | No |

## Reporting a vulnerability

Please do **not** open a public issue for a security problem. Use GitHub's
private vulnerability reporting:

<https://github.com/corapoid/dunlin/security/advisories/new>

If that is unavailable, open a minimal public issue asking for a private
channel without describing the vulnerability.

Include:

- the affected version or commit;
- macOS version and hardware;
- whether the issue is local, relay or iOS;
- reproduction steps and a proof of concept if you have one;
- the impact you believe the issue has.

We aim to acknowledge reports within a few days and to agree on a disclosure
timeline with you. Please give us a reasonable window to ship a fix before
publishing details.

## Threat model

Dunlin runs an ACP harness as a child process and gives it tools that can read
and write the project folder. The relay extends that boundary over the
network. The following are in scope:

- escape from the configured project root by the harness or relay sandbox;
- relay authentication, pairing, project-grant or session-lease bypass;
- token leakage from Keychain or relay logs;
- cross-user access to projects, sessions or transcript snapshots;
- command injection through harness commands, file paths or repository
  content;
- path traversal in file, search, diff or attachment endpoints;
- malicious repository content that compromises the app.

The following are known limitations, not vulnerabilities by themselves:

- the relay runs all users under the credentials installed on the host;
- the SQLite database is not encrypted at rest;
- the macOS seatbelt profile is a best-effort privacy boundary, not a
  container; a harness may still read its own runtime state (for example
  OpenCode's session database) while it is running, so use a separate OS
  account or container when that matters;
- ad-hoc signed beta builds are not notarized by Apple.

## Hardening recommendations

- Run the relay on a dedicated machine or OS account.
- Put the relay behind Tailscale or another authenticated private network.
- Keep `sandbox = true` unless a harness needs broader filesystem access.
- Use FileVault or an encrypted volume for the relay database.
- Pair devices with short-lived codes and revoke devices you no longer use.
- Keep Dunlin, your harnesses and macOS up to date.

## Release security gates

The `0.1.0-beta.4` hardening adds `cargo deny --locked check advisories licenses bans
sources` for the shipped macOS arm64 graph and CodeQL scans for Rust, web,
Python and workflow sources. No advisory exemptions are configured. All five
obsolete GPUI/syntect/font/image packages found by the initial audit have been
migrated out of the entire lockfile, with maintained local forks, preserved
licenses and verified upstream provenance. `scripts/verify_vendor.py` checks
their complete trees; `deny.toml` bans reintroduction. [Migration](vendor/README.md)
and [acceptance evidence](docs/release-acceptance.md).

`python3 scripts/repository_policy.py --repo corapoid/dunlin` audits rules read-only.
The explicitly approved solo-maintainer policy requires PRs, eight strict checks
bound to the GitHub Actions app, resolved threads, signed commits and squash-only
merges, with no bypass actors. Its remote audit passes. Zero independent approvals
is intentional, not a claim of second-person review; CODEOWNERS documents ownership.
GitHub signs the squash merge; no personal key or reviewer is impersonated.
Release source authorization independently verifies the actual mainline merge,
trusted PR/source checks and open CodeQL security alerts via read-only APIs, without
giving build jobs owner/admin credentials. Manual GUI/TCC/real-host acceptance is
explicitly deferred only for `0.1.0-beta.N`, never marked passed or reused for stable/RC.

Artifact provenance, Apple signing/notarization, developer commit/tag signatures,
and manual/deployed-host acceptance are distinct evidence. None substitutes for
another, and a passing checksum/native smoke must not be advertised as all of them.

Implementation PR #6 and its signed mainline commit passed all required checks,
aggregate CodeQL and read-only workflow source authorization. Both CI binary
attestations were independently verified for the exact mainline source/workflow.
Six proven analyzer false positives are individually documented, not query/path
exclusions. [Final evidence and manual boundaries](docs/release-acceptance.md).
