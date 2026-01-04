#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GGREP_BIN="${GGREP_BIN:-$ROOT_DIR/Tools/ggrep/target/debug/ggrep}"

if [[ ! -x "$GGREP_BIN" ]]; then
  echo "Building ggrep..."
  (cd "$ROOT_DIR/Tools/ggrep" && cargo build)
fi

WORK_DIR="$(mktemp -d)"
HOME_DIR="$(mktemp -d)"

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]]; then
    kill "$DAEMON_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK_DIR" "$HOME_DIR"
}
trap cleanup EXIT

export HOME="$HOME_DIR"
export GGREP_DUMMY_EMBEDDER=1
export GGREP_TEST_QUERY_DELAY_MS=200

cat >"$WORK_DIR/seed.rs" <<'EOF'
fn seed() {}
// token_seed
EOF

echo "Starting daemon..."
"$GGREP_BIN" serve --path "$WORK_DIR" >/dev/null 2>&1 &
DAEMON_PID=$!
sleep 1

echo "Running maintenance stress loop..."
for i in $(seq 1 50); do
  echo "fn churn_$i() {} // token_$i" >>"$WORK_DIR/seed.rs"
  "$GGREP_BIN" search "token_seed" --path "$WORK_DIR" --json >/dev/null 2>&1 || true
  "$GGREP_BIN" gc --path "$WORK_DIR" --json >/dev/null 2>&1 || true
  "$GGREP_BIN" compact --path "$WORK_DIR" --json >/dev/null 2>&1 || true
done

echo "Stopping daemon..."
"$GGREP_BIN" stop --path "$WORK_DIR" >/dev/null 2>&1 || true
echo "Done."
