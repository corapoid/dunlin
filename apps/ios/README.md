# Dunlin for iOS

A native SwiftUI client for a self-hosted Dunlin relay. It connects over
ACP-over-WebSocket, lists the projects the paired user can access, opens or
continues remote sessions, streams agent messages and answers permission
requests.

## Requirements

- Xcode 15 or newer with an iOS 17 simulator.
- `xcodegen` (`brew install xcodegen`) to generate the project from
  `project.yml`.
- A running `dunlin-relay` reachable from the simulator or device.

> Xcode is **not installed** on the machine where this scaffold was created, so
> the app has not been compiled yet. The Rust relay and the macOS client are
> fully tested; the Swift sources are ready for Xcode.

## Generate and run

```sh
brew install xcodegen
cd apps/ios
xcodegen
open Dunlin.xcodeproj
```

Choose an iPhone simulator and run. In the simulator, `http://127.0.0.1:8787`
reaches a relay running on the same Mac.

For a physical device, put the Mac and the phone on the same tailnet and use the
Tailscale HTTPS hostname:

```sh
tailscale serve --bg --https=443 http://127.0.0.1:8787
```

Then enter `https://<machine>.<tailnet>.ts.net` in the pairing screen.

## Pair

On the relay host:

```sh
dunlin-relay user add marek
dunlin-relay project add dunlin /path/to/repo --user marek
dunlin-relay pair marek --ttl 10
```

Enter the relay URL and the one-time code in the app. The relay returns a
per-device token; the app stores it in the iOS Keychain.

## What works

- Pairing, multiple servers per install, Keychain token storage.
- Project list filtered by relay grants.
- Session list with “active on another device” and **Take over**.
- ACP chat: streaming messages, thoughts, plans, tool cards, permissions,
  cancel, reconnect by reopening the session.
- No local agent: the harness runs on the relay, so sessions survive when the
  Mac is closed.

## Not in this scaffold

- File tree, editor, git and search (the macOS client has those remotely).
- Push notifications while the app is suspended.
- App Store distribution.
