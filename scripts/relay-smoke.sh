#!/usr/bin/env bash
# End-to-end smoke test for the relay: accounts, pairing, ACP-over-WS,
# session ownership and project grants. Requires curl, python3 and a built
# mock agent.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

cargo build -p dunlin-relay
cargo build -p dunlin-agent --bins

TMP="$(mktemp -d "${TMPDIR:-/tmp}/dunlin-relay-smoke.XXXXXX")"
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

./target/debug/dunlin-relay init --db "$DB" >/dev/null
./target/debug/dunlin-relay project add dunlin "$ROOT" --user admin --db "$DB" >/dev/null
PRJ="$(./target/debug/dunlin-relay project list --db "$DB" | sed -n 's/^\(prj-[^ ]*\).*/\1/p' | head -1)"
[[ -n "$PRJ" ]] || { echo "could not read project id" >&2; exit 1; }

DUNLIN_RELAY_HARNESS="$ROOT/target/debug/mock_acp_agent" \
  ./target/debug/dunlin-relay serve --db "$DB" --listen "127.0.0.1:$PORT" >"$TMP/relay.log" 2>&1 &
RELAY_PID=$!
sleep 1

pair() {
  local user="$1" device="$2"
  local code
  code="$(./target/debug/dunlin-relay pair "$user" --ttl 10 --db "$DB" | sed -n 's/.*: \([A-Z0-9]*\)$/\1/p' | head -1)"
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

SESSION_ID="$(echo "$ADMIN_SESSIONS" | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
TRANSCRIPT_STATUS="$(curl -sS -o /dev/null -w '%{http_code}' -X PUT "http://127.0.0.1:$PORT/v1/sessions/$SESSION_ID/transcript" -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' -d '{"payload":[{"kind":"smoke"}]}')"
[[ "$TRANSCRIPT_STATUS" == "204" ]] || { echo "transcript save failed: $TRANSCRIPT_STATUS" >&2; exit 1; }
curl -sS "http://127.0.0.1:$PORT/v1/sessions/$SESSION_ID/transcript" -H "Authorization: Bearer $ADMIN_TOKEN" | grep -q '"smoke"' || { echo "transcript load failed" >&2; exit 1; }

# Provider availability is machine-specific. OpenCode limits are client/location
# scoped, so the relay must not invent an OpenCode row just to satisfy this test.
curl -fsS "http://127.0.0.1:$PORT/v1/usage" -H "Authorization: Bearer $ADMIN_TOKEN" | python3 -c '
import json, sys
rows = json.load(sys.stdin)
assert isinstance(rows, list)
assert all(isinstance(row, dict) and isinstance(row.get("provider"), str)
           and isinstance(row.get("windows"), list) and isinstance(row.get("detail"), str)
           for row in rows)
' || { echo "usage endpoint failed" >&2; exit 1; }

./target/debug/dunlin-relay user add anna --db "$DB" >/dev/null
./target/debug/dunlin-relay grant dunlin anna --db "$DB" >/dev/null
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

./target/debug/dunlin-relay user add bob --db "$DB" >/dev/null
BOB_TOKEN="$(pair bob "Bob iPhone" | json_field '["token"]')"
BOB_PROJECTS="$(curl -sS "http://127.0.0.1:$PORT/v1/projects" -H "Authorization: Bearer $BOB_TOKEN")"
[[ "$BOB_PROJECTS" == "[]" ]] || { echo "bob should not see projects: $BOB_PROJECTS" >&2; exit 1; }
BOB_STATUS="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/v1/sessions?project=$PRJ" -H "Authorization: Bearer $BOB_TOKEN")"
[[ "$BOB_STATUS" == "403" ]] || { echo "bob should get 403, got $BOB_STATUS" >&2; exit 1; }

# History mutates only this disposable fixture during setup, never ROOT.
echo "[smoke] read-only Git history over authenticated HTTP"
HISTORY_ROOT="$TMP/history"
mkdir "$HISTORY_ROOT"
git -C "$HISTORY_ROOT" init -qb main
printf 'hello from history\n' >"$HISTORY_ROOT/file.txt"
git -C "$HISTORY_ROOT" add file.txt
git -C "$HISTORY_ROOT" -c user.name=Smoke -c user.email=smoke@example.invalid -c core.hooksPath=/dev/null -c commit.gpgsign=false commit -qm fixture
printf 'saved for later\n' >"$HISTORY_ROOT/file.txt"
git -C "$HISTORY_ROOT" -c user.name=Smoke -c user.email=smoke@example.invalid -c core.hooksPath=/dev/null stash push -qm smoke-stash
git -C "$HISTORY_ROOT" -c core.hooksPath=/dev/null worktree add -q --detach "$TMP/history-linked"
HISTORY_OID="$(git -C "$HISTORY_ROOT" rev-parse HEAD)"
HISTORY_INDEX="$(cksum "$HISTORY_ROOT/.git/index")"
./target/debug/dunlin-relay project add history "$HISTORY_ROOT" --user admin --db "$DB" >/dev/null
HISTORY_PROJECT="$(curl -fsS "http://127.0.0.1:$PORT/v1/projects" -H "Authorization: Bearer $ADMIN_TOKEN" | python3 -c 'import json,sys; print(next(p["id"] for p in json.load(sys.stdin) if p["name"] == "history"))')"
HISTORY_PAGE="$(curl -fsS "http://127.0.0.1:$PORT/v1/git/history?project=$HISTORY_PROJECT&limit=1" -H "Authorization: Bearer $ADMIN_TOKEN")"
HISTORY_SCOPE="$(echo "$HISTORY_PAGE" | python3 -c 'import json,sys; p=json.load(sys.stdin); assert p["version"] == 1 and len(p["value"]) == 1 and len(p["scope"]) == 64; print(p["scope"])')"
curl -fsS "http://127.0.0.1:$PORT/v1/git/history/commit?project=$HISTORY_PROJECT&scope=$HISTORY_SCOPE&oid=$HISTORY_OID&parent=0" -H "Authorization: Bearer $ADMIN_TOKEN" | python3 -c 'import json,sys; p=json.load(sys.stdin); assert p["value"]["parent"] is None and p["value"]["files"][0]["path"] == "file.txt"'
curl -fsS "http://127.0.0.1:$PORT/v1/git/history/patch?project=$HISTORY_PROJECT&scope=$HISTORY_SCOPE&oid=$HISTORY_OID&parent=0&file=0" -H "Authorization: Bearer $ADMIN_TOKEN" | python3 -c 'import json,sys; p=json.load(sys.stdin); assert "+hello from history" in p["value"]["patch"] and p["value"]["file"]["path"] == "file.txt"'
curl -fsS "http://127.0.0.1:$PORT/v1/git/overview?project=$HISTORY_PROJECT&scope=$HISTORY_SCOPE" -H "Authorization: Bearer $ADMIN_TOKEN" | python3 -c 'import json,sys; p=json.load(sys.stdin); v=p["value"]; assert p["version"] == 1 and len(v["stashes"]) == 1 and v["stashes"][0]["selector"] == "stash@{0}" and not v["operations"] and not v["conflicts"] and len(v["worktrees"]) == 2 and sum(w["current"] for w in v["worktrees"]) == 1'
HISTORY_DENIED="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/v1/git/history?project=$HISTORY_PROJECT" -H "Authorization: Bearer $BOB_TOKEN")"
[[ "$HISTORY_DENIED" == "403" ]] || { echo "history project grant failed" >&2; exit 1; }
OVERVIEW_DENIED="$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/v1/git/overview?project=$HISTORY_PROJECT" -H "Authorization: Bearer $BOB_TOKEN")"
[[ "$OVERVIEW_DENIED" == "403" ]] || { echo "overview project grant failed" >&2; exit 1; }
[[ "$(cksum "$HISTORY_ROOT/.git/index")" == "$HISTORY_INDEX" ]] || { echo "history changed the index" >&2; exit 1; }
[[ "$(git -C "$HISTORY_ROOT" rev-parse HEAD)" == "$HISTORY_OID" ]] || { echo "history changed HEAD" >&2; exit 1; }

echo "[smoke] relay, pairing, ACP and project isolation are green"
