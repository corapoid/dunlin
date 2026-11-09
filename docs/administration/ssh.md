# SSH hosts and relay bootstrap

Dunlin uses the system **OpenSSH**, including `~/.ssh/config`, known hosts,
SSH agent and ControlMaster. It does not implement another cryptographic stack
or disable `StrictHostKeyChecking`.

## Configure

1. Configure an alias in `~/.ssh/config` and verify its first host key in an SSH
   terminal. Never bypass a changed-host-key warning to make bootstrap succeed.
2. Open **Settings → Remote & SSH → SSH hosts** and import aliases.
3. **Test** establishes a ControlMaster; its control socket is under the app's
   `ssh` data directory.
4. Optionally store the passphrase in Keychain (`dev.dunlin.ssh`). The bundled
   `dunlin-ssh-askpass` helper supplies it to OpenSSH; private keys are not copied
   into Dunlin storage.

## Bootstrap and tunnel

**Bootstrap relay** requires a relay binary matching the remote OS/architecture.
It uploads and SHA-256-verifies the binary, installs a user launchd/systemd service
bound to loopback, starts a loopback tunnel and pairs the host. The result is a
normal [managed relay](relay-setup.md), not a separate SSH-only agent protocol.

**Tunnel** reopens the loopback tunnel and updates the host URL. **Remove** stops
the local master/tunnel and removes its profile; do not infer remote service/data
deletion or secure erasure of every saved credential from profile removal.
An SSH terminal in the dock is a separate transport from relay administration.

## Acceptance boundaries

Real-host MFA, changed host keys, dropped networks/reconnect, service cleanup and
Keychain removal need explicit manual testing. Fixtures and checksum verification
do not establish that those scenarios passed. See [manual acceptance](../quality/manual-acceptance.md)
and [roadmap](../project/roadmap.md).

Sources: [SSH service](../../crates/ssh/src/service.rs),
[Keychain helper](../../crates/ssh/src/keychain.rs),
[desktop host manager](../../apps/desktop/src/ssh.rs).
