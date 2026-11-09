# Security boundaries

For vulnerability disclosure use [SECURITY.md](../../SECURITY.md). This page
describes boundaries, not a claim that the project passed a fresh security audit.

## Local agents and project trust

Trust controls whether Dunlin starts an agent for a project. Ask/read-only and
contract ceilings gate client permission responses; they do not turn a local
harness into an OS sandbox. Worktrees separate Git state, not operating-system
access. Ordinary workbench checks can execute project code with user permissions.
The Security workspace's explicit seatbelt check is a separate bounded runner.

Notifications, Accessibility, Screen Recording, Automation and Full Disk Access
are controlled by macOS/TCC. Dunlin can report/open settings and explicitly
request supported permissions; it cannot grant them to itself. Status inspection
and automated native fixtures do not replace fresh-account/manual TCC tests.

## Relay users and host accounts

Pairing uses a one-use code with a maximum 15-minute validity. Each device gets a
revocable token. Authenticated project grants, account ownership, lease checks
and per-operation roles bound API access. A remote operation must not fall back
to a local command when an endpoint is missing or access is denied.

Managed macOS seatbelt/Linux bubblewrap profiles and per-relay-user agent homes
add restrictions but are **not a promise of mutually untrusted multi-tenant OS
isolation**. Host `gh`/`glab` authorization is administrator-only and requires an
administrator-only relay. Use separate OS accounts/services for untrusted users.
See [deployment and migration boundaries](../administration/relay-setup.md).

## Credentials and sharing

Stored credentials have no agent access until explicitly configured/granted.
Use-only broker operations constrain host/method/path or a fixed recipe without
passing the value to the model. The destination or a recipe can still cause
side effects; an allowlist is not proof of application safety. Local and relay
vaults are separate platform stores. See [credential threat model](../user-guide/credentials.md).

Artifact share links are expiring bearer capabilities. Anyone with the token can
access allowed content until expiry/revocation; already downloaded copies cannot
be recalled. Database records, transcripts, audit logs and backup exports may
contain sensitive information even when they contain no credential values.

## Plugins and tools

WASM packs run through a hash-checked `wasmi` host, without WASI/host imports and
with fuel/memory/output limits. A hash is integrity evidence, not a publisher
signature. Declarative MCP server entries are not an arbitrary process-launch
permission. Built-in MCP tools have independent scope/grant checks. Governance
write tools enqueue proposals rather than silently changing project files.

The remaining acceptance matrix is in [quality](../quality/manual-acceptance.md);
historical evidence belongs in the [archive](../archive/README.md).
