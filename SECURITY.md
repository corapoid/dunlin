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
  container;
- ad-hoc signed beta builds are not notarized by Apple.

## Hardening recommendations

- Run the relay on a dedicated machine or OS account.
- Put the relay behind Tailscale or another authenticated private network.
- Keep `sandbox = true` unless a harness needs broader filesystem access.
- Use FileVault or an encrypted volume for the relay database.
- Pair devices with short-lived codes and revoke devices you no longer use.
- Keep Dunlin, your harnesses and macOS up to date.

## Release security gates

The unreleased hardening adds `cargo deny --locked check advisories licenses bans
sources` for the shipped macOS arm64 graph and CodeQL scans for Rust, web,
Python and workflow sources. There are no blanket advisory exemptions. The first
audit found five unmaintained GPUI/syntect/font/image dependencies; the gate stays
red until reviewed remediation or explicit scoped exceptions, rather than hiding
the result. [Evidence and blockers](docs/release-acceptance.md).

`python3 scripts/repository_policy.py --repo corapoid/dunlin` audits rules read-only.
The intended release policy requires independent/code-owner and last-push review,
strict CI/security checks, signatures, and no unconditional bypass. Current remote
settings do not yet meet it. Configure a real second reviewer and commit/tag
signing before changing enforcement; never fabricate approval or automatically
remove the sole maintainer's recovery route. CODEOWNERS alone does not enforce review.

Artifact provenance, Apple signing/notarization, developer commit/tag signatures,
and manual/deployed-host acceptance are distinct evidence. None substitutes for
another, and a passing checksum/native smoke must not be advertised as all of them.
