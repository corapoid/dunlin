# Server setup and remote workspace

**Availability:** the latest [release](https://github.com/corapoid/dunlin/releases)
publishes the verified bootstrap script and macOS/Linux relay archives. For
source builds, use the commands in the [full guide](guide.md#remote-relay).

The relay starts and supports pairing, files and Git without an installed agent.
An agent is started only when an ACP conversation is opened. The desktop client
does not need local OpenCode, Codex or Claude Code to use a relay.

## One-command setup

Download `install-relay.sh` from the latest release and run it as your regular
service user:

```sh
bash install-relay.sh
```

Review `scripts/install-relay.sh` before running it. Run it as your regular
service user, not root. It collects the server name, Tailscale, service startup,
Git CLI and harness selections before downloading or installing anything.
Browser authorization, VPN approval and sudo still require human confirmation.
Set `NO_COLOR=1` for plain terminal output. `--help` has no installation effects.

Supported release targets are macOS Apple Silicon and Linux x86_64/ARM64.
Linux relay archives use static musl builds so the relay is not tied to the
build runner's glibc version. The optional Node.js runtime and hosting CLIs have
their own supported-OS requirements, checked again during real-machine acceptance.
Automatic Linux dependency installation supports apt and dnf distributions;
hosting CLIs are verified stable binaries from their official releases rather
than potentially stale distribution packages. Other Linux
distributions need equivalent packages and manual service setup. Packages
missing from a distribution's repositories stop installation with an error;
the installer never silently skips a selected component.

Relay archives and the harness script are checked against the release's
`RELAY-SHA256SUMS`. Existing accounts, grants and pairings are retained on reruns.
Previous launchd/systemd unit files are backed up before replacement. The
installed binary is `~/.local/bin/dunlin-relay`.

The curated managed catalog is:

| Agent | Stable npm package | ACP command |
| --- | --- | --- |
| OpenCode V2 | `@opencode/cli` | `opencode acp` |
| Codex | `@agentclientprotocol/codex-acp` | `codex-acp` |
| Claude Code | `@agentclientprotocol/claude-agent-acp` | `claude-agent-acp` |

The installer supplies a verified Node.js 24 runtime if Node.js 22+ is absent.
Agent installation is not provider authorization. Agent processes have dedicated
per-relay-user homes under `server/agent-homes/<user-id>`; existing host-agent
credentials are not copied into those homes.
An existing macOS relay keeps its legacy host agent home until the operator
explicitly runs `setup`/the installer. Merely upgrading the binary or changing a
server preference does not relocate its history. Opting into dedicated homes
retains the old files but requires new agent authorization and potentially new
conversations; transcript snapshots are not deleted. Linux uses dedicated agent
homes from the outset. Legacy environment/provider settings are not silently
rewritten.
The Authorize button opens a remote, sandboxed agent-setup PTY in that same
home, without requiring a project: `opencode auth login`,
Codex's device authorization, or Claude Code's login. It never starts a local
agent. Codex and Claude installations include both their native CLI and ACP
adapter so these setup commands are available.

## Tailscale

Tailscale is selected by default but can be skipped. The installer obtains the
latest stable official macOS standalone package or updates the Linux package.
It will not replace an existing incompatible macOS variant. Existing Serve
routes for another service are not overwritten.

Sign the server and client into the same tailnet. A first Tailscale sign-in can
create that private network; no router port forwarding is required. Tailnet
policies and device approval still apply. Tailscale Serve provides private
HTTPS; Funnel is not enabled by this installer. Serve may ask the tailnet
administrator to enable HTTPS certificates. The relay itself listens only on
`127.0.0.1:8787` by default.

In the client, Settings → Remote & SSH offers Tailscale download, opening the installed
app/sign-in and a state check. The client uses the operating system's Tailscale
connection, not a second embedded VPN.

macOS launch agents and the standalone Tailscale app start after user login.
On Linux, a user systemd service can start before login after explicitly enabling
lingering. A Secret Service keyring may still require unlocking. These are not
equivalent to unattended, reboot-tested system services.

## Pairing and management

The installation result includes the server endpoint and a one-time pairing
code. Codes are valid for at most 900 seconds from generation and become invalid
immediately after successful redemption. Expiry is checked on the server.
The client exchanges a code for a revocable device token stored in its Keychain;
the device does not disconnect when the original pairing code expires.

```sh
dunlin-relay pair admin                    # fresh code, default 15 minutes
dunlin-relay pair admin --ttl 5            # shorter lifetime
dunlin-relay setup --name studio-server    # friendly name, no DNS rename
```

Use `--db PATH` with these commands when using a non-default database.
Pairing opens the server workspace. The title-bar host switcher or Manage on a
paired server opens it again, even when the server has no projects or harnesses.
The panel shows the authenticated server name, platform, version and catalog.
Install/update and default-harness changes happen on that server only and
require a relay administrator. Default changes apply to new ACP connections.
Remote project harness settings never run local installation/discovery actions.
Older relays show an explicit update-required error; there is no local fallback.

Management jobs have bounded in-memory output, expire, can be cancelled, and
are visible only to the administrator who started them. Codes/output are not
saved to the transcript or operational logs. Cancelling an installation does
not roll back package-manager changes; inspect server state before retrying.
Tool installation and agent/terminal connections are mutually exclusive, so
managed packages are not replaced under an active relay process.

## Remote GitHub and GitLab authorization

Authorize on server starts the official hosting CLI there. Open the verification
page on your client and approve the code displayed by the server operation.
Credentials stay in the server CLI's OS credential store; local Git credentials
and SSH keys are never uploaded. GitHub also configures its Git credential helper.
GitLab device authorization requires a current glab and GitLab 17.9+.
Only GitHub.com and GitLab.com are supported by the current repository contract.

**Trust boundary:** gh/glab keyrings belong to the relay's OS account, not an
isolated Dunlin user. Remote authorization therefore refuses relays containing
non-admin members. After opting into host Git accounts, member access and adding
new non-admin members are disabled. Run separate relay OS accounts/services for
mutually untrusted people. This is a deliberate safety restriction, not a claim
of per-user operating-system isolation. Corrupt/unreadable account-policy files
fail closed for member access.

Linux credentials use Secret Service through `secret-tool`, including the
relay vault. When Git tools are selected, setup installs a provider, collects
the login-keyring password with the initial choices, unlocks/initializes it via
stdin, and checks it with a disposable value. The password is not saved by
Dunlin or passed in argv. A reboot still requires unlocking the keyring in the
relay service user's session. Missing/locked keyrings are errors, never an
instruction to use plaintext storage. GitHub/glab can themselves fall back to
plaintext if their keyring becomes unavailable after preflight: verify their
reported credential-storage location after authorization and before relying on
an unattended deployment.

## Copying projects

Choose Copy local project in the selected server workspace. Choose files only
or include Git history, whether to include ignored files, and optional
comma-separated excluded relative paths. Choose the folder to generate a new
preview after changing these selections. The full manifest can be copied for
review; transfer starts only after separate confirmation.
Creating a server project by copying or cloning currently requires a relay
administrator; ordinary members retain access to projects explicitly granted
by the server administrator.

This is a one-time copy, not synchronization or a move. Local files are unchanged.
Files and history may contain secrets; exclusions remove working files, not
historical commits. By default Git-ignored files are omitted. Non-Git projects
omit `.git`, `node_modules`, `target` and `.DS_Store`. Symbolic links, submodules,
special files, case-colliding names and non-UTF-8 paths are rejected rather than
silently dereferenced. Limits are 20,000 files and 1 GiB per transfer.

History uses a Git bundle containing local branches and tags, never raw `.git`.
The current branch and working-file contents are retained, including deleted
tracked files and uncommitted changes. **Staging becomes unstaged.** Remotes,
hooks, host credential helpers, ignored files unless selected, and external
worktree configuration are not copied. Detached/unborn HEAD histories require a
files-only copy, as do SHA-256-format repositories in this first implementation.
Clone from GitHub/GitLab instead when you want a configured
remote and only committed content.

Each chunk is size/offset checked, every completed file is SHA-256 checked, and
the complete project is published without replacement before registration and
granting access. Partial copies are private, unpublished staging directories.
Cancelling removes staging. Expired staging is cleaned on the next transfer;
after a process crash an administrator may need to remove orphaned
`server/projects/.transfer-*` directories. Registration failure preserves the
verified completed files and reports their path. No dependency installation,
hooks or project code run automatically after import.

Linux agent processes use bubblewrap with a dedicated home, explicitly granted
project roots, read-only tools/system libraries, and no host home/keyring mount.
Missing bubblewrap or unavailable namespaces fail at agent startup instead of
running the agent without its sandbox. Platform support still needs deployment
testing; merely detecting the bwrap executable is not a sandbox self-test.

## Verification before release

Automated coverage includes management authorization, traversal, offset/size and
checksum checks, no replacement, snapshot fidelity, symlinked source ancestors,
code lifetime/single use, future-sibling denial in the macOS server namespace,
Linux archive architecture and bootstrap contracts.
The release workflow adds native Linux x86_64/ARM64 builds and tests.

Local checks used an empty test home, dummy Git identity, offline dependencies,
network-denied sandbox and a scratch-only build target. The relay/workspace/vault
checks passed (54 relay, 164 workspace, 24 vault tests); four installer/archive
tests and release YAML/shell syntax checks passed. Core relay/workspace/vault
Clippy passed with warnings denied. Full desktop Clippy remains blocked by
pre-existing terminal/workbench/workspace-pane warnings outside this change.
Four IPC/socket tests were omitted in this sandbox, along with the new nested
macOS sandbox-application test (macOS forbids applying a second sandbox there).
The namespace policy was additionally exercised with a single combined sandbox:
the allowed project remained readable and a newly created staging sibling was
denied. Desktop tests were type-checked, not run as graphical acceptance tests.

Real Tailscale sign-in/Serve, OS keyrings, provider authorization, package
installation, reboot behavior and the graphical client require manual acceptance
on clean supported machines. They are not established by offline unit tests.

### External contracts checked (2026-11-06)

| Claim | Evidence/location | Version | Confidence | Missing link | Next check |
| --- | --- | --- | --- | --- | --- |
| OpenCode supports ACP and terminal login | [ACP guide](https://opencode.ai/v2/docs/cli/acp/), authentication/transport sections | V2 | High | Real provider approval | Clean-machine ACP/login smoke |
| GitLab supports headless device login | [glab login](https://docs.gitlab.com/cli/auth/login/), `--device` | Current docs; release API v1.128.0 | High | OS keyring on each distribution | Real glab login/storage check |
| GitLab archives contain `bin/glab` and checksums | [release config](https://gitlab.com/gitlab-org/cli/-/raw/main/.goreleaser.yml), `builds.binary`, `archives`, `checksum`; official latest-release API assets | v1.128.0 observed | High | Download/install acceptance | Native bootstrap smoke |
| Serve is private HTTPS and requires tailnet enrollment/consent | [Serve](https://tailscale.com/docs/features/tailscale-serve), prerequisites; [macOS variants](https://tailscale.com/docs/concepts/macos-variants) | Current official docs | High | Actual tailnet policy and VPN approval | Two approved clean devices |

Repository evidence is the existing `gh`/`glab` clone contract in
`crates/workspace/src/github/imports.rs::clone_with`, the relay's `AuthUser`
extractor and project grants, and the new shared `server`/`project_copy` types.
The implementation handoff is source-confirmed but deployment acceptance is
pending. The next owner is the release/operator running the native CI jobs and
the clean-machine acceptance above; no credentials or live accounts were used
to establish these contracts.
