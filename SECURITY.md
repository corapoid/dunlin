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
