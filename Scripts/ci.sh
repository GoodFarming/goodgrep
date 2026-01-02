#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="${ROOT}/Tools/ggrep/Cargo.toml"
CARGO_WRAPPER="${ROOT}/Scripts/ggrep/cargo.sh"

if [[ ! -f "${MANIFEST}" ]]; then
  echo "Missing manifest: ${MANIFEST}" >&2
  exit 2
fi

# Ensure a consistent target dir outside the repo.
"${CARGO_WRAPPER}" +nightly fmt --manifest-path "${MANIFEST}" --all -- --check
"${CARGO_WRAPPER}" +nightly check --manifest-path "${MANIFEST}" --no-default-features
"${CARGO_WRAPPER}" +nightly test --manifest-path "${MANIFEST}" --no-default-features
"${CARGO_WRAPPER}" +nightly clippy --manifest-path "${MANIFEST}" --no-default-features
