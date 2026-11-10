# Connection map

Open **Settings → Remote · Tailscale** or **Connection map & remote environments…**
from the host menu. The map uses the same visual language as the agent graph,
but it is a host view, independent of the selected project or conversation.

- The left column contains only **you**, the person running this app.
- The right column contains the relays paired with this app.
- Lines represent only your currently observed connections to those relays.
- Other connected users appear as initials avatars on each server card. Hover
  an avatar for the user's name and connected-device count. Their other hosts,
  devices' identities, projects and private conversations are not exposed.
- Click a server card, or use Tab and Enter/Space, to manage that server.
- A current relay also shows running/queued task counts, pause and interruption
  indicators. Counts include only projects explicitly granted to you; other
  users' private workloads and descriptions are not exposed. Stale counts are
  hidden with stale presence.

The desktop sends authenticated presence heartbeats every five seconds while
running, even when this page is closed. Relays expire a device's presence after
30 seconds without a heartbeat. Multiple devices for the same relay user are
grouped into one avatar; separate users with identical names are not merged.
Pairing or ordinary API activity alone does not make a device appear online.

Joining/leaving avatars and connection lines animate, and cards move smoothly
when the host layout changes. Animation frames are requested only while the
map is visible and a transition is active. Reduced Motion uses final states.

A connection failure is **Unknown**, not evidence that the server stopped or
that every other user disconnected. Stale peer avatars are hidden. Older relays
without `POST /v1/presence` show an update-required explanation instead of fake
presence. Presence is transient and resets on relay restart; accounts, pairings
and permissions are unchanged. Device revocation is rechecked for each snapshot.

The roster is shared with authenticated users on that relay. Existing project
grants and the administrator-only host Git account restriction still apply.
This does not add isolation for mutually untrusted users of the same OS account.

The selected server's management panel provides a [Linear MCP inbox and durable
task queue](linear.md), including explicit host/profile approval, cancellation,
review checkout access and recovery controls.
