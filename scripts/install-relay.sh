#!/usr/bin/env bash
# Public bootstrap. All product choices precede downloads and privileged changes.
set -euo pipefail
umask 077
REPOSITORY=corapoid/dunlin
COLOR='' RESET=''
if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then COLOR=$'\033[36;1m'; RESET=$'\033[0m'; fi
say() { printf '%s%s%s\n' "$COLOR" "$*" "$RESET"; }
fail() { printf '\nInstallation stopped: %s\n' "$*" >&2; exit 1; }
ask() { printf '%s ' "$1" >/dev/tty; IFS= read -r REPLY </dev/tty || fail 'Interactive terminal required.'; }
yesno() { ask "$1 [Y/n]"; case "$REPLY" in ''|y|Y|yes|YES) return 0;; *) return 1;; esac; }
fetch() { curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 --connect-timeout 15 --max-time 300 --max-filesize 536870912 --retry 2 "$1" -o "$2"; }
checksum() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}
verify() {
  local expected
  expected=$(awk -v name="$2" '$2 == name {print $1}' "$3")
  [[ "$expected" =~ ^[a-fA-F0-9]{64}$ && "$(checksum "$1")" == "$expected" ]] || fail "Checksum verification failed: $2"
}
[[ "${1:-}" != --help ]] || { printf 'Dunlin relay interactive installer (macOS arm64; Linux x86_64/arm64). Run as your regular service user, not root.\n'; exit 0; }
[[ "$(id -u)" != 0 ]] || fail 'Run as a regular user. sudo is requested only for system packages.'
case "$(uname -s):$(uname -m)" in
  Darwin:arm64) OS=macos; ARCH=arm64; NODE_OS=darwin; NODE_ARCH=arm64; DATA="$HOME/Library/Application Support/Dunlin" ;;
  Linux:x86_64) OS=linux; ARCH=x86_64; NODE_OS=linux; NODE_ARCH=x64; DATA="${XDG_DATA_HOME:-$HOME/.local/share}/dunlin" ;;
  Linux:aarch64|Linux:arm64) OS=linux; ARCH=arm64; NODE_OS=linux; NODE_ARCH=arm64; DATA="${XDG_DATA_HOME:-$HOME/.local/share}/dunlin" ;;
  *) fail 'Supported: macOS Apple Silicon and Linux x86_64/ARM64.' ;;
esac
command -v curl >/dev/null || fail 'Install curl first.'
say '     D U N L I N'
printf '  Your workspace. Anywhere.\n\n'
yesno 'Install or update the Dunlin server?' || exit 0
ask 'Server name [dunlin-server]:'; NAME=${REPLY:-dunlin-server}
[[ "$NAME" =~ ^[a-zA-Z0-9][a-zA-Z0-9-]{0,62}$ ]] || fail 'Use a name of 1–63 letters, digits and hyphens.'
TAILSCALE=no; yesno 'Install/update Tailscale and connect to your private network?' && TAILSCALE=yes
AUTOSTART=no; yesno 'Start the relay automatically as your user?' && AUTOSTART=yes
GIT_TOOLS=no; yesno 'Install GitHub and GitLab command-line tools for remote authorization?' && GIT_TOOLS=yes
ask 'Harnesses: 1 OpenCode, 2 Codex, 3 Claude Code (comma separated; Enter for none):'
HARNESSES=()
case "$REPLY" in *[!123,\ ]*) fail 'Choose only 1, 2, 3, separated by commas.';; esac
[[ "$REPLY" != *1* ]] || HARNESSES+=(opencode)
[[ "$REPLY" != *2* ]] || HARNESSES+=(codex)
[[ "$REPLY" != *3* ]] || HARNESSES+=(claude)
KEYRING_PASSWORD=''
if [[ "$OS" == linux && "$GIT_TOOLS" == yes ]]; then
  printf 'Linux Git authorization needs an encrypted Secret Service login keyring.\n'
  printf 'Enter its existing password, or a password for a new login keyring. It is not saved by Dunlin.\n'
  printf 'Keyring password: ' >/dev/tty
  IFS= read -r -s KEYRING_PASSWORD </dev/tty || fail 'Could not read keyring password.'
  printf '\n' >/dev/tty
  [[ -n "$KEYRING_PASSWORD" ]] || fail 'An empty keyring password is not permitted.'
fi
printf '\nServer: %s\nPlatform: %s/%s\nTailscale: %s\nAutostart: %s\nGit tools: %s\nHarnesses: %s\n' "$NAME" "$OS" "$ARCH" "$TAILSCALE" "$AUTOSTART" "$GIT_TOOLS" "${HARNESSES[*]:-none}"
printf 'Browser sign-in, VPN approval and sudo may still require your confirmation.\n'
if [[ -e "$DATA/relay.db" ]]; then
  printf 'Upgrade notice: dedicated agent homes require fresh provider sign-in. Existing host history/credentials are retained, but old live sessions may need a new conversation.\n'
fi
yesno 'Proceed with these changes?' || exit 0

TMP=$(mktemp -d "${TMPDIR:-/tmp}/dunlin-install.XXXXXXXX")
trap 'rm -rf -- "$TMP"' EXIT
trap 'exit 130' INT TERM
say '1/6 · Downloading a verified relay release'
if [[ -n "${DUNLIN_VERSION:-}" ]]; then TAG="v${DUNLIN_VERSION#v}"; else
  fetch "https://api.github.com/repos/$REPOSITORY/releases?per_page=1" "$TMP/release.json"
  TAG=$(sed -nE 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/p' "$TMP/release.json" | head -1)
fi
[[ "$TAG" =~ ^v[0-9][a-zA-Z0-9.+-]*$ ]] || fail 'Could not resolve a published release.'
VERSION=${TAG#v}
ASSET="dunlin-relay-$VERSION-$OS-$ARCH.tar.gz"
BASE="https://github.com/$REPOSITORY/releases/download/$TAG"
if [[ -n "${DUNLIN_RELEASE_BASE_URL:-}" ]]; then
  [[ "$DUNLIN_RELEASE_BASE_URL" == https://* && "$DUNLIN_RELEASE_BASE_URL" != *[@?#[:space:]]* ]] || fail 'Release mirror must be an HTTPS URL without credentials, query or fragment.'
  BASE="${DUNLIN_RELEASE_BASE_URL%/}"
fi
fetch "$BASE/$ASSET" "$TMP/$ASSET"
fetch "$BASE/RELAY-SHA256SUMS" "$TMP/SHA256SUMS"
verify "$TMP/$ASSET" "$ASSET" "$TMP/SHA256SUMS"
[[ "$(tar -tzf "$TMP/$ASSET")" == dunlin-relay ]] || fail 'Unexpected relay archive contents.'
[[ "$(tar -tvzf "$TMP/$ASSET" | cut -c1)" == '-' ]] || fail 'Relay archive is not a regular executable.'
tar -xzf "$TMP/$ASSET" -C "$TMP" dunlin-relay
chmod 700 "$TMP/dunlin-relay"
"$TMP/dunlin-relay" --help >/dev/null || fail 'The verified relay cannot run on this OS/CPU. No system packages have been changed.'
mkdir -p "$HOME/.local/bin" "$DATA"
install -m 700 "$TMP/dunlin-relay" "$HOME/.local/bin/dunlin-relay.new"
mv "$HOME/.local/bin/dunlin-relay.new" "$HOME/.local/bin/dunlin-relay"
RELAY="$HOME/.local/bin/dunlin-relay"
export PATH="$DATA/server/tools/bin:$DATA/server/node/bin:$HOME/.local/bin:$PATH"

say '2/6 · Preparing platform services'
if [[ "$OS" == linux ]]; then
  if command -v apt-get >/dev/null; then
    sudo apt-get update
    sudo apt-get install -y git bubblewrap libsecret-tools dbus-user-session gnome-keyring libstdc++6
  elif command -v dnf >/dev/null; then
    sudo dnf install -y git bubblewrap libsecret gnome-keyring libstdc++
  else fail 'Automatic Linux setup currently requires apt-get or dnf.'; fi
elif ! git --version >/dev/null 2>&1; then
  /usr/bin/xcode-select --install || true
  printf 'Approve Apple Command Line Tools installation to provide Git. Waiting up to 30 minutes…\n'
  for attempt in $(seq 1 360); do
    if /usr/bin/xcode-select -p >/dev/null 2>&1; then break; fi
    sleep 5
  done
  git --version >/dev/null 2>&1 || fail 'Finish installing Apple Command Line Tools, then rerun setup.'
fi
if [[ "$GIT_TOOLS" == yes ]]; then
  # Official stable binaries avoid stale distribution CLIs without device auth.
  GO_ARCH=arm64; [[ "$ARCH" != x86_64 ]] || GO_ARCH=amd64
  GH_OS=linux; [[ "$OS" != macos ]] || GH_OS=macOS
  GLAB_OS=linux; [[ "$OS" != macos ]] || GLAB_OS=darwin
  fetch https://api.github.com/repos/cli/cli/releases/latest "$TMP/gh-release.json"
  GH_TAG=$(sed -nE 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/p' "$TMP/gh-release.json" | head -1)
  [[ "$GH_TAG" =~ ^v[0-9.]+$ ]] || fail 'Could not resolve stable GitHub CLI.'
  GH_ROOT="gh_${GH_TAG#v}_${GH_OS}_${GO_ARCH}"
  GH_EXT=tar.gz; [[ "$OS" != macos ]] || GH_EXT=zip
  GH_ASSET="$GH_ROOT.$GH_EXT"
  GH_BASE="https://github.com/cli/cli/releases/download/$GH_TAG"
  fetch "$GH_BASE/$GH_ASSET" "$TMP/$GH_ASSET"
  fetch "$GH_BASE/gh_${GH_TAG#v}_checksums.txt" "$TMP/gh-checksums"
  verify "$TMP/$GH_ASSET" "$GH_ASSET" "$TMP/gh-checksums"
  if [[ "$OS" == macos ]]; then unzip -p "$TMP/$GH_ASSET" "$GH_ROOT/bin/gh" >"$TMP/gh";
  else tar -xzOf "$TMP/$GH_ASSET" "$GH_ROOT/bin/gh" >"$TMP/gh"; fi
  fetch https://gitlab.com/api/v4/projects/gitlab-org%2Fcli/releases/permalink/latest "$TMP/glab-release.json"
  GLAB_TAG=$(sed -nE 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/p' "$TMP/glab-release.json" | head -1)
  [[ "$GLAB_TAG" =~ ^v[0-9.]+$ ]] || fail 'Could not resolve stable GitLab CLI.'
  GLAB_ASSET="glab_${GLAB_TAG#v}_${GLAB_OS}_${GO_ARCH}.tar.gz"
  GLAB_BASE="https://gitlab.com/gitlab-org/cli/-/releases/$GLAB_TAG/downloads"
  fetch "$GLAB_BASE/$GLAB_ASSET" "$TMP/$GLAB_ASSET"
  fetch "$GLAB_BASE/checksums.txt" "$TMP/glab-checksums"
  verify "$TMP/$GLAB_ASSET" "$GLAB_ASSET" "$TMP/glab-checksums"
  tar -xzOf "$TMP/$GLAB_ASSET" bin/glab >"$TMP/glab"
  [[ -s "$TMP/gh" && -s "$TMP/glab" ]] || fail 'Empty hosting CLI archive member.'
  mkdir -p "$DATA/server/tools/bin"
  install -m 700 "$TMP/gh" "$DATA/server/tools/bin/gh"
  install -m 700 "$TMP/glab" "$DATA/server/tools/bin/glab"
fi
if [[ "$OS" == linux && "$GIT_TOOLS" == yes ]]; then
  systemctl --user start dbus.socket || fail 'A systemd user D-Bus session is required for the encrypted keyring.'
  export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
  printf '%s' "$KEYRING_PASSWORD" | timeout 20s gnome-keyring-daemon --unlock --components=secrets >/dev/null
  unset KEYRING_PASSWORD
  PROBE="dunlin-install-$$"
  printf 'probe' | timeout 15s secret-tool store --label='Dunlin setup keyring check' service dev.dunlin.setup id "$PROBE" || fail 'Could not unlock the encrypted login keyring. No Git account was authorized.'
  secret-tool clear service dev.dunlin.setup id "$PROBE"
fi

say '3/6 · Installing selected harnesses'
if [[ ${#HARNESSES[@]} -gt 0 ]]; then
  if ! command -v node >/dev/null || ! node -e 'process.exit(+process.versions.node.split(".")[0] >= 22 ? 0 : 1)'; then
    NODE_BASE=https://nodejs.org/dist/latest-v24.x
    fetch "$NODE_BASE/SHASUMS256.txt" "$TMP/NODE-SHA256SUMS"
    NODE_ASSET=$(awk -v suffix="-$NODE_OS-$NODE_ARCH.tar.gz" '$2 ~ suffix "$" {print $2}' "$TMP/NODE-SHA256SUMS")
    [[ "$NODE_ASSET" =~ ^node-v[0-9.]+-(darwin|linux)-(arm64|x64)\.tar\.gz$ ]] || fail 'Could not resolve Node.js runtime.'
    fetch "$NODE_BASE/$NODE_ASSET" "$TMP/$NODE_ASSET"
    verify "$TMP/$NODE_ASSET" "$NODE_ASSET" "$TMP/NODE-SHA256SUMS"
    mkdir -p "$DATA/server/node"
    tar -xzf "$TMP/$NODE_ASSET" -C "$DATA/server/node" --strip-components=1
    export PATH="$DATA/server/node/bin:$PATH"
  fi
  fetch "$BASE/install-harness.sh" "$TMP/install-harness.sh"
  verify "$TMP/install-harness.sh" install-harness.sh "$TMP/SHA256SUMS"
  for harness in "${HARNESSES[@]}"; do DUNLIN_TOOLS="$DATA/server/tools" bash "$TMP/install-harness.sh" "$harness"; done
fi

say '4/6 · Configuring the relay'
DB="$DATA/relay.db"
if [[ ! -e "$DB" ]]; then "$RELAY" init --db "$DB" >/dev/null; fi
"$RELAY" setup --name "$NAME" --db "$DB"
URL=http://127.0.0.1:8787
if [[ "$TAILSCALE" == yes ]]; then
  if [[ "$OS" == macos ]]; then
    if [[ -d /Applications/Tailscale.app ]]; then
      TS_ID=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' /Applications/Tailscale.app/Contents/Info.plist)
      [[ "$TS_ID" == io.tailscale.ipn.macsys ]] || fail 'A different Tailscale variant is already installed. Update it through its package manager; this installer will not replace it.'
    fi
    # The official latest stable package also upgrades the standalone variant.
      fetch https://pkgs.tailscale.com/stable/Tailscale-latest-macos.pkg "$TMP/tailscale.pkg"
      pkgutil --check-signature "$TMP/tailscale.pkg" >"$TMP/tailscale-signature"
      grep -q Tailscale "$TMP/tailscale-signature" || fail 'Unexpected Tailscale installer signer.'
      sudo installer -pkg "$TMP/tailscale.pkg" -target /
      open -a Tailscale
      printf 'Approve the Tailscale system extension/VPN if macOS asks.\n'
    TAILSCALE_BIN=/Applications/Tailscale.app/Contents/MacOS/Tailscale
  elif ! command -v tailscale >/dev/null; then
    fetch https://tailscale.com/install.sh "$TMP/tailscale-install.sh"
    sh "$TMP/tailscale-install.sh"
  fi
  if [[ "$OS" == linux ]]; then TAILSCALE_BIN=$(command -v tailscale) || fail 'Tailscale CLI unavailable.'; fi
  [[ -x "$TAILSCALE_BIN" ]] || fail 'Tailscale CLI unavailable. Finish Tailscale installation, then rerun.'
  tailscale() { "$TAILSCALE_BIN" "$@"; }
  if [[ "$OS" == linux ]] && ! sudo "$TAILSCALE_BIN" update --yes; then
    printf 'Tailscale update needs its installed app/package manager. Update it there before continuing.\n'
    fail 'Could not confirm the requested current Tailscale installation.'
  fi
  if [[ "$OS" == linux ]]; then sudo "$TAILSCALE_BIN" set --operator="$(id -un)"; fi
  if ! tailscale status --json | grep -Eq '"BackendState"[[:space:]]*:[[:space:]]*"Running"'; then tailscale up; fi
  tailscale serve status >"$TMP/serve-status"
  if grep -q 'https://' "$TMP/serve-status" && ! grep -q '127.0.0.1:8787' "$TMP/serve-status"; then
    fail 'Existing Tailscale Serve routes found. They were not changed; configure a dedicated relay route first.'
  fi
  tailscale serve --bg --https=443 http://127.0.0.1:8787 | tee "$TMP/serve-result"
  URL=$(grep -Eo 'https://[a-zA-Z0-9.-]+\.ts\.net' "$TMP/serve-result" | head -1)
  [[ -n "$URL" ]] || fail 'Tailscale did not return a private HTTPS endpoint.'
fi

say '5/6 · Starting the server'
if [[ "$AUTOSTART" == yes && "$OS" == linux ]]; then
  command -v systemctl >/dev/null || fail 'Automatic service setup requires systemd.'
  mkdir -p "$HOME/.config/systemd/user"
  # systemd quoted values need escaping even though no shell interprets ExecStart.
  ESC_RELAY=${RELAY//\\/\\\\}; ESC_RELAY=${ESC_RELAY//\"/\\\"}; ESC_RELAY=${ESC_RELAY//%/%%}
  ESC_DB=${DB//\\/\\\\}; ESC_DB=${ESC_DB//\"/\\\"}; ESC_DB=${ESC_DB//%/%%}
  ESC_PATH=${PATH//\\/\\\\}; ESC_PATH=${ESC_PATH//\"/\\\"}; ESC_PATH=${ESC_PATH//%/%%}
  if [[ -e "$HOME/.config/systemd/user/dunlin-relay.service" ]]; then
    cp -p "$HOME/.config/systemd/user/dunlin-relay.service" "$HOME/.config/systemd/user/dunlin-relay.service.previous-$(date +%s)"
  fi
  cat >"$HOME/.config/systemd/user/dunlin-relay.service" <<EOF
[Unit]
Description=Dunlin relay
After=network-online.target
[Service]
ExecStart="$ESC_RELAY" serve --db "$ESC_DB"
Environment="PATH=$ESC_PATH"
Environment=DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$(id -u)/bus
Restart=on-failure
UMask=0077
NoNewPrivileges=true
[Install]
WantedBy=default.target
EOF
  systemctl --user daemon-reload
  systemctl --user enable --now dunlin-relay.service
  systemctl --user restart dunlin-relay.service
  printf 'For startup before login: sudo loginctl enable-linger %s\n' "$(id -un)"
elif [[ "$AUTOSTART" == yes ]]; then
  mkdir -p "$HOME/Library/LaunchAgents"
  # Generate XML through the system's plist utility, not unescaped string interpolation.
  PLIST="$HOME/Library/LaunchAgents/dev.dunlin.relay.plist"
  if [[ -e "$PLIST" ]]; then cp -p "$PLIST" "$PLIST.previous-$(date +%s)"; fi
  plutil -create xml1 "$PLIST"
  plutil -insert Label -string dev.dunlin.relay "$PLIST"
  plutil -insert ProgramArguments -xml '<array/>' "$PLIST"
  plutil -insert ProgramArguments.0 -string "$RELAY" "$PLIST"
  plutil -insert ProgramArguments.1 -string serve "$PLIST"
  plutil -insert ProgramArguments.2 -string --db "$PLIST"
  plutil -insert ProgramArguments.3 -string "$DB" "$PLIST"
  plutil -insert RunAtLoad -bool YES "$PLIST"
  plutil -insert KeepAlive -bool YES "$PLIST"
  plutil -insert EnvironmentVariables -xml '<dict/>' "$PLIST"
  plutil -insert EnvironmentVariables.PATH -string "$PATH" "$PLIST"
  launchctl bootout "gui/$(id -u)/dev.dunlin.relay" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "$PLIST"
else
  nohup "$RELAY" serve --db "$DB" >"$DATA/relay.log" 2>&1 </dev/null &
  RELAY_PID=$!
  printf '%s\n' "$RELAY_PID" >"$DATA/relay.pid"
fi
READY=no
for attempt in $(seq 1 30); do
  if curl --fail --silent --max-time 1 http://127.0.0.1:8787/health >/dev/null; then READY=yes; break; fi
  sleep 1
done
[[ "$READY" == yes ]] || fail 'Relay did not become healthy. Inspect its service logs.'
sleep 1
if [[ "$AUTOSTART" != yes ]]; then
  kill -0 "$RELAY_PID" 2>/dev/null || fail 'The new relay exited; port 8787 may already belong to another process. Inspect relay.log.'
elif [[ "$OS" == linux ]]; then
  systemctl --user is-active --quiet dunlin-relay.service || fail 'The relay service failed. Inspect journalctl --user -u dunlin-relay.'
else
  launchctl print "gui/$(id -u)/dev.dunlin.relay" | grep -q 'state = running' || fail 'The relay launch agent failed. Check for an occupied port 8787.'
fi
say '6/6 · Ready to pair'
printf '\nServer: %s\nEndpoint: %s\n\n' "$NAME" "$URL"
"$RELAY" pair admin --ttl 15 --db "$DB"
printf '\nUse Settings → Remote in Dunlin. Join the same Tailscale network on the client first.\n'
printf 'New code at any time: "%s" pair admin --db "%s"\n' "$RELAY" "$DB"
if [[ "$OS" == linux ]]; then printf 'Remote Git login requires an unlocked Secret Service keyring for the service user. Plaintext fallback is not supported.\n'; fi
