#!/usr/bin/env bash
set -euo pipefail

TARGET_DIR="${GGREP_CARGO_TARGET_DIR:-${CARGO_TARGET_DIR:-${HOME}/.cache/ggrep/target}}"
mkdir -p "${TARGET_DIR}"
export CARGO_TARGET_DIR="${TARGET_DIR}"

exec cargo "$@"
