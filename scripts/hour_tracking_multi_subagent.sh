#!/usr/bin/env bash
# Multi-subagent hour-tracking verification against an isolated local instance.
# Privacy: allowlisted metadata only. API key via STRUGGLETIME_TEST_API_KEY or
# minted from the local test server (never written into the repo / never logged fully).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${STRUGGLETIME_BIN:-$ROOT/rust/target/release/struggletime}"
SERVER_BIN="${STRUGGLETIME_SERVER_BIN:-$ROOT/rust/target/release/struggletime-server}"
WORKERS="${HARNESS_WORKERS:-6}"
REAL_HOME="${HOME:-/root}"
export RUSTUP_HOME="${RUSTUP_HOME:-$REAL_HOME/.rustup}"
export CARGO_HOME="${CARGO_HOME:-$REAL_HOME/.cargo}"

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/st-hourtrack.XXXXXX")"
cleanup() {
  if [[ -n "${SERVER_PID:-}" ]] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

export STRUGGLETIME_HOME="$WORKDIR/aitime"
export STRUGGLETIME_CONFIG="$WORKDIR/config"
export HOME="$WORKDIR/home"
mkdir -p "$STRUGGLETIME_HOME" "$STRUGGLETIME_CONFIG" "$HOME" "$WORKDIR/db" \
  "$WORKDIR/repo/src" "$WORKDIR/workers"

echo "== hour-tracking workdir: $WORKDIR"

if [[ ! -x "$BIN" ]]; then
  (cd "$ROOT/rust" && cargo build --release -p struggletime)
fi
if [[ ! -x "$SERVER_BIN" ]]; then
  (cd "$ROOT/rust" && cargo build --release -p st-server)
fi

PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
DB="$WORKDIR/db/server.db"
"$SERVER_BIN" --bind 127.0.0.1 --port "$PORT" --db "$DB" \
  --static-dir "$ROOT/server/static" >"$WORKDIR/server.log" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 80); do
  curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && break
  kill -0 "$SERVER_PID" 2>/dev/null || { cat "$WORKDIR/server.log"; exit 1; }
  sleep 0.1
done

SERVER="http://127.0.0.1:$PORT"
if [[ -n "${STRUGGLETIME_TEST_API_KEY:-}" ]]; then
  TOKEN="$STRUGGLETIME_TEST_API_KEY"
  case "$TOKEN" in
    st_*) ;;
    *) echo "STRUGGLETIME_TEST_API_KEY must look like st_…"; exit 1 ;;
  esac
else
  KEY_JSON="$(curl -sf -X POST "$SERVER/v1/keys" \
    -H 'Content-Type: application/json' \
    -d '{"handle":"hourtrack-bot"}')"
  TOKEN="$(printf '%s' "$KEY_JSON" | python3 -c 'import sys,json; print(json.load(sys.stdin)["token"])')"
fi
echo "== server $SERVER key_prefix=${TOKEN:0:8}… workers=$WORKERS"

printf 'fn seed() {}\n' >"$WORKDIR/repo/src/lib.rs"

assert_no_leaks() {
  local file="$1"
  if grep -Eiq 'prompt_text|/Users/|/home/[^/]+/|rm -rf|transcript|argv|BEGIN RSA' "$file"; then
    echo "PRIVACY LEAK in $file"
    exit 1
  fi
}

# Concurrent subagent simulators emit overlapping heartbeats / edits / execs.
# They must not race the ledger sealer — only event emission is parallel.
emit_worker() {
  local id="$1"
  local out="$WORKDIR/workers/w${id}.jsonl"
  {
    for burst in 1 2 3; do
      for n in $(seq 1 8); do
        printf '{"event_type":"tool","category":"ai_cli","project_id":"hour-proj","worker":%s,"burst":%s,"n":%s}\n' \
          "$id" "$burst" "$n"
        printf '{"event_type":"edit","project_id":"hour-proj","editor":"cursor","files_touched_count":1,"worker":%s,"burst":%s,"n":%s}\n' \
          "$id" "$burst" "$n"
      done
      for _ in $(seq 1 5); do
        printf '{"event_type":"tool","category":"ai_cli","project_id":"hour-proj","worker":%s,"burst":%s,"n":1}\n' \
          "$id" "$burst"
      done
    done
    printf '{"event_type":"exec","exit_code":0,"project_id":"hour-proj","worker":%s}\n' "$id"
  } >"$out"
  printf 'fn w%s() {}\n' "$id" >"$WORKDIR/repo/src/w${id}.rs"
  # Idle gap vs burst: staggered finish times.
  sleep "0.0$((id % 5))"
  echo "emitted $id" >"$WORKDIR/workers/w${id}.done"
}

PIDS=()
for i in $(seq 1 "$WORKERS"); do
  emit_worker "$i" &
  PIDS+=("$!")
done
for p in "${PIDS[@]}"; do
  wait "$p"
done

: >"$STRUGGLETIME_HOME/agent_events.jsonl"
for i in $(seq 1 "$WORKERS"); do
  cat "$WORKDIR/workers/w${i}.jsonl" >>"$STRUGGLETIME_HOME/agent_events.jsonl"
done

# Sequential seals over overlapping activity windows (multiple buckets).
for round in 1 2 3; do
  printf 'fn round%s() {}\n' "$round" >>"$WORKDIR/repo/src/lib.rs"
  "$BIN" daemon --once --bucket-secs 30 \
    --repo "$WORKDIR/repo" --project hour-proj \
    --server "$SERVER" --token "$TOKEN" \
    --user hourtrack-bot >>"$WORKDIR/daemon.log" 2>&1 || true
  sleep 0.2
done

LEDGER="$WORKDIR/repo/.aitime-ledger.json"
test -f "$LEDGER" || { echo "missing ledger"; ls -la "$WORKDIR/repo"; exit 1; }
assert_no_leaks "$LEDGER"
assert_no_leaks "$WORKDIR/server.log"
assert_no_leaks "$WORKDIR/daemon.log"

"$BIN" verify --ledger "$LEDGER" >>"$WORKDIR/verify.log" 2>&1 \
  || { echo "verify failed"; cat "$WORKDIR/verify.log"; exit 1; }

# Configure isolated HOME so status/apikey never touch the real user profile.
"$BIN" apikey "$TOKEN" >/dev/null
"$BIN" server "$SERVER" >/dev/null
STATUS_OUT="$("$BIN" status 2>&1 || true)"
printf '%s\n' "$STATUS_OUT" | head -20

OVERVIEW="$(curl -sf -H "Authorization: Bearer $TOKEN" "$SERVER/v1/me/overview")"
echo "== overview: $OVERVIEW"

python3 - "$LEDGER" "$OVERVIEW" "$WORKERS" <<'PY'
import json, sys
ledger = json.load(open(sys.argv[1]))
overview = json.loads(sys.argv[2])
workers = int(sys.argv[3])
entries = ledger.get("entries") or []
assert entries, "no sealed buckets"
assert ledger.get("salt"), "salt missing"
prev = None
active_total = 0
for e in entries:
    assert e.get("hash") and e.get("prev_hash")
    if prev is not None:
        assert e["prev_hash"] == prev, "chain broken"
    prev = e["hash"]
    for f in e.get("files") or []:
        fid = f.get("file_id", "")
        assert len(fid) == 16 and all(c in "0123456789abcdef" for c in fid), fid
        assert "path" not in f
    c = e.get("counts") or {}
    assert c.get("active_secs", 0) <= 30, c
    active_total += int(c.get("active_secs") or 0)
raw = open(sys.argv[1]).read()
for bad in ("prompt_text", "/Users/", "transcript", "argv", "rm -rf"):
    assert bad not in raw, bad
hours = float(overview.get("weighted_hours") or 0)
buckets = int(overview.get("bucket_count") or 0)
assert buckets >= 1, overview
assert hours >= 0, overview
print(
    f"hourtrack ok workers={workers} entries={len(entries)} "
    f"active_secs_sum={active_total} api_buckets={buckets} weighted_hours={hours}"
)
PY

if git -C "$ROOT" status --porcelain 2>/dev/null | grep -Eiq 'server\.db|\.aitime-ledger|hourtrack'; then
  echo "test artifacts leaked into git status"
  git -C "$ROOT" status --porcelain
  exit 1
fi

echo
echo "MULTI-SUBAGENT HOUR-TRACKING PASSED"
echo "(temp dir removed on exit)"
