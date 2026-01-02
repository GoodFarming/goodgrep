# GGREP Phase II Implementation Checklist (v0.1)

Status: Draft v0.1 (re-ordered for gating)  
Scope: `Tools/ggrep`

## References (Canonical)

- Plan: `Tools/ggrep/Docs/Plan/GGREP-Phase-II-Hardening-Plan-v0.1.md`
- Snapshot/index contracts: `Tools/ggrep/Docs/Spec/GGREP-Snapshot-Index-Contracts-v0.1.md`
- Query/daemon contracts: `Tools/ggrep/Docs/Spec/GGREP-Query-Daemon-Contracts-v0.1.md`
- Spec index: `Tools/ggrep/Docs/Spec/GGREP-Spec-Index-v0.1.md`
- Conformance map: `Tools/ggrep/Docs/Spec/GGREP-Conformance-Map-v0.1.md`

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
- [ ] Pin LanceDB/lance versions for Phase II
- [ ] Validate multi-process reader/writer behavior in a smoke test (no read corruption / blocking)
- [ ] Validate durability barrier semantics (segment commit + fsync + pointer swap)
- [ ] Validate lock and atomic rename behavior on target filesystems

### Embeddings + artifacts
- [ ] Confirm embedding model source + license constraints (offline allowed, caching path)
- [ ] Confirm network policy for downloads (online vs offline seed packages)
- [x] Confirm CPU-only build and runtime profile (GPU disabled, CUDA not required)

### Test/CI infrastructure
- [ ] Confirm CI runners available (Linux-only) and which tests run per tier
- [ ] Define test tier policy (PR vs nightly vs manual for stress/fuzz/loom)
- [ ] Confirm failpoint/crash-injection tooling approach and feature flags

### Gate 0 checks
- [ ] Assumption tests recorded (storage engine + filesystem results)
- [ ] Target environment matrix documented and accepted
- [x] External docs notes recorded: `Tools/ggrep/Docs/Research/External-Docs-Notes.md`

## Gate A — Contract Freeze + Governance (no implementation before this)

### Spec governance + coherence
- [ ] Finalize spec docs and treat them as SSOT for Wave II
- [x] Spec Index + Conformance Map exist (SSOT docs)
- [x] Add requirement IDs to critical MUST statements
- [ ] Confirm JSON schema + golden fixture plan
- [ ] Confirm module boundary enforcement (pub(crate) + dependency lint)
- [ ] Confirm upgrade/downgrade behavior (compat matrix + fail-fast rules)
- [ ] Run doc path sanity scan (no legacy `GoodFarmingAI/Tools/ggrep` references)

### Store + snapshot decisions
- [ ] Confirm LanceDB physical strategy (per-segment tables) and table naming convention
- [ ] Confirm tombstone enforcement API boundary (single blessed query entrypoint)
- [ ] Confirm fail-vs-skip taxonomy for strict publish
- [ ] Confirm stable file read policy (mid-write detection + bounded retries)
- [ ] Confirm TOCTOU hardening for out-of-root enforcement (open-then-verify)
- [ ] Confirm symlink loop detection policy (max hop depth)

### Repo + change detection decisions
- [ ] Confirm repo state semantics (tracked/untracked defaults, dirty behavior)
- [ ] Confirm repo SSOT config strategy (`.ggrep.toml` + `.ggignore` tracked; hashed into fingerprint)
- [ ] Confirm change detection primary (filesystem-first) and reconciliation policy
- [ ] Confirm reconciliation cadence (time-based default + idle-triggered runs)
- [ ] Confirm fast precheck for mtime-only changes (size + head hash before full hash)
- [ ] Confirm ignore semantics (`.gitignore` hierarchy; disable global ignores and `.git/info/exclude`)
- [ ] Confirm ignore conformance testing strategy (golden fixtures for tricky `.gitignore` cases)

### Lease, GC, and maintenance decisions
- [ ] Confirm reader model / GC safety policy (daemon-coordinated reads; offline mode; conservative GC)
- [ ] Confirm offline GC requires writer lease acquisition
- [ ] Confirm lease steal protocol (compare-and-swap)
- [ ] Confirm lease guard for atomic updates (exclusive-create guard)
- [ ] Confirm store lifecycle management strategy (store inventory + safe store GC)
- [ ] Confirm store explosion mitigation (ignore_fingerprint separation or segment reuse)
- [ ] Confirm store discovery is single-level scan of `~/.ggrep/data/<store_id>`
- [ ] Confirm tombstone compaction policy
- [ ] Confirm compaction hard segment limits (immediate compaction or fail publish)
- [ ] Confirm compaction livelock avoidance (rebase/commit window)
- [ ] Confirm `last_used_at` updates are coalesced (no per-query writes)
- [ ] Confirm staging janitor cleanup ownership (startup/low-priority loop)
- [ ] Confirm repair mapping strategy (`segment_file_index.jsonl` or full reindex fallback)

### Shared-store + platform policies
- [ ] Confirm shared-store untracked policy (exclude by default; explicit opt-in)
- [x] Confirm shared-store network filesystem policy (Phase II: refuse NFS/SMB; local FS only)
- [ ] Confirm shared-store capability checks (exclusive-create/rename/read-after-write)

### Deferred (out of scope for Phase II, Linux-only)
- [ ] Windows path normalization and rename semantics (defer)
- [ ] macOS path normalization and NFD/NFC behavior (defer)

### Daemon/IPC + output policies
- [ ] Confirm CLI <-> daemon protocol handshake + compatibility policy (protocol_version + supported schema versions)
- [ ] Confirm config change debounce and stale daemon auto-restart behavior
- [ ] Confirm query error contract (busy/timeout JSON + exit codes)
- [x] Confirm folder scoping behavior (CLI defaults scope to cwd; optional `path` restricts results)
- [ ] Confirm explainability contract (`--explain` + JSON meta)
- [ ] Confirm per-query resource caps (candidate/snippet caps) and interruptible cancellation requirements
- [ ] Confirm anti-starvation mechanism (reserved sync permits)
- [ ] Confirm IPC framing + payload limits + socket permissions
- [ ] Confirm IPC socket path length mitigation (short socket paths)
- [ ] Confirm output sanitization + deterministic warnings/limits ordering

### Defaults + budgets + test harness plan
- [ ] Define `status --json` and `health --json` schemas (schema_version=1) and fields
- [ ] Define segment/tombstone growth thresholds and compaction overdue policy (surfaced via health)
- [ ] Define retention/GC defaults and safety rules
- [ ] Freeze numeric defaults (GC safety margin, compaction thresholds, IPC caps, backpressure, handle budgets)
- [ ] Confirm host-wide embed limiter + stale lock recovery policy
- [ ] Confirm disk budgets (store/log/cache) and refusal behavior on exceed
- [ ] Confirm performance/operability budgets (latency, GC time, publish time)
- [ ] Confirm per-client fairness policy (client_id + caps)
- [ ] Confirm crash-point injection test matrix and concurrency model checking approach (loom or equivalent)
- [ ] Confirm maintenance stress harness and filesystem failure matrix coverage

### Gate A checks
- [ ] Conformance Map has no `TBD` and all referenced tests/asserts exist
- [ ] JSON schemas validate all doc examples
- [ ] Compatibility policy is written (N-1 read, future schema fail-fast)

## Phase 1 — Snapshot Core (Gate B: Atomic Publish + Lock-Free Reads)

- [ ] G1 - Snapshot manager (staging -> manifest -> pointer swap)
- [ ] B1 - Lease-based single-writer lock + heartbeat
- [ ] G13 - Staging janitor (reap stale staging txns on lease acquire/startup)
- [ ] B2 - Lock-free readers (pin snapshot per request)
- [ ] G2 - Integrity checks at publish
- [ ] G6 - Durable publish (fsync semantics)

### Gate B checks
- [ ] Crash test: kill mid-staging -> active snapshot unchanged
- [ ] Concurrency test: queries during publish -> consistent old/new snapshot only
- [ ] Lease steal test: stale writer cannot publish after losing lease
- [ ] Lease epoch preflight enforced (IDX.MUST.002)

## Phase 2 — Sync + Change Detection (Gate C: Deterministic ChangeSets)

- [ ] A1 - ChangeDetector interface + filesystem-first backend
- [ ] A2 - FS fallback change detection (non-git)
- [ ] A3 - SyncEngine accepts explicit ChangeSet (no full walk when provided)
- [ ] B3 - Watcher queue + debounce + reconciliation
- [ ] A4 - Delete + rename semantics
- [ ] A6 - Ignore parity enforcement across discovery/watcher/manual
- [ ] A7 - Path normalization + case collision detection
- [ ] A8 - Out-of-root enforcement + symlink policy
- [ ] A9 - Repo SSOT config + fingerprint inputs
- [ ] A10 - Ignore conformance (golden) tests
- [ ] B6 - Concurrent-safe artifact downloads (models + grammars)
- [ ] I1 - Repo config validation + safety caps

### Gate C checks
- [ ] Rename/delete + strict publish behavior tests
- [ ] Ignore parity fixtures pass across discovery/watcher/manual
- [ ] Stable read + TOCTOU tests (including symlink loop coverage)

## Phase 3 — Daemon + Query Plane (Gate D: Protocol + QoS)

- [ ] B7 - CLI <-> daemon handshake + version negotiation
- [ ] B8 - IPC framing + socket permissions
- [ ] B9 - Shared-store capability checks + filesystem probe (rename/O_EXCL/read-after-write)
- [ ] B10 - IPC socket path length mitigation
- [ ] B11 - Offline reader locks
- [ ] H1 - Query concurrency limiter + bounded queue
- [ ] H2 - Per-query timeout + cancellation propagation
- [ ] H11 - Signal handling (SIGINT/SIGTERM) + graceful shutdown
- [ ] H3 - Sync anti-starvation under heavy query load
- [ ] H4 - Query observability (status counters + per-stage timings)
- [ ] H5 - Explainability (`--explain` + JSON meta)
- [ ] H6 - Query caps + interruptible cancellation
- [ ] H7 - IPC robustness + fuzzing
- [ ] H8 - Output sanitization
- [ ] H9 - Open handle budgets
- [ ] H10 - Per-client fairness
- [ ] E1 - SnapshotView caching
- [ ] Add `ggrep status --json` and `ggrep health --json` (schemas from spec)

### Gate D checks
- [ ] Handshake mismatch produces stable invalid_request errors
- [ ] Protocol negotiation selects highest common version (IPC.MUST.001)
- [ ] IPC fuzzing: oversized payloads rejected without daemon crash
- [ ] Deterministic ordering tests (ties + warnings/limits ordering)
- [ ] Load test: sustained concurrent queries while sync publishes

## Phase 4 — Maintenance + Recovery (Gate E: Safe GC/Compaction)

- [ ] D1 - Health gates for growth + store lifecycle
- [ ] D2 - Repair mapping + corruption recovery
- [ ] D3 - Store explosion mitigation
- [ ] D4 - Audit counts vs segments
- [ ] D5 - JSON schema parity + golden fixtures
- [ ] D6 - Performance + operability budgets
- [ ] E2 - Background compaction publish lease
- [ ] E3 - Host-wide embed limiter + disk budgets
- [ ] E4 - Compaction hard limits
- [ ] Index compaction thresholds + compaction command
- [ ] Add `ggrep upgrade-store` placeholder (even if it only reports "reindex required")

### Gate E checks
- [ ] GC safety test with pinned snapshots
- [ ] Compaction correctness test (before/after query equivalence)
- [ ] Crash-point injection for publish/GC/compaction boundaries

## Phase 5 — Quality Gates + Stress Harness (Gate F: Release Readiness)

- [ ] G3 - Failure handling + retry policy
- [ ] G4 - Optional degraded snapshots (`--allow-degraded`)
- [ ] G5 - Property-based sync fuzz tests (fixed seed in CI; longer soak optional)
- [ ] G7 - Crash-point injection publish tests
- [ ] G8 - Concurrency model checking (loom)
- [ ] G9 - Forward-compat schema tests
- [ ] G10 - Spec index + conformance map enforcement (no TBD)
- [ ] G11 - Maintenance stress harness
- [ ] G12 - Filesystem failure matrix
- [ ] Eval suite baseline + regression gates
- [ ] Upgrade/downgrade compatibility fixtures (newer schema -> fail-fast)

### Gate F checks
- [ ] CI green on all conformance tests + schemas + fuzz
- [ ] Perf smoke checks meet budgets (latency/publish/GC)
- [ ] Soak test: daemon runs >1h under churn without leaking (nightly)

## Documentation / Operator UX

- [ ] Update `Tools/ggrep/GOODFARMINGAI.md` with Wave II behavior notes once implemented
- [ ] Add runbook entry for `status/health/audit/repair/gc`
