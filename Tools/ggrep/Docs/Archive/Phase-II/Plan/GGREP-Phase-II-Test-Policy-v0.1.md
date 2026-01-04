# GGREP Phase II Test + CI Policy (v0.1)

Status: Archived (Phase II; moved under `Docs/Archive/Phase-II/Plan`; superseded by Phase III)
Scope: `Tools/ggrep`

This policy remains useful as historical context for the “correctness hardening” era, but Phase III may add new
agent/usability regression gates. Phase III SSOT:

- `Tools/ggrep/Docs/Spec/GGREP-Spec-Index-v0.2.md`

## CI runners (assumed)

- Linux x86_64 runners only (CPU-only, no GPU/CUDA required).
- Tests in Tier 0 must run without network access or pre-downloaded artifacts.
- Disk budget assumes local SSD (vector store + caches under `~/.ggrep/`).

## Test tiers

### Tier 0 (PR / fast)

Goal: fast, deterministic checks on every PR.

Commands (see `Scripts/ci.sh`):
- `cargo fmt -- --check`
- `cargo check --no-default-features`
- `cargo test --no-default-features`
- `cargo clippy --no-default-features`

Tier 0 must be offline-safe (no model/grammar downloads).

### Tier 1 (nightly / integration)

Goal: broader coverage, including integration paths.

- `cargo test` with default features
- Multiprocess + schema validation tests
- Longer-running sync/query tests (if added)

Tier 1 may allow network if explicitly enabled, but should prefer pre-seeded caches.

### Tier 2 (manual / stress)

Goal: stress, fuzz, and long-running correctness.

- Stress harness (publish/query/GC cycles)
- Filesystem failure matrix
- Long soak runs (>= 1h)
- Optional fuzzing and model-checking (loom)
- Scripts:
  - `Scripts/ggrep/maintenance_stress.sh`
  - `Scripts/ggrep/filesystem_failure_matrix.sh`
  - `Scripts/ggrep/eval_regression.sh`
  - `Scripts/ggrep/perf_smoke.sh`
  - `Scripts/ggrep/soak_test.sh`

## Failpoints + crash injection

- Tooling approach: use feature-gated failpoints via the `fail` crate.
- Feature flag: `failpoints` (default off).
- Failpoints are enabled only in dedicated test binaries / nightly runs.
- Crash-point matrix (minimum set):
  - publish: after segment write, after manifest write, before pointer swap, after pointer swap
  - lease: after acquire, after heartbeat, during steal
  - GC: before delete, after delete list generation
  - compaction: after new segment build, before publish, after publish

## Concurrency model checking (loom)

- Target: snapshot pin/publish/GC/cache races and embed limiter permit handling.
- Feature flag: `loom` (default off).
- Run cadence: nightly or manual (Tier 2); keep bounds small and deterministic.

## Maintenance stress + filesystem failure matrix

- Stress harness should include sustained publish/query/GC churn for >= 1h.
- Filesystem failure matrix should cover:
  - `EACCES`/permission errors
  - `ENOSPC` during writes
  - `EMFILE`/fd exhaustion
  - rename failures / partial writes
  - read-after-write inconsistencies (simulated)
