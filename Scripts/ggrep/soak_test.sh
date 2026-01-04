#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-/dev/shm/ggrep-target}"
CLEAN_TARGET_DIR="${CLEAN_TARGET_DIR:-1}"
GGREP_BIN="${GGREP_BIN:-$TARGET_DIR/debug/ggrep}"

BUILT=0
if [[ ! -x "$GGREP_BIN" ]]; then
  echo "Building ggrep..."
  (cd "$ROOT_DIR/Tools/ggrep" && CARGO_TARGET_DIR="$TARGET_DIR" cargo build)
  BUILT=1
fi

WORK_DIR="$(mktemp -d)"
HOME_DIR="$(mktemp -d)"

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]]; then
    kill "$DAEMON_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK_DIR" "$HOME_DIR"
  if [[ "$BUILT" == "1" && "$CLEAN_TARGET_DIR" == "1" && "$TARGET_DIR" == /dev/shm/* ]]; then
    rm -rf "$TARGET_DIR"
  fi
}
trap cleanup EXIT

export HOME="$HOME_DIR"
export GGREP_DUMMY_EMBEDDER=1

DURATION_SEC="${DURATION_SEC:-3600}"
SLEEP_MS="${SLEEP_MS:-50}"
GC_EVERY="${GC_EVERY:-25}"
COMPACT_EVERY="${COMPACT_EVERY:-50}"

cat >"$WORK_DIR/seed.rs" <<'EOF'
fn seed() {}
// token_seed
EOF

echo "Starting daemon..."
"$GGREP_BIN" serve --path "$WORK_DIR" >/dev/null 2>&1 &
DAEMON_PID=$!
sleep 1

echo "Priming daemon..."
"$GGREP_BIN" search "token_seed" --path "$WORK_DIR" --json >/dev/null 2>&1 || true

echo "Waiting for initial snapshot..."
STATUS_PATH="$WORK_DIR/status.json"
READY=0
for _ in $(seq 1 60); do
  (cd "$WORK_DIR" && "$GGREP_BIN" status --json >"$STATUS_PATH")
  if python3 - "$STATUS_PATH" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r") as f:
    data = json.load(f)

snapshot = data.get("snapshot", {}) or {}
sync = data.get("sync", {}) or {}

active = snapshot.get("active_snapshot_id")
state = sync.get("state")

if active and (state in (None, "idle", "ready")):
    sys.exit(0)

sys.exit(1)
PY
  then
    READY=1
    break
  fi
  sleep 1
done

if [[ "$READY" != "1" ]]; then
  echo "Soak test failed: initial snapshot not ready."
  exit 1
fi

sleep_sec="$(awk "BEGIN { printf \"%.3f\", ${SLEEP_MS}/1000 }")"
start_ts="$(date +%s)"
iter=0

echo "Running soak test for ${DURATION_SEC}s..."
while true; do
  now="$(date +%s)"
  elapsed=$((now - start_ts))
  if (( elapsed >= DURATION_SEC )); then
    break
  fi

  iter=$((iter + 1))
  echo "fn churn_${iter}() {} // token_${iter}" >>"$WORK_DIR/seed.rs"
  "$GGREP_BIN" search "token_seed" --path "$WORK_DIR" --json >/dev/null 2>&1 || true

  if (( iter % GC_EVERY == 0 )); then
    "$GGREP_BIN" gc --path "$WORK_DIR" --force --json >/dev/null 2>&1 || true
  fi
  if (( iter % COMPACT_EVERY == 0 )); then
    "$GGREP_BIN" compact --path "$WORK_DIR" --force --json >/dev/null 2>&1 || true
  fi

  if (( iter % 200 == 0 )); then
    echo "  iterations=${iter} elapsed=${elapsed}s"
  fi

  sleep "$sleep_sec"
done

echo "Stopping daemon..."
"$GGREP_BIN" stop --path "$WORK_DIR" >/dev/null 2>&1 || true
echo "Done."
