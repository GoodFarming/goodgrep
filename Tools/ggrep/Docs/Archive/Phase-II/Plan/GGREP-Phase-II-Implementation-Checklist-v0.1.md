# GGREP Phase II Implementation Checklist (v0.1)

Status: Archived (Phase II; superseded by Phase III)  
Scope: `Tools/ggrep`

This checklist is retained for historical context. Phase III (usefulness) SSOT:

- `Tools/ggrep/Docs/Spec/GGREP-Spec-Index-v0.2.md`

## References (Canonical)

- Plan: `Tools/ggrep/Docs/Archive/Phase-II/Plan/GGREP-Phase-II-Hardening-Plan-v0.1.md`
- Snapshot/index contracts: `Tools/ggrep/Docs/Spec/GGREP-Snapshot-Index-Contracts-v0.1.md`
- Query/daemon contracts: `Tools/ggrep/Docs/Spec/GGREP-Query-Daemon-Contracts-v0.1.md`
- Spec index: `Tools/ggrep/Docs/Archive/Phase-II/Spec/GGREP-Spec-Index-v0.1.md`
- Conformance map: `Tools/ggrep/Docs/Archive/Phase-II/Spec/GGREP-Conformance-Map-v0.1.md`

## Operating Rules (Checks + Balances)

- No phase starts until the prior Gate is fully checked off.
- Any new MUST in specs requires:
  - a requirement ID,
  - a Conformance Map entry,
  - a concrete test/assertion identifier.
- Defaults are contract-bound: changes require spec + code parity updates.
- Module boundaries are enforced: no new public bypasses of SnapshotView or store APIs.
- Checklist items must reference requirement IDs and plan tickets when applicable (e.g., `IDX.MUST.002`, `IPC.MUST.001`, `A1`, `G9`).
- Conformance Map MUST have no `TBD` entries, and every test/assertion identifier must exist (even as a stub).
- Gate sign-off requires evidence artifacts (test logs, schema validation output, or documented decision records).
- Any material assumption change (storage engine, platform, or deployment target) reopens Gate A.

## Gate 0 — External Preconditions + Assumption Tests (no implementation before this)

### Target environments + deployment assumptions
- [x] Confirm supported OS targets (Linux-only)
- [x] Confirm filesystem support policy (local FS only)
- [x] Confirm shared-store scope (same-host, same-group)

### Storage engine and durability assumptions
- [x] Pin LanceDB/lance versions for Phase II
- [x] Validate multi-process reader/writer behavior in a smoke test (no read corruption / blocking)
- [x] Validate durability barrier semantics (segment commit + fsync + pointer swap)
- [x] Validate lock and atomic rename behavior on target filesystems

### Embeddings + artifacts
- [x] Confirm embedding model source + license constraints (offline allowed, caching path)
- [x] Confirm network policy for downloads (online vs offline seed packages)
- [x] Confirm CPU-only build and runtime profile (GPU disabled, CUDA not required)

### Test/CI infrastructure
- [x] Confirm CI runners available (Linux-only) and which tests run per tier
- [x] Define test tier policy (PR vs nightly vs manual for stress/fuzz/loom)
- [x] Confirm failpoint/crash-injection tooling approach and feature flags

### Gate 0 checks
- [x] Assumption tests recorded (storage engine + filesystem results)
- [x] Target environment matrix documented and accepted
- [x] External docs notes recorded: `Tools/ggrep/Docs/Research/External-Docs-Notes.md`

## Gate A — Contract Freeze + Governance (no implementation before this)

### Spec governance + coherence
- [x] Finalize spec docs and treat them as SSOT for Wave II
- [x] Spec Index + Conformance Map exist (SSOT docs)
- [x] Add requirement IDs to critical MUST statements
- [x] Confirm JSON schema + golden fixture plan
- [x] Confirm module boundary enforcement (pub(crate) + dependency lint)
- [x] Confirm upgrade/downgrade behavior (compat matrix + fail-fast rules)
- [x] Run doc path sanity scan (no legacy `GoodFarmingAI/Tools/ggrep` references)

### Store + snapshot decisions
- [x] Confirm LanceDB physical strategy (per-segment tables) and table naming convention
- [x] Confirm tombstone enforcement API boundary (single blessed query entrypoint)
- [x] Confirm fail-vs-skip taxonomy for strict publish
- [x] Confirm stable file read policy (mid-write detection + bounded retries)
- [x] Confirm TOCTOU hardening for out-of-root enforcement (open-then-verify)
- [x] Confirm symlink loop detection policy (max hop depth)

### Repo + change detection decisions
- [x] Confirm repo state semantics (tracked/untracked defaults, dirty behavior)
- [x] Confirm repo SSOT config strategy (`.ggrep.toml` + `.ggignore` tracked; hashed into fingerprint)
- [x] Confirm change detection primary (filesystem-first) and reconciliation policy
- [x] Confirm reconciliation cadence (time-based default + idle-triggered runs)
- [x] Confirm fast precheck for mtime-only changes (size + head hash before full hash)
- [x] Confirm ignore semantics (`.gitignore` hierarchy; disable global ignores and `.git/info/exclude`)
- [x] Confirm ignore conformance testing strategy (golden fixtures for tricky `.gitignore` cases)

### Lease, GC, and maintenance decisions
- [x] Confirm reader model / GC safety policy (daemon-coordinated reads; offline mode; conservative GC)
- [x] Confirm offline GC requires writer lease acquisition
- [x] Confirm lease steal protocol (compare-and-swap)
- [x] Confirm lease guard for atomic updates (exclusive-create guard)
- [x] Confirm store lifecycle management strategy (store inventory + safe store GC)
- [x] Confirm store explosion mitigation (ignore_fingerprint separation or segment reuse)
- [x] Confirm store discovery is single-level scan of `~/.ggrep/data/<store_id>`
- [x] Confirm tombstone compaction policy
- [x] Confirm compaction hard segment limits (immediate compaction or fail publish)
- [x] Confirm compaction livelock avoidance (rebase/commit window)
- [x] Confirm `last_used_at` updates are coalesced (no per-query writes)
- [x] Confirm staging janitor cleanup ownership (startup/low-priority loop)
- [x] Confirm repair mapping strategy (`segment_file_index.jsonl` or full reindex fallback)

### Shared-store + platform policies
- [x] Confirm shared-store untracked policy (exclude by default; explicit opt-in)
- [x] Confirm shared-store network filesystem policy (Phase II: refuse NFS/SMB; local FS only)
- [x] Confirm shared-store capability checks (exclusive-create/rename/read-after-write)

### Deferred (out of scope for Phase II, Linux-only)
- [x] Windows path normalization and rename semantics (defer)
- [x] macOS path normalization and NFD/NFC behavior (defer)

### Daemon/IPC + output policies
- [x] Confirm CLI <-> daemon protocol handshake + compatibility policy (protocol_version + supported schema versions)
- [x] Confirm config change debounce and stale daemon auto-restart behavior
- [x] Confirm query error contract (busy/timeout JSON + exit codes)
- [x] Confirm folder scoping behavior (CLI defaults scope to cwd; optional `path` restricts results)
- [x] Confirm explainability contract (`--explain` + JSON meta)
- [x] Confirm per-query resource caps (candidate/snippet caps) and interruptible cancellation requirements
- [x] Confirm anti-starvation mechanism (reserved sync permits)
- [x] Confirm IPC framing + payload limits + socket permissions
- [x] Confirm IPC socket path length mitigation (short socket paths)
- [x] Confirm output sanitization + deterministic warnings/limits ordering

### Defaults + budgets + test harness plan
- [x] Define `status --json` and `health --json` schemas (schema_version=1) and fields
- [x] Define segment/tombstone growth thresholds and compaction overdue policy (surfaced via health)
- [x] Define retention/GC defaults and safety rules
- [x] Freeze numeric defaults (GC safety margin, compaction thresholds, IPC caps, backpressure, handle budgets)
- [x] Confirm host-wide embed limiter + stale lock recovery policy
- [x] Confirm disk budgets (store/log/cache) and refusal behavior on exceed
- [x] Confirm performance/operability budgets (latency, GC time, publish time)
- [x] Confirm per-client fairness policy (client_id + caps)
- [x] Confirm crash-point injection test matrix and concurrency model checking approach (loom or equivalent)
- [x] Confirm maintenance stress harness and filesystem failure matrix coverage

### Gate A checks
- [x] Conformance Map has no `TBD` and all referenced tests/asserts exist
- [x] JSON schemas validate all doc examples
- [x] Compatibility policy is written (N-1 read, future schema fail-fast)

## Phase 1 — Snapshot Core (Gate B: Atomic Publish + Lock-Free Reads)

- [x] G1 - Snapshot manager (staging -> manifest -> pointer swap)
- [x] B1 - Lease-based single-writer lock + heartbeat
- [x] G13 - Staging janitor (reap stale staging txns on lease acquire/startup)
- [x] B2 - Lock-free readers (pin snapshot per request)
- [x] G2 - Integrity checks at publish
- [x] G6 - Durable publish (fsync semantics)

### Gate B checks
- [x] Crash test: kill mid-staging -> active snapshot unchanged
- [x] Concurrency test: queries during publish -> consistent old/new snapshot only
- [x] Lease steal test: stale writer cannot publish after losing lease
- [x] Lease epoch preflight enforced (IDX.MUST.002)

## Phase 2 — Sync + Change Detection (Gate C: Deterministic ChangeSets)

- [x] A1 - ChangeDetector interface + filesystem-first backend
- [x] A2 - FS fallback change detection (non-git)
- [x] A3 - SyncEngine accepts explicit ChangeSet (no full walk when provided)
- [x] B3 - Watcher queue + debounce + reconciliation
- [x] A4 - Delete + rename semantics
- [x] A5 - Deterministic chunk IDs
- [x] A6 - Ignore parity enforcement across discovery/watcher/manual
- [x] A7 - Path normalization + case collision detection
- [x] A8 - Out-of-root enforcement + symlink policy
- [x] A9 - Repo SSOT config + fingerprint inputs
- [x] A10 - Ignore conformance (golden) tests
- [x] B6 - Concurrent-safe artifact downloads (models + grammars)
- [x] I1 - Repo config validation + safety caps

### Gate C checks
- [x] Rename/delete + strict publish behavior tests
- [x] Ignore parity fixtures pass across discovery/watcher/manual
- [x] Stable read + TOCTOU tests (including symlink loop coverage)

## Phase 3 — Daemon + Query Plane (Gate D: Protocol + QoS)

- [x] B7 - CLI <-> daemon handshake + version negotiation
- [x] B8 - IPC framing + socket permissions
- [x] B9 - Shared-store capability checks + filesystem probe (rename/O_EXCL/read-after-write)
- [x] B10 - IPC socket path length mitigation
- [x] B11 - Offline reader locks
- [x] H1 - Query concurrency limiter + bounded queue
- [x] H2 - Per-query timeout + cancellation propagation
- [x] H11 - Signal handling (SIGINT/SIGTERM) + graceful shutdown
- [x] H3 - Sync anti-starvation under heavy query load
- [x] H4 - Query observability (status counters + per-stage timings)
- [x] H5 - Explainability (`--explain` + JSON meta)
- [x] H6 - Query caps + interruptible cancellation
- [x] H7 - IPC robustness + fuzzing
- [x] H8 - Output sanitization
- [x] H9 - Open handle budgets
- [x] H10 - Per-client fairness
- [x] E1 - SnapshotView caching
- [x] Add `ggrep status --json` and `ggrep health --json` (schemas from spec)

### Gate D checks
- [x] Handshake mismatch produces stable invalid_request errors
- [x] Protocol negotiation selects highest common version (IPC.MUST.001)
- [x] IPC fuzzing: oversized payloads rejected without daemon crash
- [x] Deterministic ordering tests (ties + warnings/limits ordering)
- [x] Load test: sustained concurrent queries while sync publishes

## Phase 4 — Maintenance + Recovery (Gate E: Safe GC/Compaction)

- [x] D1 - Health gates for growth + store lifecycle
- [x] D2 - Repair mapping + corruption recovery
- [x] D3 - Store explosion mitigation
- [x] D4 - Audit counts vs segments
- [x] D5 - JSON schema parity + golden fixtures
- [x] D6 - Performance + operability budgets
- [x] E2 - Background compaction publish lease
- [x] E3 - Host-wide embed limiter + disk budgets
- [x] E4 - Compaction hard limits
- [x] Index compaction thresholds + compaction command
- [x] Add `ggrep upgrade-store` placeholder (even if it only reports "reindex required")

### Gate E checks
- [x] GC safety test with pinned snapshots
- [x] Compaction correctness test (before/after query equivalence)
- [x] Crash-point injection for publish/GC/compaction boundaries

## Phase 5 — Quality Gates + Stress Harness (Gate F: Release Readiness)

- [x] G3 - Failure handling + retry policy
- [x] G4 - Optional degraded snapshots (`--allow-degraded`)
- [x] G5 - Property-based sync fuzz tests (fixed seed in CI; longer soak optional)
- [x] G7 - Crash-point injection publish tests
- [x] G8 - Concurrency model checking (loom)
- [x] G9 - Forward-compat schema tests
- [x] G10 - Spec index + conformance map enforcement (no TBD)
- [x] G11 - Maintenance stress harness
- [x] G12 - Filesystem failure matrix
- [x] Eval suite baseline + regression gates
- [x] Upgrade/downgrade compatibility fixtures (newer schema -> fail-fast)

### Gate F checks
- [x] CI green on all conformance tests + schemas + fuzz
- [x] Perf smoke checks meet budgets (latency/publish/GC)
- [ ] Soak test: daemon runs >1h under churn without leaking (nightly)

## Documentation / Operator UX

- [x] Update `Tools/ggrep/GOODFARMINGAI.md` with Wave II behavior notes once implemented
- [x] Add runbook entry for `status/health/audit/repair/gc`
