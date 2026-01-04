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
  rm -rf "$WORK_DIR" "$HOME_DIR"
}
trap cleanup EXIT

export HOME="$HOME_DIR"
export GGREP_DUMMY_EMBEDDER=1

cat >"$WORK_DIR/seed.rs" <<'EOF'
fn seed() {}
// token_seed
EOF

echo "=== EACCES (read-only ggrep home) ==="
mkdir -p "$HOME_DIR/.ggrep"
chmod 400 "$HOME_DIR/.ggrep"
set +e
"$GGREP_BIN" index --path "$WORK_DIR" >/dev/null 2>&1
echo "exit=$? (expected non-zero)"
set -e
chmod 700 "$HOME_DIR/.ggrep"

echo "=== EMFILE (low ulimit) ==="
ulimit -n 32 || true
set +e
"$GGREP_BIN" search "token_seed" --path "$WORK_DIR" --json >/dev/null 2>&1
echo "exit=$? (expected busy/timeout or non-zero)"
set -e

echo "=== Read-only store directory ==="
mkdir -p "$HOME_DIR/.ggrep/data"
chmod 500 "$HOME_DIR/.ggrep/data"
set +e
"$GGREP_BIN" index --path "$WORK_DIR" >/dev/null 2>&1
echo "exit=$? (expected non-zero)"
set -e
chmod 700 "$HOME_DIR/.ggrep/data"

echo "=== Done (manual ENOSPC/rename-share tests require a constrained mount) ==="
