# Manual deployed relay acceptance

This suite is **manual-only**. It is not part of GitHub Actions, release gates,
`scripts/release_qa.py`, native-smoke discovery, or ordinary `cargo test`.
Importing the Python package does not start anything. Run commands explicitly.

The client uses Dunlin's production HTTP, ACP, terminal and OpenSSH transports.
The server runs a real relay and **OpenCode V2 `opencode acp`**, not a mock ACP
agent. Anonymous `opencode/*-free` models are discovered with a fresh HOME and a
private OpenCode server. Paid models, inherited accounts and automatic paid
fallbacks are forbidden. Availability and anonymous rate limits are external
dependencies: failure is reported, not silently skipped or authenticated.

## Requirements

- Apple Silicon macOS with an interactive desktop for native UI checks.
- Python 3.11+, OpenSSL, Git, OpenSSH and local OpenCode V2.
- Lima for Linux; Tart plus a pinned prepared image for macOS.
- Source builds use the pinned Rust/OpenCode versions shown by `--help`.
- Enough space for a disposable 16 GiB Linux disk, Rust toolchain and build.

The Linux recipe uses a digest-pinned Debian 13 ARM64 image with systemd,
bubblewrap, Node.js/npm, Python, Git and Secret Service tools. There are **no host
mounts, SSH agent forwarding, or inherited personal SSH keys**. The tested SSH
connection uses a new per-run client key and an explicitly pinned VM host key.
Lima's own generated config disables host-key verification; it is intentionally
not used as the configuration of the system under test.

## Quick start

From the repository root:

```sh
cargo build -p dunlin --locked
python3 scripts/remote-e2e.py doctor --provider lima
python3 scripts/remote-e2e.py list
python3 scripts/remote-e2e.py run --profile linux-core
```

To exercise the complete native remote save/chat/restart workflow, explicitly
consent to fixture Keychain entries in the current macOS login:

```sh
python3 scripts/remote-e2e.py run --profile linux-ui --build --allow-keychain
```

`run` creates a fresh server, builds/copies the relay, installs a pinned OpenCode,
starts a uniquely named user service, pairs clients, executes assertions and
cleans its resources. It never replaces the installed desktop app or a personal
relay. No provider sign-in is performed. Prompts and projects contain only
synthetic fixture data; anonymous model requests still leave the machine.

### Profiles

The authoritative inventory is [the runner](../../scripts/remote_e2e/runner.py).

| Profile | Scope |
|---|---|
| `linux-core` | Pairing, expiry/revocation, grants, files/conflicts, Git diff, snapshot/history transfer, real ACP, resume, permissions, cancel, concurrency and PTY |
| `linux-ssh` | Real production bootstrap, unknown host key, wrong client identity and occupied local port |
| `linux-tls` | HTTPS/WSS/PTY with a per-run CA, and rejection of untrusted, wrong-name and expired certificates |
| `linux-ui` | Real pairing controls, remote project selection, keyboard edit/save, real prompt, then a fresh app process restoring Keychain pairing and the same ACP conversation with a second prompt |
| `linux-resilience` | Network stalls/reset, service restart/crash, reboot with lingering, sandbox startup failure |
| `linux-keyring` | Production OS credential-store roundtrip and unavailable Secret Service failure |
| `linux-install` | Candidate installer, real harness recipe and archive through a private verified HTTPS mirror, then a repeat installation |
| `linux-upgrade` | A distinct previous archive and fresh old DB upgraded to the candidate, preserving token/project/files |
| `linux-full` | All Linux checks; requires the explicit inputs/consents below |
| `linux-headless` | All Linux server/transport checks, without native UI or the client's macOS Keychain prerequisite |
| `macos-full` | The corresponding launchd/seatbelt/macOS host checks in Tart |
| `remote-host` | A dedicated SSH host plus an explicitly supplied real-network HTTPS endpoint |

There is no default `all` command that silently selects an external host, writes
to Keychain or performs account authorization. Use `--only NAME` repeatedly to
select checks from a profile. A selected check with missing prerequisites is
`BLOCKED` and makes the command fail. For TLS checks include `tls-setup` first;
for `resume` include `acp` first.

`--model opencode/NAME-free` is an explicit per-run selection, including when
reusing a prepared environment. It must be present in the anonymous catalog;
there is no automatic model/account fallback. A service-wide `provider.free-limit`
may affect multiple free models. Stop model-dependent acceptance until quota is
available again; changing HOME, identities or network routes is not a remedy.

### Reuse while diagnosing

```sh
python3 scripts/remote-e2e.py prepare --profile linux-core
# Use the exact directory printed by prepare:
python3 scripts/remote-e2e.py run --profile linux-core --run-directory "$RUN" --keep-environment
python3 scripts/remote-e2e.py inspect --run-directory "$RUN"
python3 scripts/remote-e2e.py cleanup --run-directory "$RUN"
```

`--keep-on-failure` retains a failed environment. `--keep-environment` retains it
even after success. Retries archive earlier reports as `attempt-*.json`; they do
not overwrite failure evidence. Reuse is not clean-machine installation evidence.
Do not run two commands against the same run directory concurrently.

## Explicit side effects

Native pairing uses the real macOS Keychain. Select `--allow-keychain` only in a
disposable macOS login when clipboard/focus, OS dialogs or credential-store
interaction must not affect your current session. Fixture host IDs are journaled
before writing and removed after the run. HOME isolation alone does not isolate
Keychain, TCC or the user's global OpenSSH configuration.

The consented native UI/Keychain subprocess uses the OS login's real HOME:
Security.framework cannot resolve its default file Keychain under an empty HOME,
and can misleadingly report canceled authorization. This does **not** unlock a
locked Keychain or bypass its access controls. The client keeps private XDG
directories and a fixture-only `DUNLIN_DATA_DIR`; API-key variables, token
overrides, SSH agent and provider configuration environment remain absent.
Local agent and hosting CLIs are blocked in the client's PATH; the tested
OpenCode process runs only on the separate server, not in the login home.
Only the consented native probes use that HOME; catalog discovery, headless
clients and the relay's real OpenCode agent retain their isolated homes.
No account or API key is needed by the server-side free model.

The runner restarts the relay between the two app launches. The UI report
includes two process IDs, one unchanged ACP session/paired host,
the real agent/model, Keychain read evidence and named GUI steps. It checks the
saved file directly on the separate server and leaves a local project canary
unchanged. A fresh app must display the earlier reply and submit a second real
prompt in the same conversation, not silently create another one.
Remote ACP replay arrives before the `session/load` response. The client accepts
that replay only for the pinned remote session and marks it complete when the
load finishes; it must not discard history merely because loading is still in
progress. Local sessions keep their existing snapshot/replay suppression policy.

The public installer and normal SSH bootstrap are tested only in disposable VMs:
they write normal installation/service locations and may install system packages.
The separate deployed fixture uses a uniquely named user service and private DB.
The installer profile defaults to the candidate: it packages the built relay and
current harness recipe with checksums, serves them inside the VM over HTTPS and
passes `DUNLIN_RELEASE_BASE_URL` plus a process-local `CURL_CA_BUNDLE`. Downloads,
verification, PTY dialogue and package installation remain real; system trust is
unchanged. The version comes from the candidate workspace.
`--published-installer --release-version VERSION` instead tests immutable release
assets. It never resolves `latest`. The release version is not necessarily the
checkout's candidate binary. Upgrade needs `--previous-relay-artifact FILE` and
`--previous-relay-sha256 HASH` and rejects identical old/candidate binaries.

For an already reviewed relay archive or raw binary, use `--relay-artifact FILE
--relay-sha256 HASH`. Only a single regular `dunlin-relay` archive member is
accepted. Source deployment records the checkout commit, dirty flag and archive
checksum; review uncommitted source before uploading it to an external host.

## macOS and real hosts

Pass `--provider tart --host-config FILE` with a digest-pinned macOS image. The
image must have a dedicated logged-in test account, Remote Login, Python, Node.js
22+/npm and source-build tools, or provide a matching prebuilt relay artifact.
An unlocked login Keychain and GUI session are explicit acceptance prerequisites.
Tart is not installed implicitly and no system login password is collected.

Example host configuration (replace placeholders; do not commit private keys):

```json
{
  "dedicated_test_host": true,
  "image": "registry.example/test-macos@sha256:REVIEWED_DIGEST",
  "hostname": "test-server.example",
  "user": "dunlin-test",
  "port": 22,
  "identity": "/absolute/path/to/test-key",
  "known_hosts": "/absolute/path/to/pinned-test-known-hosts"
}
```

Tart obtains the VM IP itself. Its pinned `known_hosts` must match that image and
the assigned address (or a deliberately configured known-hosts pattern). The
`ssh-host` provider uses `hostname`; `image` is unnecessary. No host, personal
SSH alias or key is inferred. Existing hosts are never rebooted without
`--allow-reboot`; changing lingering requires `--allow-system-changes`.

For a real TLS/Tailscale test, configure the endpoint to reach **this test
relay**, then pass `--endpoint https://...`. The production trust store is used,
not the local fixture CA. Tailnet enrollment/Serve approval remains an operator
action: the runner does not log into a tailnet or modify existing Serve routes.
Native Linux x86_64 needs its own matching host/image; an ARM64 VM is not evidence
for x86_64. Additional distributions can be selected with `--vm-template FILE`.

## TLS, faults and boundaries

The local PKI is per run. Both HTTP and WebSocket verify the supplied CA,
certificate validity and host name; no `insecure` option or system trust change
exists. On Lima only the TLS listener is forwarded separately for this profile,
so its HTTPS/WSS requests do not use Dunlin's SSH tunnel. Other providers can
exercise verified TLS over a TCP tunnel; the report distinguishes that from a
real-network, publicly trusted endpoint.

The fault proxy never logs payloads. Scenarios assert bounded errors and recovery,
not transparent resumption of every operation. A lost mutation response does not
establish whether the operation ran: clients must refresh state, not blindly
retry. Reboot acceptance explicitly uses Linux lingering; it does not claim
unattended unlock of a Secret Service keyring or macOS login Keychain.

A small startup guard runs **inside the actual relay agent sandbox**, verifies a
project file is readable and a server-secret canary is not, records evidence,
then execs the real OpenCode. This is not a replacement protocol implementation.
PTY shells have different privileges from sandboxed ACP agents.

## Evidence and infrastructure regression tests

Each private run directory contains `report.json`, `summary.md`, JUnit XML,
checksums, redacted client/server/service logs and owned-resource manifests.
Native assertion failures retain structured GUI evidence. `PASS`, `FAIL` and
`BLOCKED` are separate; success requires every selected check and cleanup to pass.
Cancellation is not success. `cleanup` is idempotent and retains evidence.

Run the runner's infrastructure tests explicitly:

```sh
python3 -m unittest discover -s scripts/remote_e2e/tests -v
```

These tests exercise ownership, redaction, process deadlines, free-model policy,
network fault mechanics and separation from automatic entry points. They neither
start a VM nor call a model. They live outside `scripts/tests` so existing
automatic Python discovery does not execute the new suite.

Implementation does not imply a platform/profile passed. Use the actual report
for this checkout; unavailable Tart images, external hosts or rate-limited models
must remain visible as missing evidence. UI fixtures use in-process GPUI events,
not proof of physical input, IME, VoiceOver or TCC acceptance.

[Quality index](README.md)
