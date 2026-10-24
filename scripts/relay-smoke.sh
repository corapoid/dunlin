#!/usr/bin/env bash
# End-to-end smoke test for the relay: accounts, pairing, ACP-over-WS,
# session ownership and project grants. Requires curl, python3 and a built
# mock agent.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

cargo build -p kestrel-relay
cargo build -p kestrel-agent --bins

TMP="$(mktemp -d "${TMPDIR:-/tmp}/kestrel-relay-smoke.XXXXXX")"
PORT=$((8900 + RANDOM % 90))
DB="$TMP/relay.db"
RELAY_PID=""

cleanup() {
  if [[ -n "$RELAY_PID" ]]; then
    kill "$RELAY_PID" 2>/dev/null || true
    wait "$RELAY_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

./target/debug/kestrel-relay init --db "$DB" >/dev/null
./target/debug/kestrel-relay project add kestrel "$ROOT" --user admin --db "$DB" >/dev/null
PRJ="$(./target/debug/kestrel-relay project list --db "$DB" | sed -n 's/^\(prj-[^ ]*\).*/\1/p' | head -1)"
[[ -n "$PRJ" ]] || { echo "could not read project id" >&2; exit 1; }

KESTREL_RELAY_HARNESS="$ROOT/target/debug/mock_acp_agent" \
  ./target/debug/kestrel-relay serve --db "$DB" --listen "127.0.0.1:$PORT" >"$TMP/relay.log" 2>&1 &
RELAY_PID=$!
sleep 1

pair() {
  local user="$1" device="$2"
  local code
  code="$(./target/debug/kestrel-relay pair "$user" --ttl 10 --db "$DB" | sed -n 's/.*: \([A-Z0-9]*\)$/\1/p' | head -1)"
  curl -sS -X POST "http://127.0.0.1:$PORT/v1/pair" \
    -H 'Content-Type: application/json' \
    -d "{\"code\":\"$code\",\"device_name\":\"$device\"}"
}

json_field() {
  python3 -c "import json,sys; print(json.load(sys.stdin)$1)"
}

ADMIN_PAIR="$(pair admin "Smoke Mac")"
ADMIN_TOKEN="$(echo "$ADMIN_PAIR" | json_field '["token"]')"

echo "[smoke] admin creates a remote session"
"$ROOT/target/debug/remote_smoke" "http://127.0.0.1:$PORT" "$ADMIN_TOKEN" "$PRJ" "$ROOT" "Relay smoke session" >"$TMP/admin.log" 2>&1
grep -q "PromptFinished" "$TMP/admin.log" || { cat "$TMP/admin.log"; exit 1; }
grep -q "Relay smoke session" "$TMP/admin.log" || true

ADMIN_SESSIONS="$(curl -sS "http://127.0.0.1:$PORT/v1/sessions?project=$PRJ" -H "Authorization: Bearer $ADMIN_TOKEN")"
echo "$ADMIN_SESSIONS" | grep -q "Relay smoke session" || { echo "missing admin session"; exit 1; }

./target/debug/kestrel-relay user add anna --db "$DB" >/dev/null
./target/debug/kestrel-relay grant kestrel anna --db "$DB" >/dev/null
ANNA_TOKEN="$(pair anna "Anna iPhone" | json_field '["token"]')"

echo "[smoke] anna creates an isolated session"
"$ROOT/target/debug/remote_smoke" "http://127.0.0.1:$PORT" "$ANNA_TOKEN" "$PRJ" "$ROOT" "Anna private" >"$TMP/anna.log" 2>&1
grep -q "PromptFinished" "$TMP/anna.log" || { cat "$TMP/anna.log"; exit 1; }

ADMIN_COUNT="$(curl -sS "http://127.0.0.1:$PORT/v1/sessions?project=$PRJ" -H "Authorization: Bearer $ADMIN_TOKEN" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
ANNA_COUNT="$(curl -sS "http://127.0.0.1:$PORT/v1/sessions?project=$PRJ" -H "Authorization: Bearer $ANNA_TOKEN" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
[[ "$ADMIN_COUNT" == "1" && "$ANNA_COUNT" == "1" ]] || {
  echo "session isolation failed: admin=$ADMIN_COUNT anna=$ANNA_COUNT" >&2
  exit 1
}

./target/debug/kestrel-relay user add bob --db "$DB" >/dev/null
BOB_TOKEN="$(pair bob "Bob iPhone" | json_field '["token"]')"
BOB_PROJECTS="$(curl -sS "http://127.0.0.1:$PORT/v1/projects" -H "Authorization: Bearer $BOB_TOKEN")"
[[ "$BOB_PROJECTS" == "[]" ]] || { echo "bob should not see projects: $BOB_PROJECTS" >&2; exit 1; }
BOB_STATUS="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/v1/sessions?project=$PRJ" -H "Authorization: Bearer $BOB_TOKEN")"
[[ "$BOB_STATUS" == "403" ]] || { echo "bob should get 403, got $BOB_STATUS" >&2; exit 1; }

echo "[smoke] relay, pairing, ACP and project isolation are green"
