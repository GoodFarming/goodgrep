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

FILE_COUNT="${FILE_COUNT:-50}"
QUERY_COUNT="${QUERY_COUNT:-100}"

for i in $(seq 1 "$FILE_COUNT"); do
  cat >"$WORK_DIR/file_${i}.rs" <<EOF
// token_${i}
fn func_${i}() {}
EOF
done

echo "Starting daemon..."
"$GGREP_BIN" serve --path "$WORK_DIR" >/dev/null 2>&1 &
DAEMON_PID=$!
sleep 1

echo "Priming daemon..."
"$GGREP_BIN" search "token_1" --path "$WORK_DIR" --json >/dev/null 2>&1 || true

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
  echo "Perf smoke failed: initial snapshot not ready."
  exit 1
fi

echo "Running queries..."
for i in $(seq 1 "$QUERY_COUNT"); do
  token="token_$(( (i - 1) % FILE_COUNT + 1 ))"
  "$GGREP_BIN" search "$token" --path "$WORK_DIR" --json >/dev/null 2>&1 || true
done

echo "Running GC + compaction..."
"$GGREP_BIN" gc --path "$WORK_DIR" --force --json >/dev/null 2>&1 || true
"$GGREP_BIN" compact --path "$WORK_DIR" --force --json >/dev/null 2>&1 || true

(cd "$WORK_DIR" && "$GGREP_BIN" status --json >"$STATUS_PATH")

python3 - "$STATUS_PATH" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r") as f:
    data = json.load(f)

perf = data.get("performance", {})
errors = []

def require(name):
    value = perf.get(name)
    if value is None:
        errors.append(f"{name} missing")
    return value

def check_budget(name, value_name, budget_name):
    value = perf.get(value_name)
    budget = perf.get(budget_name)
    if value is None:
        errors.append(f"{value_name} missing")
        return
    if budget is None:
        errors.append(f"{budget_name} missing")
        return
    if value > budget:
        errors.append(f"{value_name}={value} > {budget_name}={budget}")

require("query_latency_p50_ms")
require("query_latency_p95_ms")
require("segments_touched_max")
require("publish_time_last_ms")
require("gc_time_last_ms")
require("compaction_time_last_ms")

check_budget("query p50", "query_latency_p50_ms", "query_latency_budget_p50_ms")
check_budget("query p95", "query_latency_p95_ms", "query_latency_budget_p95_ms")
check_budget("segments touched", "segments_touched_max", "segments_touched_budget")
check_budget("publish time", "publish_time_last_ms", "publish_time_budget_ms")
check_budget("gc time", "gc_time_last_ms", "gc_time_budget_ms")
check_budget("compaction time", "compaction_time_last_ms", "compaction_time_budget_ms")

if errors:
    print("Perf smoke failed:")
    for err in errors:
        print(f"  - {err}")
    sys.exit(1)

print("Perf smoke passed.")
PY
