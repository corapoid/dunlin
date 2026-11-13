# Dunlin Web application — implementation preview

This is the new application, not the Astro marketing site at `../../web`.
Current scope: browser pairing, optional OIDC login, read-only live granted projects and **your own**
session index, opt-in workspaces/memberships and explicitly granted logical team
projects, plus opt-in AHP transcripts, explicit read-only sharing, queued text prompts and queue/cancellation controls for existing actors.
Complete interactive chat,
multi-host orchestration and workflow
execution are not implemented here yet. See `../../implement/README.md`.

## Run

Node 22.12+ (24 LTS recommended). Install locked dependencies, then:

```sh
npm ci --ignore-scripts
npm run check
npm test
npm run build
```

Start an initialized relay with an existing granted project:

```sh
cargo run -p dunlin-relay -- serve --web-root apps/web/dist
```

For local development, run `npm run dev` and open `http://127.0.0.1:5173`.
Its fixed loopback proxy preserves Host and Origin. Do not enable permissive
CORS, `changeOrigin`, external dev bindings or arbitrary proxy targets.
Production: serve build from the relay and publish it through **private
Tailscale Serve**, never Funnel. All inter-machine Dunlin traffic stays on
Tailscale; HTTP/WS is allowed only on the same computer's loopback.

The build directory is opt-in: `--web-root PATH`, TOML `web_root`, or
`DUNLIN_RELAY_WEB_ROOT`. The environment override follows existing relay config
precedence. The root must contain a completed `index.html`; use only a trusted
build directory, never a user's home or project checkout. Only `/`, `index.html`,
`REUI-LICENSE.txt` and `assets/` are served; unknown API/source paths remain 404.
The server supplies a same-origin CSP and does not enable CORS or a public bind.

Generate a one-use code with `dunlin-relay pair USER`. Browser sessions expire
after 8 hours without sliding renewal. Separate hashed credentials are persisted
in SQLite, so an unexpired cookie survives server restart. Logging out revokes
that browser device. The browser
cookie cannot authenticate the native bearer API. There are no tokens in
JavaScript storage, URLs or pairing JSON responses.

Backups include credential hashes and device revocation state. Restoring an old
database can restore older access decisions; rotate/revoke affected devices
before reopening access after a restore. A complete verified backup/restore
procedure (including artifacts/keyring) remains part of E10, not this preview.

## Live contract

`/v1/web/live` is a bounded, same-origin, cookie-authenticated WebSocket. Send
`{"type":"subscribe","protocol_version":1,"project_id":null}` for projects,
or select a granted project ID for private session summaries. In-process store
writes trigger payload-free invalidations and scoped snapshots, not HTTP polling.
Reconnect always gets a new authoritative snapshot. This catalog protocol is
**not AHP** and does not replay mutations. Other processes writing SQLite do not
yet publish invalidations; reconnect to see their catalog changes. Revocation
is rechecked by the heartbeat even if another process revokes the device.

## Teams and workflow definitions

Relay administrators may explicitly bootstrap a workspace from **Zespół**.
Workspace owners/admins manage existing relay users, with revision-checked writes
and last-owner protection. Membership does not expose private sessions, imply a
project grant, or grant host filesystem/Git authority. Logical team projects use
separate IDs and grants; the legacy native project catalog remains unchanged.

Cookie-authenticated routes under `/v1/web/workspaces/{workspace}/projects` expose
the logical catalog and project grants. Workflow routes under
`/{project}/workflows` store validated definitions and revision-checked drafts;
`/{workflow}/publish` creates an immutable, SHA-256-addressed version. Publishing
a definition **does not execute or deploy it**. No run endpoint or AHP capability
is advertised for this unfinished engine.

## Personal MCP connections

**Połączenia MCP** lists the signed-in user's connections in the selected workspace.
An operator configures trusted providers in relay.toml; users choose a provider,
name their connection, and sign in on the provider's page. Local disconnect and
cancelling incomplete sign-in use revision-checked writes. Local disconnect does
not claim to revoke the provider's OAuth grant.

**Sprawdź narzędzia** explicitly reads the provider catalog through the server's
OAuth broker. It shows the confirmed protocol version and observed tool names;
catalog membership does not grant execution rights. An expired local token can
rotate before dispatch. Lost refresh responses remain durably blocked, and a
provider authentication error never automatically retries the MCP request.

An uncertain write requires an explicit list refresh before another mutation.
The browser never automatically retries a write. Tokens stay on the server;
the one-use return preference stores only user/workspace IDs in sessionStorage.
The panel checks the expected user on each MCP API request and discards late
responses after leaving the view. Runtime tool routing is a separate integration.

## Isolated verification

`scripts/test_web_sandbox.py` uses cached images and readonly source snapshots,
empty credentials/HOME, no external network, and hard resource limits. Provision
test tools separately (network needed only for package/image downloads):

```sh
docker build -t dunlin-web-checks:rust-1.99 - < scripts/web-checks.Dockerfile
docker build -t dunlin-web-checks:browser - < scripts/web-browser-checks.Dockerfile
```

Pass an explicit local socket with `--docker-host unix:///path/to/docker.sock`.
Suites: `probe`, `frontend`, `typecheck`, `build`, `domain`, `server`, `clippy`,
`browser`, `tooling`, `docs`. Frontend builds require Linux-native dependencies passed through
`--node-modules`; macOS native packages cannot be reused on Linux.

The `browser` suite builds production assets and runs the real React application
against a temporary Rust relay in Chromium. It needs a separately installed
`apps/web/e2e` locked driver (`npm ci --ignore-scripts`) via
`--browser-node-modules`. The test receives one-use fixture pairing codes on
stdin, never through URLs or process arguments. Browser execution is offline
inside the container; Chromium's internal sandbox is disabled there, so the
outer container isolation is essential. CI explicitly runs this acceptance test
with `--ignored`; ordinary `cargo test` does not silently claim browser coverage.

The focused `--browser-test mcp_oauth` scenario uses two local sites and a fixture
OAuth provider to exercise the real connection panel and cookie behavior.
Safari/Firefox, real Tailscale Serve, provider OAuth and native-keyring acceptance remain
separate unverified gates. Exact local evidence is in `implement/progress.md`.

## ReUI provenance

The public `@reui` registry is configured in `components.json`. The Frame
composition is adapted from the free Base Nova registry, retaining its panel
structure and tokens while omitting unused CVA variants. Source and license:

- https://reui.io/r/base-nova/frame.json
- https://reui.io/docs/components/base/frame.md
- https://github.com/keenthemes/reui/blob/main/LICENSE.md
- `REUI-LICENSE.txt` (also included in the build)

No premium blocks, icons or templates are included. Further components should
be selected through authenticated ReUI MCP and verified against documented APIs.

### Logowanie OIDC

OIDC jest opcjonalne. Operator najpierw tworzy lokalnego użytkownika przez
`dunlin-relay user add` i odczytuje jego ID przez `dunlin-relay user list`.
To lokalna rola użytkownika wyznacza authority; claims `email`, `name`, `groups`
ani role dostawcy nie tworzą konta i nie nadają uprawnień. Istniejący lokalny
administrator pozostaje bezpiecznym sposobem bootstrapu i odzyskania dostępu.

Przykład w pliku konfiguracyjnym relay (podstaw własny issuer, subject i user ID):

```toml
[oidc]
issuer = "https://identity.example.com"
client_id = "dunlin-web"
redirect_uri = "https://relay.tail123.ts.net/v1/web/oidc/callback"
client_secret_env = "DUNLIN_OIDC_CLIENT_SECRET"
allowed_origins = []

[oidc.bindings]
"subject-from-provider" = "usr-local-user-id"
```

`client_secret_env` to nazwa zmiennej środowiskowej, nie sekret. Dla publicznego
klienta można pominąć ten parametr; PKCE S256 obowiązuje w obu trybach.
Klient poufny używa `client_secret_basic`; inne metody uwierzytelnienia klienta
(np. `private_key_jwt`) nie są obecnie obsługiwane. Nie
wpisuj sekretu do repozytorium. Jeśli provider umieszcza JWKS lub token endpoint
na innym originie, operator musi dodać dokładny origin do `allowed_origins`.
Discovery, JWKS i token exchange nie podążają za redirectami; obowiązuje HTTPS
z wyjątkiem jawnie skonfigurowanego loopback HTTP do lokalnych testów.

Zarejestruj u dostawcy dokładny `redirect_uri`. Callback Dunlin musi używać
prywatnego Tailscale Serve lub loopback; nie wymaga publicznego wystawienia
serwera. Przeglądarka użytkownika musi mieć dostęp do tailnetu. Istniejąca
blokada hostowych kont Git nadal może ograniczać relay do administratorów.

Przycisk „Zaloguj kontem organizacji” pojawia się na ekranie logowania po włączeniu konfiguracji.
Flow cookie jest HttpOnly/SameSite=Lax i ważne 5 minut; końcowe cookie sesji
pozostaje HttpOnly/SameSite=Strict. Stan jest jednorazowy i związany z browserem,
nonce i PKCE pozostają na serwerze. Restart wymaga ponownego rozpoczęcia
niedokończonego logowania. Callback zawiera standardowy jednorazowy authorization
code; nie zapisuj pełnych URL callbacków w access logach/proxy/telemetrii.
Odpowiedź usuwa flow cookie, ustawia no-store/no-referrer i kieruje na `/`.
Provider access/refresh/ID tokens nie trafiają do odpowiedzi, storage ani logów.

Sesja trwa najwyżej 8 godzin i nie dłużej niż ważność ID tokenu. Nie ma sliding
renewal ani refresh tokenów. Logowanie wymaga podpisu RS256, ES256 lub EdDSA,
zgodnego issuer/audience/authorized party, nonce, exp, świeżego iat oraz at_hash,
jeśli provider go zwraca. Lokalny logout odwołuje urządzenie; nie wylogowuje
sesji u dostawcy. Po zastosowaniu zmienionej konfiguracji OIDC przez restart relay (także po
zmianie bindings) wcześniej wydane sesje OIDC tracą lokalną authority przy
następnym sprawdzeniu. Zmiana dowolnego bindingu wymaga ponownego logowania
wszystkich użytkowników OIDC. Sesje pairing
pozostają odrębną ścieżką. Odwołanie sesji tylko u dostawcy nie ma back-channel
powiadomień: ważne lokalne cookie działa do expiry lub lokalnej revocation.

Testy z lokalnym podpisującym dostawcą nie zastępują odbioru z prawdziwym OIDC,
Tailscale Serve i docelową polityką operatora.

### Automation identities

Workspace owners/admins may create a dedicated service principal with
`POST /v1/web/workspaces/{workspace}/service-principals` using their same-origin
browser cookie. The JSON body contains `project_id`, `name`, `permissions`
(`view_project` required; optional `run_workflow`) and `expires_in` in seconds
(1 second to 30 days). The response returns principal metadata and its token
once, with `Cache-Control: no-store`. Store it in the automation's secret store;
never put it in a URL or browser storage. No token is returned when listing
principals with GET on the same route.

`POST /v1/web/workspaces/{workspace}/service-principals/{id}/revoke` with
`{"expected_revision":1}` permanently revokes the credential. Revisions protect
against stale writes; there is no operation that reactivates a revoked token.
The catalog is bounded to 256 principal records per workspace, retaining revoked
records for audit. Automation identities have their own lifecycle; removing a
human membership does not implicitly delete a workspace-owned automation.

A service token is accepted only as `Authorization: Bearer dsp_…` by the dedicated
API. Currently it can read an immutable published workflow version from
`GET /v1/service/workspaces/{workspace}/projects/{project}/workflows/{workflow}/versions/{version}`.
It cannot read drafts, private transcripts, host files, or credentials and does
not authenticate browser/native APIs. A browser cookie does not authenticate the
service API. Every read reloads expiry/revocation and checks the exact project.
The `run_workflow` grant is reserved for the workflow engine; this release still
has no run endpoint. It does not claim to execute a graph or make human approvals.
Host grants and controller/worker enrollment remain separate implementation work.

### Local checkout binding

A logical team project is independent of host directories. A relay administrator
who also has workspace `ManageHosts`, the project's `ManageProject` grant and an
existing native checkout grant may configure a local binding:

- `GET /v1/web/workspaces/{workspace}/projects/{project}/binding` returns
  `{binding, configuration_current, can_manage}`. Current configuration requires
  the caller's own fresh checkout Read authority. Missing metadata permission
  leaves authorized saved configuration visible with `configuration_current:false`.
  This does not indicate whether an agent is running or ready. Physical paths and inode identifiers are
  omitted from the response. Existing bindings are visible only with both the
  logical view grant and a native checkout grant.
- `PUT` on that route accepts `{native_project_id, expected_revision}`. Use null
  only for the first binding; later rebinds require the current revision.
  Linked worktrees additionally require an explicit `metadata_approval_revision`
  from the operator's local metadata approval and the caller's own metadata Read
  grant. Ordinary requests omit this optional field.
- `POST .../binding/revoke` accepts `{expected_revision}` and invalidates the
  binding without removing the underlying native project or its grants.

All mutations require the browser cookie and same-origin protection. The
operation never grants checkout access to other team members. A runtime consumer
must recheck the exact binding revision, current logical/native grants, device
and host-account policy before dispatch. Changing a root/symlink target,
replacing its directory or `.git` directory, or changing project settings requires
an explicit rebind. Ordinary content/branch edits do not change directory identity.

The adapter identifies the current coordinator as `host_id: local`. It supports
ordinary directories/internal Git metadata and explicitly approved linked worktrees.
A linked approval pins checkout identity, settings and the exact operator approval
revision; it does not borrow the operator's grants. Reapproving metadata requires
an explicit rebind, while every read/execute consumer checks the current caller's
own authority. Nested repository paths and unapproved external metadata remain denied.

In **Zespół → projekt → Lokalny checkout**, administrators enter the native
project ID and optional linked approval revision. The editor uses the saved CAS
baseline and clears drafts on catalog, scope or identity changes. Writes are
single-attempt; conflicts or uncertain responses require an explicit fresh read.
Revocation remains available without the checkout filesystem or metadata grant.

### Experimental server-owned ACP actor

Disabled by default. The server config `experimental_interactive_runtime = true`
requires transcript storage and a platform sandbox. Use only the local fixture
until real harness acceptance is performed. Cookie/origin protected bootstrap is
`POST /v1/web/workspaces/{workspace}/projects/{project}/interactive-sessions`
with `{model, mode, max_seconds, max_output_bytes}`. The profile must be confirmed
by ACP config options. Time is 1–1800 seconds per turn, output 1 KiB–4 MiB, with
8 actors maximum, a one-hour actor lifetime and 60-second idle timeout.

`GET /v1/web/interactive-sessions/{id}` returns private metadata without prompts;
`POST .../{id}/stop` requires `{expected_epoch}`. These are bootstrap/diagnostic
routes, not UI polling. Text enqueue uses the separate AHP dispatch transport;
general chat/workflow execution capabilities stay disabled. Process ownership is
independent of observers. Grant/device/binding/settings changes, time/output
limits and explicit stop interrupt work and preserve uncertain turns as unknown.
SIGINT/SIGTERM stop actors and reject new starts. Crash recovery does not replay
prompts. ReadOnly starts are rejected pending a kernel adapter. Client-provided filesystem/terminal tools remain unavailable. Valid ACP tool
permission requests use the one-use approval flow below; malformed challenges fail closed. This actor
does not provide token/dollar budgeting or prove business success at end_turn.


The journal now stores a typed AHP chat projection using the pinned official
1.0.0 SDK reducers. Source events, actions and the resulting snapshot commit
together; one ACP event may emit multiple independently ordered actions.
The opt-in observer transport exposes this projection as described below.
Unsupported legacy content retains its original transcript and explicitly blocks
projection reads pending a compatible adapter. Unknown outcomes remain errors.


### Experimental AHP observer

Both `experimental_interactive_runtime` and `store_transcripts` must be enabled.
Authenticated `GET /v1/web/session` advertises `capabilities.ahp_observer` and
`ahp_endpoint: /v1/web/ahp`. The separate same-origin cookie-protected WebSocket
supports released AHP initialize, listSessions, subscribe, unsubscribe and
reconnect. Root/session/chat snapshots and ordered actions are generated by
pinned 1.0.0 types/reducers; offers negotiate supported 1.0/0.9 ranges.
Queued plain user text, queue edits/removal/reorder and active turn cancellation
are supported for an existing owned actor.
No AHP createSession, steering or resource operation is exposed. One-use tool
approval is described below.

The “Sesje” panel receives live notifications, replays after disconnect and
restores snapshots after restart or an unavailable replay range. It does not
poll REST. Sessions are private to their owner and require fresh project grants;
revocation clears the protected view. Bounded snapshots (32 channels, 900 KiB)
and replay (128 actions, 900 KiB) fail explicitly or request a snapshot instead
of truncating a transcript. Large history pagination remains unfinished.

Schema 20 persists session projections and device-scoped negotiated clients.
Source events and both projections commit atomically before notification.
Client records expire after eight hours; a new server boot forces a fresh cut.
The official SDK reducer does not aggregate parent session status from chat
updates. This adapter computes the current summary and emits root summary
updates; a generic session-only mirror cannot yet reproduce every snapshot.
Full conformance and reconciliation for other interactive operations remain open.
Raw tool input is preserved as standard inline `toolInput`; agent reports never
stand in for a human permission decision.


### Queued text dispatch

The existing opt-in AHP transport accepts typed `dispatchAction` notifications
with a new `chat/pendingMessageSet`, `kind: queued`, a client message ID and plain
user text. A live actor, fresh ownership and CreateSession authority are required.
The actor keeps its starter device, lease, checkout binding and policy authority;
dispatch never creates or restarts a process. Other actions and message overrides
are rejected; queue/cancellation controls are described below. The browser composer requires chat interactivity `full` plus the
session summary extension `_meta["dunlin.queueText"] === true`.

Schema 21 persists the exact outcome under `(device, clientId, clientSeq)` with
a fingerprint of the typed channel/action payload. Accepted enqueue, source,
projection, origin and ledger commit together. Retrying an identical command
returns the original envelope, even after reconnect/reopen or actor stop; a
changed payload conflicts. Client message IDs stay distinct from server turn
IDs. Rejections also have durable ordered envelopes and never enter the reducer.
The ledger is independent of the eight-hour client negotiation TTL and has no
automatic expiry (8192 global/1024 per device); exhausted capacity fails closed.

The browser keeps a memory-only outbox with immutable command identities and
confirmed state plus optimistic overlays. It resolves origin before filtering
old server sequence numbers, so an old outcome after a newer snapshot can settle
the outbox without reapplying an old action. After reconnect it rebuilds the
baseline and retries only unacknowledged commands unchanged. Reload does not
restore or resend prompts. Lost client identity requires manual history review.
Offline disables sending while retaining the draft; rejected text returns to the
draft. UTF-8 prompt and complete encoded-frame limits are checked before send.
Permanent ledger/identity conflicts terminate automatic retry; ordinary bounded
replay loss still requests a snapshot. Acceptance means queued, not executed or
successful. Unknown outcomes are never automatically dispatched again.


### Queue controls and cancellation

With `_meta["dunlin.controlTurns"] === true`, full chats expose editing and
removing queued text, moving it up/down, and cancelling the active turn.
Controls use the same immutable outbox, durable outcome ledger and fresh actor
admission as text dispatch. Queued message replacement preserves its position;
removal retains an unsent cancelled SQL tombstone without inventing a chat turn.
Reorder ignores stale unknown IDs and appends omitted queued IDs in their prior
order. The host emits the complete canonical order to avoid an ordering defect
in the pinned SDK. A consumed message cannot be edited back into the queue.

Schema 22 records fenced cancellation intents. Accepted `chat/turnCancelled`
ends host-side chat processing immediately, using producer monotonic duration
(or an explicit unknown-duration marker). SQL remains busy until the original
ACP prompt responds. The actor sends one `session/cancel`, waits at most five
seconds within its existing deadline, and starts no later prompt before settlement.
If cancellation is already pending before prompt delivery, the prompt is not sent.
Completion versus cancellation is linearized by the journal transaction.

The UI shows pending cancellation separately. Cancellation cannot undo tool
side effects. If the agent fails to settle, the actor process group is terminated,
the runtime outcome becomes unknown, queued work remains unexecuted, and the
session advertises `dunlin.cancellationUnconfirmed` with an error status. This is
not reported as confirmed agent cancellation or task success. Raw late ACP updates
remain in the journal but cannot append to the already cancelled chat turn.
The cancellation receipt uses recovery quota reserve. Tools that were unfinished
in a terminal turn render cancelled/skipped, matching the pinned SDK reducer.
ACP JSON cannot supply local evidence that the host never sent a prompt.


### One-use tool permission decisions

An ACP `session/request_permission` becomes a host-owned AHP pending-confirmation
challenge for the current tool/turn. The host captures the complete reported tool
input, request identity, lease/runtime, project authority revisions, starter device,
checkout binding revision and harness fingerprint. It creates opaque option IDs
that include a fresh nonce. Only `allow_once`/`reject_once` options are mapped;
persistent allow/reject policies are excluded. A virtual deny is available when
the peer provides none. The root advertises `_meta["dunlin.approveTools"]` only
for a currently authorized owned actor. Private session ownership plus human
CreateSession authority is required; this does not change workflow Review or
separation-of-duties policy.

The browser sends a minimal `chat/toolCallConfirmed` with the advertised option
ID. The first valid decision wins atomically with the shared outcome ledger.
The server supplies canonical `user-action` confirmation evidence; the client and
agent cannot manufacture it through metadata. A stale nonce, changed operation,
expired challenge, revoked authority or competing decision is rejected. An
identical retry only returns the existing outcome. The browser preserves immutable
outbox identities across reconnect and never makes an old receipt approve a new
challenge. Canonical revocation metadata replaces earlier live proof metadata.

Before forwarding the selected raw ACP option, the actor rechecks the deciding
device, OIDC/host policy, exact consumed nonce, lease/stamp, expiry and active
turn. Consumption is persisted before the external write, and delivery is
recorded only after flush. A consumed but unconfirmed delivery becomes unknown
on interruption/restart and is never re-sent. The reader continues processing ACP
frames while waiting for the human, so completion/cancellation invalidates pending
choices. The root limits each challenge to at most 120 seconds within the turn
budget (library bound 1–300 seconds), eight unresolved challenges per session,
256 total per session and 8192 global. Lifecycle writes use recovery reserve.

This authorizes one reported agent request within the existing sandbox; it does
not widen filesystem/terminal permissions, guarantee that the agent's report is
truthful, undo side effects, or prove execution success. Denial/revocation remains
visible even if a later agent report claims success. UI distinguishes pending
human input, pending forwarding and unknown delivery. No permission is restored
or granted automatically after restart.


### Udostępnianie sesji AHP

Właściciel może udostępnić całą rozmowę wskazanym uprawnionym odbiorcom.
Dostęp obejmuje transkrypt oraz parametry i wyniki narzędzi. Odbiorcy mają wyłącznie
odczyt; panel udostępniania i ostatnich 20 zmian audytu jest dostępny właścicielowi,
także gdy aktor sesji nie działa. Zapis wymaga aktualnej wersji: konflikt wczytuje
stan serwera bez automatycznego ponawiania zmiany. Nieaktywne wpisy są odznaczone;
zapis je usuwa, a ponowne nadanie wymaga jawnego zaznaczenia uprawnionego odbiorcy.
Katalog pokazuje do 256 kandydatów z oznaczeniem ograniczenia; istniejące
udostępnienia nadal można usuwać. Cofnięcie dostępu usuwa sesję i transkrypt z
widoku odbiorcy; ponowne nadanie ładuje aktualny snapshot całej historii.
