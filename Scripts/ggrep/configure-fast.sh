#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="${ROOT}/Tools/ggrep/config.goodfarmingai.fast.toml"
DST_DIR="${HOME}/.ggrep"
DST="${DST_DIR}/config.toml"

if [[ ! -f "${SRC}" ]]; then
  echo "Missing source config: ${SRC}" >&2
  exit 2
fi

mkdir -p "${DST_DIR}"

if [[ -f "${DST}" ]]; then
  TS="$(date -u +%Y%m%dT%H%M%SZ)"
  BACKUP="${DST}.bak.${TS}"
  cp "${DST}" "${BACKUP}"
  echo "Backed up existing config: ${BACKUP}"
fi

cp "${SRC}" "${DST}"
echo "Wrote: ${DST}"
echo
echo "Next:"
echo "  ggrep setup      # downloads models (if needed)"
echo "  ggrep doctor     # sanity-check models/paths"
echo "  cd ${ROOT} && ggrep eval --eval-store --out /tmp/ggrep-eval.json"

