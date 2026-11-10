# SSH connectivity removed

Dunlin no longer connects to relay hosts through SSH, bootstraps over SSH,
creates SSH tunnels, or offers SSH terminal sessions. Remote connectivity uses
**private Tailscale Serve HTTPS only**. See [relay setup](relay-setup.md).

Install/configure the relay on the host, join both devices to the same tailnet,
enable private HTTPS Serve, and pair the client with a one-time relay code in
**Settings → Remote · Tailscale**. Replace old loopback-tunnel endpoints with
the server's `https://…ts.net` endpoint. Do not enable Funnel.

This change does not delete existing user SSH keys, Keychain items or legacy
storage records. The standalone legacy SSH crate remains in the repository but
is not used by the desktop or included as an application helper.
