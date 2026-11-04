# Use-only credentials

Dunlin can hold short-lived API credentials and let an ACP harness *use* them
without putting the value into the model context, the harness environment, argv,
project files or SQLite. This is a capability boundary, not a general secret
manager.

## Shape

- Values live in the macOS Keychain under `dev.dunlin.secrets`.
- Security-critical policy lives in a second Keychain item under
  `dev.dunlin.secret-policies`: allowed hosts, path prefix, methods, injection,
  named HTTP operations, TTL, project grants and CLI recipes. SQLite is only a
  display cache;
  a harness that edits it cannot widen the policy the broker enforces. Aliases,
  labels and descriptions are display metadata and may be changed in that cache,
  but they grant no additional host, method, project or recipe.
- The app runs a local broker on a Unix socket. A per-session token is generated
  before `session/new` and attached as a stdio MCP server:
  `Dunlin --mcp-vault`.
- The MCP proxy has no Keychain access. It forwards `secrets_list`,
  `secrets_request` and `secrets_run` to the app and returns results only.
- The app reads the value only after the policy checks and (when required) user
  approval. The outbound request is executed by the app, never by the harness.

The optional relay-side vault uses a separate namespace so a relay host never
shares items with the desktop app: values live under
`dev.dunlin.relay-secrets`, policies under `dev.dunlin.relay-secret-policies`,
and metadata in `vault.db` beside the relay database. The relay starts the same
broker on a `0600` Unix socket in its temporary directory and injects
`dunlin-relay --mcp-vault` into ACP `session/new`, `session/load` and
`session/fork`. It revokes that capability when the connection closes or a
lingering turn finishes. Relay keys are managed from the relay host:

```sh
dunlin-relay vault add github-work --host api.github.com --project dunlin --ttl 60
dunlin-relay vault list
dunlin-relay vault grant github-work dunlin
dunlin-relay vault revoke github-work dunlin
dunlin-relay vault remove github-work
```

`vault add` reads the value from a hidden prompt or `--value-file`. It supports
`--writes`, `--header`/`--prefix`, OAuth 2.0 refresh
(`--oauth-token-url` + `--oauth-client-id`) and AWS SigV4
(`--aws-access-key-id`, `--aws-region`, `--aws-service`). The OAuth stored value
is the refresh token; the AWS stored value is the secret access key. The relay
vault is disabled with a log message when the Keychain or storage is
unavailable, and the rest of the relay keeps running.

## Agent surface

- `secrets_list` returns public aliases, descriptions, allowed hosts/methods,
  named HTTP operations, recipes and expiry. It never returns the value or the
  internal id.
- `secrets_request` performs an HTTPS request with the credential injected by
  Dunlin. For a named operation the model supplies only the alias, `operation`
  and declared `params`; the host, method and path template stay policy. The
  free-form form supplies method, URL and optional allowed headers/body.
  Redirects are not followed, environment proxies are ignored,
  private/loopback/metadata addresses are rejected, and caller-supplied
  `Authorization`, `Cookie` and `Host` headers are refused.
- Injection is a policy choice: a custom header, HTTP Basic, a query parameter,
  OAuth 2.0 refresh, or AWS Signature Version 4. The OAuth exchange and AWS
  signing happen inside the broker. Neither the stored value nor the exchanged
  access token is returned to the harness; both are redacted from results and
  errors. OAuth token URLs must be plain HTTPS, and AWS signing uses only the
  policy's access key ID, region and service. OAuth refresh-token rotation is not
  persisted; use a non-rotating refresh token or replace the key if the provider
  rotates it.
- `secrets_run` runs a fixed CLI recipe. The recipe program and argument template
  are stored in the Keychain policy, not chosen by the model. The model may fill
  only declared parameters. Programs inside the agent's project are refused;
  other programs are hash-pinned when the recipe is saved. The command is executed
  directly, without a shell. Shells and general-purpose interpreters are refused
  as recipe programs.
- There is no `get`, `reveal`, export or clipboard operation for a stored value.

## Lifetime and grants

- Every key is a quick key: default 1 hour, maximum 24 hours. Expiry is enforced
  by the broker from the Keychain policy. The desktop app and relay purge expired
  values and policies at most once a minute while running and on the next launch.
- A key is inert until it is granted to a project. The grant list is part of the
  Keychain policy. Removing a project grant or revoking the key takes effect on
  the next request.
- Managed keys (`managed: true` in the Keychain policy) are auto-approved only
  within their host/path/method/grant/operation scope. Keys without that marker,
  or with `auto_approve: false`, open a native approval prompt with **Deny**,
  **Allow once** and **Allow for this session**. The session allowance is keyed
  by session, alias and policy revision, so editing the key, its hosts, methods,
  operations or grants invalidates existing allowances.
- Read-only project mode allows only GET/HEAD and refuses CLI recipes. A paused
  task budget disables the session's credential capability until work is approved.

## UI

- **Settings → Secrets** manages credentials, project grants, TTL, named HTTP
  operations, OAuth/AWS injection and CLI recipes, and shows recent audit
  metadata. The create/edit form can require approval for every use instead of
  auto-approving a managed key. Policy and audit reads run on the background
  executor, so a locked Keychain does not stall the window.
- An **HTTP operation** is a named, pre-approved method + host + path template
  with declared parameters (for example `issues-list`:
  `GET https://api.github.com/repos/{owner}/{repo}/issues`). Parameter values are
  percent-encoded; the model cannot substitute another host, method or path.
- **Secrets** in the bottom panel is the quick path: paste a key, choose a host,
  TTL and read/write scope, and create it for the current project. The value field
  is dropped after creation; the value is never displayed again.
- Deleting a credential removes the value, its policy and its SQLite row. Audit
  history remains, without the value, body, query string or headers.
- **Revoke all** removes every quick-key value and policy, revokes active and
  parked harness capabilities and requires a harness restart before new keys can
  be used. Audit history remains.

## Verification

- `cargo test -p dunlin-vault -p dunlin-domain -p dunlin-storage -p dunlin-agent -p dunlin-relay`
- `cargo test -p dunlin --bin dunlin`
- `python3 scripts/vault-smoke.py` — starts a mock vault service, launches
  OpenCode's ACP server, verifies that `session/new` causes it to spawn
  `dunlin --mcp-vault` and complete the local handshake. It sends no model prompt
  and does not touch the Keychain. Set `DUNLIN_BIN` to the bundled executable to
  repeat it against the app bundle.
- `./target/debug/dunlin --vault-keychain-smoke` (or the same flag on the
  bundled executable) — real Keychain round-trip of a throwaway value and policy;
  no real credential is read.
- `dunlin-relay vault ...` against a temporary relay database — real relay
  Keychain round-trip plus grant/revoke/remove.
- `python3 scripts/vault-live-smoke.py` — one real model turn through OpenCode
  ACP with a mock broker and mock `api.github.com` response. It verifies that the
  model discovers and calls `secrets_request`, and therefore consumes provider
  tokens. Set `VAULT_LIVE_MODEL` to choose a working provider/model.

## What this does not protect against

- A local harness runs with the user's permissions. It cannot read the Keychain
  item without the app/ACL, and the Keychain policy prevents it from widening the
  policy through SQLite. It can still delete its own display row, race a hash-pinned
  program between verification and execution, or attack the app process with a
  debugger. Use a separate OS account, VM or container for strong isolation.
- A CLI recipe necessarily gives the value to the child process environment. A
  same-user process that can inspect that child (for example with `ps`) can see it
  while the recipe runs. Keep recipes short and keys temporary.
- The allowed host is trusted with the value. Redaction removes exact and common
  encoded occurrences from responses and output, but a host that transforms or
  reflects the credential cannot be fully contained. Do not allow echo/debug hosts.
- Prompt injection can still cause *use* within the policy (for example a write
  method you enabled). The value itself remains unavailable to the model.
- The relay-side vault belongs to the relay host, not the Mac that started the
  session. Any relay user with a grant to the project can use a key granted to
  that project, within its policy. Protect the relay host account and Keychain,
  and grant only the project that needs the key.
- A relay host may still run harnesses with the host user's permissions. The
  broker prevents them from reading the value directly, but it does not create a
  new OS security boundary between relay users.
- Provider authentication for the model itself is separate. The harness must know
  the provider key (or use a local proxy); Dunlin's provider sign-in already keeps
  it out of the prompt.
