#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GGREP_BIN="${GGREP_BIN:-$ROOT_DIR/Tools/ggrep/target/debug/ggrep}"

if [[ ! -x "$GGREP_BIN" ]]; then
  echo "Building ggrep..."
  (cd "$ROOT_DIR/Tools/ggrep" && cargo build)
fi

BASELINE="${BASELINE:-$ROOT_DIR/Datasets/ggrep/eval_baseline.json}"
OUT="${OUT:-$ROOT_DIR/Datasets/ggrep/eval_report.json}"
DROP_PASS="${DROP_PASS:-0.02}"
DROP_MRR="${DROP_MRR:-0.02}"

if [[ ! -f "$BASELINE" ]]; then
  echo "Baseline not found; generating $BASELINE"
  "$GGREP_BIN" eval --out "$BASELINE"
  exit 0
fi

"$GGREP_BIN" eval \
  --out "$OUT" \
  --baseline "$BASELINE" \
  --baseline-max-drop-pass-rate "$DROP_PASS" \
  --baseline-max-drop-mrr "$DROP_MRR"

echo "Eval report: $OUT"
