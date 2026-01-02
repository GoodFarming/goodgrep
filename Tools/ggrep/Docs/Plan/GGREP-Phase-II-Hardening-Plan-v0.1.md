# GGREP Phase II Hardening Plan (Wave II)

Status: Draft v0.1  
Owner: Engine Dev (ggrep)  
Scope: Tools/ggrep in goodgrep repo

## Phase II Scope Constraints (Execution)

- Supported OS: Linux-only (Phase II does not target Windows/macOS).
- Embedding runtime: CPU-only (no CUDA/GPU requirements).
- IPC transport: Unix domain sockets (no Windows named pipes in Phase II).
- Filesystems: local only (Phase II does not support shared-store on NFS/SMB/network mounts).
- Shared-store scope: same-host, same-group (POSIX group/ACL permissions; no multi-host coordination).

Windows/macOS-specific notes may remain in docs as *future* work, but are not Phase II gating items.

## Context

Wave II focuses on making ggrep deterministic, incremental, and safe in a multi-agent repo.
Synchronization must reflect repo reality: file add/modify updates only those files, deletes are
removed, and renames behave as delete+add unless a direct mapping exists. Indexing should be
reliable under concurrent agent usage and frequent retrievals.

## Goals

- Perfect reflection: index content matches the working tree at sync time.
- Incremental updates: avoid full re-indexes; only changed files are touched.
- Multi-agent safety: no store thrash or daemon conflicts across agents.
- Stable quality: retrieval improvements are measurable and regression-protected.
- Operational clarity: clear status, logs, and recovery paths.

## Non-goals

- Changing the embedding model family in this wave.
- Adding remote/cloud indexing or any external dependency.
- Large-scale schema refactors outside ggrep internals.

## Correctness Invariants

- Every indexed chunk maps to a specific file path and file hash.
- Deleted files return no results.
- Index metadata and vector store are atomically consistent after each sync.
- Path normalization is stable and consistent across indexing and querying.
- Out-of-root safety: no indexed file may resolve (via symlink or `..`) outside the canonical repo root.
- Path uniqueness: within a snapshot, no two distinct files may map to the same normalized path key
  (including casefold collisions).
- Sync is idempotent for unchanged inputs.
- Queries see either the previous snapshot or the next snapshot, never partial state.

## North Star Contract (Snapshot Isolation)

- Writes are atomic: sync publishes a new snapshot or nothing.
- Single-writer, many-readers: one sync writer per store, unlimited concurrent readers.
- Incremental by default: change sets only; full scans are fallback/repair.
- Crash-safe: failures leave the last-good snapshot intact and recoverable.
- Deterministic IDs: same inputs produce stable chunk IDs and metadata.
- Availability under load: query backpressure/timeouts prevent sync starvation and keep the daemon responsive.

## Component Model (High-Level)

- Change Detector: filesystem-first deltas with scheduled reconciliation (git diff or full hash audit).
- Chunker: deterministic chunking per file type.
- Embedder + Cache: batched, retryable, fingerprinted embeddings.
- Storage Layer: vector store (chunks) + manifest/index state.
- Snapshot Manager: staging build, integrity check, atomic publish.
- Query Engine: snapshot-aware reads, hybrid ranking, output profiles.
- Daemon: watcher + reconciliation loop with low-impact throttles.

## Technical Parameters Checklist

- Canonical root selection (repo root vs cwd).
- Path normalization (absolute vs repo-relative).
- Ignore rules parity (discovery, watcher, manual index).
- Ignore SSOT (repo `.gitignore` hierarchy + tracked `.ggignore`; disable global/exclude to avoid drift).
- Change detection (git diff, fs events, metadata scan).
- Change detection precheck for mtime-only changes (size + head hash before full hash).
- Reconciliation cadence (time-based default; run after sync idle).
- Git command contention handling (`.git/index.lock` retries/backoff; no partial publishes).
- Git backend implementation (CLI vs lib) must preserve NUL-safe path handling and rename semantics.
- Rename detection (git diff name-status).
- Dirty tree semantics (tracked + untracked coverage).
- Branch/merge-base behavior when `last_head` is not an ancestor.
- Reader model / GC safety across processes (daemon-coordinated reads vs offline mode; unknown readers).
- Repair mapping strategy (path_key -> segment_id or full reindex fallback).
- Chunker consistency across file types, including Mermaid.
- Deterministic chunk ID strategy and versioning.
- Embedding config fingerprinting (model, prefix, max lengths).
- Repo SSOT config (tracked `.ggrep.toml`/profiles; index fingerprint excludes ignores; ignore inputs hashed into
  `ignore_fingerprint`).
- Artifact determinism (pin embedding model revisions + tree-sitter grammar versions; avoid `latest`).
- Artifact download integrity (checksum/validation) and concurrency safety (per-artifact locks + atomic writes).
- Store identity strategy (per worktree vs shared store).
- Shared-store permission model (single-user default; multi-user via explicit ACLs).
- Shared-store network filesystem policy: refuse NFS/SMB/network mounts in Phase II (local FS only).
- Shared-store capability checks (exclusive-create/rename/read-after-write).
- Shared-store filesystem capability probe (Linux): validate rename + O_EXCL + read-after-write; refuse weak mounts by default.
- IPC framing + payload limits (length-prefixed JSON, max request/response bytes).
- IPC socket/pipe permissions (0700 single-user, 0770 shared-store).
- IPC socket path length limits (short hash paths or temp socket indirection).
- Index transaction strategy (segments + atomic swap).
- Publish durability (fsync semantics for manifests/pointer swap).
- Index consistency checks (metadata vs vector store).
- Concurrency controls (locks, leases, daemon protocol).
- Config change debounce + stale daemon auto-restart behavior.
- Signal handling (SIGINT/SIGTERM) and graceful shutdown behavior (cancel sync/query; release leases promptly).
- Lock lease/heartbeat semantics (stale lock recovery).
- Lease guard for atomic lease updates (exclusive-create short-lived guard).
- Partial failure handling (batch retries, backoff).
- Stable file reads during sync (detect mid-write changes; retry bounded times).
- Low-impact mode semantics (throttling, debounce, single-thread).
- Query backpressure/timeouts (max concurrent queries, queue depth, per-query deadlines, cancellation).
- Output sanitization (control chars/ANSI; deterministic truncation).
- Deterministic warnings/limits ordering in JSON output.
- CLI <-> daemon protocol/version negotiation (handshake; schema/protocol compatibility).
- Per-query resource caps (candidate caps, snippet byte caps) and interruptible cancellation.
- Open handle budgets (per-query + daemon-global) and status/health exposure.
- Sync anti-starvation / fairness (resource isolation between query and sync).
- Ranking pipeline (dense vs lexical blend, intent routing).
- Output profiles (quotas, per-file caps, snippet lengths).
- Explainability/trace output contract (`--explain` + JSON metadata for tuning/debugging).
- Evaluation suite coverage and baselines.
- Sensitive content policy (denylist, redaction, opt-in corpora).
- Source adapter contracts (filesystem vs future sources).
- Schema/versioning and migration policy.
- Requirement IDs for MUSTs + conformance map enforcement.
- Module boundary enforcement (pub(crate) + dependency lint).
- JSON schema parity + golden fixtures for manifest/status/health/query/handshake.
- Performance + operability budgets (latency, segments touched, GC time, publish time).
- Per-client fairness (client_id + per-client caps).
- Binary/large file handling policy and size caps.
- Symlink/submodule policy.
- Symlink watcher resilience (no watcher loops on symlink cycles).
- Encoding normalization and UTF-8 handling.
- Cross-platform case normalization + collision detection policy.
- Windows extended-length path normalization for `path_key`.
- Explicit out-of-root enforcement (realpath boundary checks; traversal protection).
- Windows share-delete semantics for reading `ACTIVE_SNAPSHOT`.
- Filesystem failure matrix coverage (EACCES/EMFILE/rename/share violations).
- Maintenance stress harness (publish + pin + GC + compaction).
- Windows atomic replace semantics for lease + pointer swaps (MoveFileEx with replace + write-through).
- TOCTOU protection for out-of-root enforcement (open-then-verify).
- Symlink loop detection (max hop depth).
- Property-based sync fuzz tests (random add/modify/delete/rename sequences; crash-point injection).
- Crash-point injection tests for publish boundaries.
- Concurrency model checking (loom or equivalent) for snapshot pin/publish/GC/cache races.
- Spec index + conformance map (MUST coverage) and JSON schema validation of doc examples.
- Store lifecycle management (enumerate stores; safe store GC to prevent store explosion).
- Staging janitor ownership and TTL cleanup policy.
- Store `last_used_at` updates must be coalesced (no per-query writes).
- Store explosion mitigation (ignore-fingerprint separation or segment reuse across stores).
- Compaction livelock avoidance (rebase or short commit window).
- Offline reader GC safety (shared reader locks or exclusive GC window).
- Host-wide embed limiter (global concurrency; stale lock recovery).
- Disk budgets (store/log/cache) and health warnings.
- Compaction hard segment limit (immediate compaction when exceeded).
- Forward-compat schema tests (future schema_version rejection).

## Wave II Workstreams

### Stream A: Sync Fidelity And Index Correctness

1) Change detection
   - Maintain `index_state.json` with last seen git HEAD + scan timestamp.
   - Filesystem-first ChangeSet generation is the default; reconcile with git diff or full hash audit.
   - Define explicit reconciliation cadence (time-based default, plus idle-triggered runs).
   - Optionally include working tree deltas (dirty + untracked).
   - When git is available, use `git diff --name-status` against last HEAD for reconciliation.
   - When git is unavailable, fall back to metadata scan with full hash confirm (sampling may prefilter).
   - Use a fast precheck (size + head hash) to filter mtime-only changes before full hashing.
   - Allow sync engine to accept explicit change sets (no full walk).

2) Delete and rename handling
   - Apply deletes immediately; treat rename as delete+add by default.
   - Optionally preserve chunk ids for renames to reduce churn.

3) Ignore parity
   - Apply the same ignore rules across discovery, watcher, and manual index.
   - Add conformance tests (golden cases) for nested `.gitignore` semantics.

4) Atomicity
   - Write new segments, then atomically swap pointers for queries.
   - Use a staging table or segment set before swap.
   - Track partial failures and roll back to last stable snapshot.

5) Store consistency
   - Add an integrity check between MetaStore and vector store.
   - Decide whether MetaStore is authoritative or derived.

6) Determinism
   - Stable chunk IDs using repo-relative path + file hash + offsets + chunker version.
   - Persist config + ignore fingerprints in `index_state.json` and snapshot manifests.

7) Path safety and portability
   - Define canonical `path_key` as repo-relative with stable separator normalization.
   - Enforce out-of-root exclusion: resolved real path must remain under canonical root.
   - Define symlink behavior: index symlinks only if the target resolves inside root.
   - Add casefold collision detection for cross-platform safety; surface collisions in `ggrep health`
     and fail strict publish unless explicitly overridden.

### Stream B: Multi-Agent Safety And Lifecycle

1) Store identity
   - Default to per-worktree store id (root hash in id).
   - Optional shared store via explicit flag or config.

2) Daemon compatibility
   - Namespaced sockets per git hash or config signature.
   - Allow concurrent daemons with explicit routing rules.
   - Add CLI <-> daemon handshake/version negotiation (protocol + schema versions; fail fast on incompatibility).

3) Concurrency controls
   - Cooperative lock + lease with timeout and heartbeats.
   - Backoff and queueing for rapid file changes.
   - Detect and reclaim stale locks safely.
   - Readers are lock-free and read only the active snapshot.

4) Sync behavior
   - Default search does not sync unless explicitly requested.
   - Daemon syncs incrementally via watcher + periodic reconciliation.
   - Define low-impact mode throttles (debounce, batch size, single-thread).
   - Retry embed batches with bounded backoff on failure.
   - Batch failures do not abort processing of other batches; snapshot publish is strict by default
     (no publish if errors remain) unless `--allow-degraded` is enabled.

### Stream C: Retrieval Quality And Evaluation

1) Hybrid ranking
   - Blend lexical score with dense similarity.
   - Introduce intent routing (code vs doc vs diagram).
   - Provide explicit query profiles with configurable quotas and weights.

2) Diagram/doc recall
   - Ensure Mermaid and planning docs get dedicated quotas.
   - Evaluate anchor handling under both modes.

3) Evaluation suite
   - Expand `Datasets/ggrep/eval_cases.toml` with doc/diagram cases.
   - Define baseline metrics (MRR, recall@k) and regression gates.
   - Add regression tests for query profiles and per-kind quotas.

### Stream D: Observability And Operations

1) Sync reporting
   - Emit structured sync logs (counts, durations, error types).
   - `ggrep status --json` for daemon and index health.
   - Add `ggrep health` for drift checks between MetaStore and LanceDB.
   - Expose snapshot metadata (head SHA, dirty flag, config fingerprint).
   - Expose query metrics: in-flight queries, queue depth, timeouts, slow query counts, and
     per-stage timings (retrieve/rank/format).
   - Expose daemon protocol compatibility metadata (binary_version, protocol_version, supported schemas).

2) Recovery tools
   - `ggrep audit` to reconcile store vs filesystem.
   - `ggrep repair` to rebuild missing or stale files only.
   - Add crash recovery cleanup for stale staging transactions.
   - Add store inventory + store GC (`ggrep stores --json`, `ggrep gc --stores`) with conservative defaults.

### Stream E: Performance And Storage

1) Index compaction
   - Periodic merge of delta segments.
   - Configurable compaction thresholds.

2) Resource limits
   - Batch size and rate control for embeddings.
   - Cache reuse for unchanged chunks.
   - Embedding cache keyed by chunk hash + config fingerprint.
   - Cache SnapshotView per snapshot_id in the daemon (segment handles + tombstone filters) to avoid rebuild thrash.
   - Add health thresholds for segment/tombstone growth and compaction overdue conditions.

### Stream F: Extensibility And Policy

1) Source adapters
   - Define a contract for non-filesystem sources (future memory/CRM).
   - Isolate corpora by namespace with explicit opt-in.

2) Sensitive content controls
   - Denylist patterns for private data and datasets by default.
   - Require explicit include rules for sensitive sources.
   - Add binary/size caps and encoding safety to avoid runaway indexing.
   - Treat repo config (`.ggrep.toml`) as untrusted input: validate scope and enforce hard safety caps.

### Stream G: Reliability And Recovery

1) Snapshot manager
   - Build staging data, validate integrity, then publish via pointer swap.
   - Retain last-good snapshot and allow rollback.

2) Failure handling
   - Default is strict publish: do not publish if any changed file fails.
   - `--allow-degraded` publishes with `degraded=true` and error counts in manifest.

3) Invariant hardening via property-based tests
   - Add property-based sync fuzz tests generating random add/modify/delete/rename sequences.
   - Include editor-style patterns (temp write + rename) and optional crash-point injection.

4) Concurrency hardening
   - Add crash-point injection tests across publish boundaries (segments/tombstones/manifest/pointer swap/cleanup).
   - Add concurrency model checking (loom or equivalent) for snapshot pin + publish + GC interactions.

### Stream H: Query Resilience, Fairness, And Backpressure

1) Backpressure controls
   - Add `max_concurrent_queries` (daemon-wide semaphore) and `max_query_queue_depth`.
   - When saturated, fail fast with a clear "busy" response (optionally include retry-after hint).

2) Deadlines and cancellation
   - Add per-query deadline (`query_timeout_ms`) and cancel work once deadline expires.
   - Ensure downstream stages (lexical search, vector search, rerank, formatting) honor cancellation.

3) Sync anti-starvation
   - Reserve capacity for sync tasks (separate pools or explicit priority/quotas).
   - Ensure sync can publish snapshots even when query load saturates the daemon.

4) Query observability
   - `ggrep status --json` includes in-flight queries, queue depth, timeouts, slow-query counts.
   - Structured query logs include per-stage timing (admission, snapshot read, retrieve, rank, format).

5) Query resource caps and isolation
   - Enforce per-query candidate and snippet byte caps; surface truncation in `--explain`/JSON metadata.
   - Ensure cancellation is interruptible inside long-running retrieval loops.
   - Separate publish-critical sync execution from query execution (pools/executors) in addition to reserved permits.

### Stream I: Determinism, Artifacts, And Repo SSOT

1) Repo SSOT config
   - Add a tracked repo config file (e.g. `.ggrep.toml`) for index-critical settings + query profiles.
   - Hash repo config + repo ignore inputs into the config fingerprint to prevent agent drift.

2) Artifact determinism + integrity
   - Pin embedding model revisions and tree-sitter grammar versions (avoid `releases/latest`).
   - Require checksums (or load/validation) and atomic downloads.
   - Use per-artifact locks to avoid concurrent corruption.

3) Publish durability
   - Make publish crash-durable with fsync semantics (manifest + pointer + directories).

4) Repo hygiene signals
   - Add a `ggrep health` check for non-ignored untracked files (count/bytes/top dirs) to enforce the
     "track everything unless gitignored" repo standard.

## Decision Record (Wave II Defaults)

#### D1. Shared store behavior
- Default: opt-in only (`--shared-store <id>` or config); otherwise per-worktree store.
- Why: avoids cross-clone contamination, lock contention, and mixed-config results.
- Escape hatch: allow shared store only when config fingerprint matches exactly.
- Shared-store permissions MUST be group/ACL friendly (0770/0660 or equivalent) and MUST refuse weak network FS
  mounts that lack atomic rename + exclusive-create semantics.

#### D2. Authoritative change detector when git is unavailable
- Default: metadata scan (mtime/size) -> full hash for changed candidates -> ChangeSet.
- Why: sampling-only creates correctness ambiguity.
- Escape hatch: allow sampled hash as a prefilter, but require periodic full-hash audit.

#### D3. Hash strictness
- Default: full content hash for any file used to publish a snapshot.
- Why: chunk IDs and correctness invariants depend on it.
- Escape hatch: sampling may screen candidates but not replace full hash on publish.

#### D4. Anchor chunk merge strategy
- Default: anchors are a separate `kind=anchor` chunk type with stable IDs.
- Why: keeps determinism and allows profile-driven quotas without merging data.
- Escape hatch: if needed, implement anchor expansion at query time only.

#### D5. MetaStore authority
- Default: vector store + snapshot manifest are authoritative for chunk rows.
- Why: dual sources of truth cause drift and complicate recovery.
- Escape hatch: extra Meta indexes allowed only as derived/rebuildable caches.

#### D6. Atomic swap mechanism
- Default: manifest pointer swap via `ACTIVE_SNAPSHOT` atomic rename.
- Why: simple, testable snapshot isolation and lock-free reads.
- Escape hatch: swap internals later if the pointer contract is preserved.

#### D7. Low-impact mode semantics
- Default: debounce 2000ms, small embed batches (8-16), workers=1, longer reconciliation
  (5-10m), compaction disabled unless explicit.
- Why: a flag must change measurable behavior.
- Escape hatch: allow overrides via config.

#### D8. Snapshot model choice
- Default: delta segments + tombstones + compaction thresholds; query always applies
  tombstones.
- Why: preserves incrementality while keeping correctness; filtering must be hardwired.
- Escape hatch: optional full materialization for small repos or strict simplicity mode.

#### D9. Degraded snapshot policy
- Default: do not publish if any changed file fails to embed; keep last-good snapshot.
- Why: preserves "perfect reflection" for published snapshots.
- Escape hatch: `--allow-degraded` publishes with `degraded=true` + error counts.

#### D10. Rename handling
- Default: preserve chunk IDs only when git reports rename and file hash unchanged.
- Why: reduces churn safely; avoids misleading IDs when content changes.
- Escape hatch: content-similarity rename mapping may be added behind a flag.

#### D11. Query backpressure and timeout policy
- Default: enforce `max_concurrent_queries` + `max_query_queue_depth` + `query_timeout_ms` with
  initial defaults `max_concurrent_queries=8`, `max_query_queue_depth=32`, `query_timeout_ms=60000`.
- Why: prevents daemon overload and sync starvation under heavy multi-agent query load.
- Escape hatch: allow higher limits or disabling limits in dev environments.

#### D12. Cross-platform path normalization and out-of-root enforcement
- Default: canonical repo-relative `path_key`; enforce out-of-root exclusion using resolved real paths;
  detect casefold collisions and fail strict publish (surfaced via `ggrep health`).
- Why: prevents silent aliasing/corruption and closes symlink traversal risks.
- Escape hatch: allow explicit overrides/allowlists for rare repos that require case collisions.
- Windows: normalize extended-length prefixes (`\\?\`, `\\?\UNC\`) before hashing `path_key`.
- TOCTOU hardening: prefer open-then-verify or openat-style traversal for out-of-root enforcement.

#### D13. Fuzz testing policy for sync invariants
- Default: run a fixed-seed property-based suite in CI (bounded runtime) and a longer randomized soak
  in nightly/soak runs.
- Why: catches rare rename/delete/tombstone/publish corner cases that normal tests miss.
- Escape hatch: allow temporarily reducing fuzz depth if CI time becomes a bottleneck.

#### D14. Repo SSOT config
- Default: support tracked repo config (e.g. `.ggrep.toml`) for index-critical settings + query profiles; hash it
  into the `config_fingerprint`, and hash `.gitignore` + `.ggignore` into `ignore_fingerprint`.
- Why: prevents per-agent drift and makes multi-agent results consistent and reproducible.
- Escape hatch: allow per-user overrides, but any index-critical override MUST change the store id/fingerprint.

#### D15. Artifact version pinning
- Default: pin embedding model revisions and tree-sitter grammar versions; do not use moving `latest` URLs.
- Why: ensures deterministic embeddings/chunking across time and machines.
- Escape hatch: allow a dev-only "floating" mode that marks the fingerprint and surfaces warnings in health/status.

#### D16. Artifact download integrity + concurrency safety
- Default: downloads MUST use per-artifact locks, temporary files, validation/checksums, atomic renames, and
  directory fsync where relevant.
- Why: prevents corrupted caches and nondeterministic behavior when multiple agents start concurrently.
- Escape hatch: none (safety requirement).

#### D17. Shared-store cleanliness gating
- Default: shared-store excludes untracked files unless explicitly enabled (to prevent cross-user thrash).
- Why: untracked collisions across users can cause rapid index churn and inconsistent results.
- Escape hatch: allow `--shared-store-include-untracked` for trusted shared environments; dirty/untracked state MUST
  be surfaced loudly.

#### D18. Explainability contract
- Default: provide `--explain` (and JSON meta) including snapshot id, config fingerprint, profile/quotas/weights,
  per-stage timings, and candidate mix details.
- Why: makes quality tuning and debugging practical for multi-agent use.
- Escape hatch: allow disabling in minimal mode, but keep JSON meta available for eval/CI.

#### D19. Reader model and GC safety
- Default: daemon-coordinated reads; public query paths route through the daemon when running. GC is daemon-driven
  by default and must acquire the writer lease when invoked without a daemon.
- Why: prevents GC from breaking unknown readers and keeps snapshot pinning consistent.
- Escape hatch: explicit offline/direct-store mode and `--force` GC for operator workflows.
- Offline reads are best-effort under concurrent GC; paused processes beyond the safety margin may fail.

#### D20. CLI <-> daemon protocol negotiation
- Default: add a handshake that exposes `binary_version`, `protocol_version`, and supported schema versions; fail fast
  on incompatibility.
- Why: multi-agent environments will have mixed binary versions; this prevents silent breakage.
- Escape hatch: none (compatibility requirement).

#### D21. Ignore conformance tests
- Default: maintain golden tests validating nested `.gitignore` semantics (negation, precedence, anchors).
- Why: ignore drift is a primary source of "agent A indexed different files than agent B".
- Escape hatch: none (correctness requirement).

#### D22. Query resource caps
- Default: enforce per-query candidate and response-size caps and ensure cancellation is interruptible; surface cap hits
  in `--explain` and JSON metadata.
- Why: permits alone don't prevent a single query from exhausting resources under load.
- Escape hatch: allow higher limits in dev configs, but keep safe defaults.

#### D23. Segment/tombstone growth health gates
- Default: health reports segment/tombstone growth and compaction overdue status; daemon caches SnapshotView per snapshot.
- Why: prevents month-3 performance cliffs and makes compaction needs visible.
- Escape hatch: thresholds configurable via repo SSOT config.

#### D24. Repo config security posture
- Default: treat `.ggrep.toml` as untrusted; validate that config cannot expand scope outside root and cannot disable
  safety caps (size/chunk/bytes per sync limits).
- Why: tracked config is also an attack surface in multi-agent and multi-repo contexts.
- Escape hatch: operator-only overrides gated by explicit flags.

#### D25. Crash-point + concurrency testing
- Default: add crash-point injection tests at publish boundaries and concurrency model checking where feasible.
- Why: fuzz tests catch many issues, but publish/GC races often require targeted testing.
- Escape hatch: allow reducing depth in CI if runtime becomes a bottleneck.

#### D26. Store lifecycle management
- Default: provide store inventory and conservative store GC to prevent disk churn ("store explosion").
- Why: config fingerprint evolution naturally creates many stores over time.
- Escape hatch: `--force` required for deleting recent/active stores.
- Store discovery MUST be a single-level scan of `~/.ggrep/data/<store_id>` (no deep nesting).

#### D27. Lease steal protocol
- Default: lease stealing uses compare-and-swap (read/verify stale/write temp/atomic replace).
- Why: avoids races where a stale writer updates the lease during a steal attempt.
- Escape hatch: none (safety requirement).

#### D28. Compaction publish lease
- Default: compaction may build segments without the lease but MUST acquire the writer lease to publish; if the
  active snapshot changes, compaction MUST abort and retry/rebase.
- Why: prevents long compactions from blocking small syncs while preserving atomic publish.
- Escape hatch: none (safety requirement).

#### D29. IPC framing + payload limits
- Default: length-prefixed JSON framing with strict request/response size caps and early rejection of oversize frames.
- Why: prevents daemon crashes and payload-based DoS in multi-agent environments.
- Escape hatch: none (robustness requirement).

#### D30. Host-wide embed limiter
- Default: enforce a host-wide embed concurrency limit using a lease+heartbeat (stale lock recovery).
- Why: prevents cross-daemon embedding stampedes and API throttling.
- Escape hatch: allow higher limits in dev configs, but keep a hard cap.

#### D31. Repair mapping strategy
- Default: maintain `segment_file_index.jsonl` mapping (path_key -> segment_id) for incremental repair.
- Why: enables targeted repair without full reindex when a segment is lost.
- Escape hatch: if mapping is absent, `ggrep repair` must return "reindex required".

#### D32. Compaction hard limits
- Default: enforce `max_segments_per_snapshot`/`max_total_segments_referenced`/`max_tombstones_per_snapshot` and
  trigger immediate compaction (or fail publish if compaction is disabled).
- Why: prevents segment explosion and query latency collapse.
- Escape hatch: allow higher limits in dev configs, but keep a hard cap.

#### D33. Lease guard for atomic updates
- Default: serialize lease updates (acquire/heartbeat/steal) with a short-lived exclusive-create guard file.
- Why: filesystem rename is not a true CAS; guard prevents concurrent steal races.
- Escape hatch: none (safety requirement).

#### D34. Store explosion mitigation
- Default: separate `ignore_fingerprint` from `index_fingerprint` so ignore-only changes do NOT force a new store.
  Ignore changes publish a new snapshot (tombstones/filtering) within the same store.
- Why: trivial ignore edits should not trigger full reindex or new store creation.
- Escape hatch: if separation is not implemented, require segment reuse across stores (hardlinks or shared blobs).

#### D35. Compaction livelock avoidance
- Default: compaction MUST either rebase to the latest snapshot when it moves or acquire a short commit window
  to publish (shared-intent lock + brief exclusive window).
- Why: high-churn repos can otherwise cause compaction to abort forever.
- Escape hatch: allow compaction to back off and defer, but enforce hard limits to avoid unbounded growth.

#### D36. IPC socket path length mitigation
- Default: use short hashed socket paths (or `/tmp/ggrep-<uid>/` indirection) to avoid `sockaddr_un` length limits.
- Why: store ids can exceed Unix socket path length limits on Linux/macOS.
- Escape hatch: Linux abstract sockets are allowed where available; otherwise use short filesystem paths.

#### D37. Offline reader GC safety
- Default: offline readers SHOULD acquire a shared lock (e.g., `readers.lock`) and GC MUST acquire an exclusive lock
  before deleting artifacts.
- Why: best-effort time windows are not safe for paused/debugged offline readers.
- Escape hatch: if locks are unavailable on the platform, document best-effort behavior clearly.

#### D38. Windows atomic replace semantics
- Default: Windows rename operations MUST use `MoveFileEx` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`
  for `ACTIVE_SNAPSHOT` and `writer_lease.json` swaps, plus bounded retries on `AccessDenied`.
- Why: default move semantics are not atomic or durable enough under AV/indexer contention.
- Escape hatch: none (platform correctness requirement).

#### D39. Spec index + normative ownership
- Default: maintain a Spec Index that declares normative ownership and schema versions for each artifact.
- Why: prevents spec drift across multiple SSOT docs.
- Escape hatch: none (governance requirement).

#### D40. Conformance map (MUST coverage gate)
- Default: maintain a conformance map that links each critical MUST to a test or runtime assertion.
- Why: prevents silent implementation shortcuts that bypass contracts.
- Escape hatch: none (safety requirement).

#### D41. Open handle budgets
- Default: enforce per-query and daemon-global open handle budgets and surface them in status/health.
- Why: prevents file descriptor exhaustion under high segment counts + concurrency.
- Escape hatch: allow higher limits in dev configs, but keep a hard cap.

#### D42. Defaults freeze
- Default: numeric defaults (GC safety margin, compaction thresholds, IPC caps, backpressure, handle budgets)
  are locked early and centralized per domain (storage vs query).
- Why: prevents tests and ops behavior from drifting due to "tuning churn".
- Escape hatch: changes require explicit schema/defaults version bump and doc updates.

#### D43. Requirement IDs for MUSTs
- Default: all normative MUST statements in spec docs must carry stable IDs (e.g., `IDX.MUST.###`, `IPC.MUST.###`).
- Why: prevents enforcement drift when prose changes and enables conformance gating.
- Escape hatch: none (governance requirement).

#### D44. Module boundary enforcement
- Default: enforce module boundaries via `pub(crate)` and a dependency lint to prevent cross-layer coupling.
- Why: stable seams reduce regressions and clarify ownership.
- Escape hatch: none (architecture requirement).

#### D45. Maintenance fuzz + stress harness
- Default: fuzz/model tests must interleave publish/query/GC/compaction with pinned readers and lease steals.
- Why: the GC/compaction/pin interaction is the highest risk under load.
- Escape hatch: none (reliability requirement).

#### D46. Filesystem failure matrix
- Default: test matrix covers EACCES, read-only FS, EMFILE, rename failures, and sharing violations.
- Why: prevents partial publishes and daemon deadlocks under real-world failure modes.
- Escape hatch: none (safety requirement).

#### D47. JSON schema parity + golden fixtures
- Default: check in JSON Schemas and validate all doc examples + golden fixtures in CI.
- Why: prevents client breakage and schema drift.
- Escape hatch: none (contract requirement).

#### D48. Performance + operability budgets
- Default: define measurable budgets (latency, segment touches, GC time, publish time) and expose in status/health.
- Why: prevents silent scalability regressions.
- Escape hatch: allow dev overrides, but keep hard caps.

#### D49. Per-client fairness
- Default: identify clients in handshake and enforce per-client concurrency caps or fair queueing.
- Why: prevents a single client from starving others under load.
- Escape hatch: allow disabling fairness in single-user dev mode.

## Phase II Contract Specs (SSOT)

Wave II contract details live in these spec documents (treat as SSOT):

- `Tools/ggrep/Docs/Spec/GGREP-Snapshot-Index-Contracts-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Query-Daemon-Contracts-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Spec-Index-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Conformance-Map-v0.1.md`

Execution checklist:

- `Tools/ggrep/Docs/Plan/GGREP-Phase-II-Implementation-Checklist-v0.1.md`

## Implementation Tickets (Streams A/B/D/E/G/H/I)

### Stream A Tickets

**A1 - ChangeDetector interface + filesystem-first backend**
- Implement `ChangeDetector` returning `ChangeSet { add, modify, delete, rename }`.
- Filesystem-first backend: watcher/metadata scan + full hash confirm for publish candidates.
- Use a fast precheck (size + head hash) to filter mtime-only changes before full hashing.
- Reconcile with git diff or full hash audit on a time-based cadence and after sync idle.
- Persist `last_head` + `last_sync_ts` in `index_state.json`.
- Acceptance tests:
  - temp repo add/modify/rename/delete; `ChangeSet` matches.
  - rebase or missing `last_head` -> fallback to filesystem scan ChangeSet (no partial publish).

**A2 - FS fallback change detection (non-git)**
- Implement metadata scan + full hash confirm for candidates.
- Acceptance tests: modify file without mtime change; hash still detects change.

**A3 - SyncEngine accepts explicit ChangeSet**
- Implement `sync(changeset: Option<ChangeSet>)`.
- Acceptance tests: no full walk when changeset provided; only changed files processed.

**A4 - Delete + rename semantics**
- Deletes remove results; rename treated as delete+add unless mapping used.
- Optional: preserve chunk IDs when rename-only (hash unchanged).
- Acceptance tests:
  - delete removes hits; rename-only preserves IDs when enabled.
  - rename where destination fails to index -> strict publish fails or degraded publish (if enabled) without
    leaving the deleted source alive in the live view.

**A5 - Deterministic chunk IDs**
- Stable chunk ID from repo-relative path + file hash + offsets + chunker version.
- Acceptance tests: unchanged file yields identical IDs across runs.

**A6 - Ignore parity enforcement**
- One shared ignore evaluator across discovery, watcher, and manual sync.
- Acceptance tests: ignored file never indexed across paths.

**A7 - Path normalization + case collision detection**
- Implement canonical `path_key` (repo-relative + separator normalization).
- Compute `path_key_ci` (casefolded) for collision detection and cross-platform safety.
- Decide collision handling: default strict publish fails and `ggrep health` reports collisions.
- Acceptance tests:
  - Create two files whose paths collide under casefolding -> collision surfaced, strict publish fails,
    and health reports the offending paths.
  - Path normalization stable across indexing and querying.

**A8 - Out-of-root enforcement + symlink policy**
- Resolve candidate files to a real path and enforce `realpath.starts_with(canonical_root)`.
- Default symlink rule: index only if target resolves inside root; otherwise ignore + log.
- Acceptance tests:
  - Repo contains symlink to outside-root target -> never indexed; health/status reports skip reason.
  - Repo contains symlink to in-root target -> indexed deterministically (no duplicate identity).
  - Circular symlink loop -> Change Detector/walker rejects path without hang; health reports loop.

**A9 - Repo SSOT config + fingerprint inputs**
- Add support for tracked repo config (e.g. `.ggrep.toml`) and ensure its contents are included in the
  config fingerprint.
- Ensure ignore inputs (`.gitignore` + `.ggignore`) are hashed into `ignore_fingerprint`.
- Acceptance tests:
  - Changing repo config changes fingerprint/store id as expected.
  - Changing `.ggignore` changes fingerprint and reflected eligible set.

**A10 - Ignore conformance (golden) tests**
- Add golden tests for nested `.gitignore` semantics (negations, precedence, anchors, double-star).
- Acceptance tests: ignore evaluator matches expected outcomes for a fixed set of tricky fixtures.

### Stream B Tickets

**B1 - Lease-based single-writer lock + heartbeat**
- Implement `writer_lease.json` with owner + heartbeat; stale lock reclaim.
- Lease stealing uses compare-and-swap (read/verify stale/write temp/atomic replace).
- Serialize lease updates using an exclusive-create guard (`lease_guard.lock`).
- Acceptance tests: crash -> lease expires -> new writer takes over.
- Acceptance tests: zombie writer loses lease mid-sync and aborts without corrupting staging.
- Acceptance tests: concurrent steal attempts -> only one writer becomes owner.

**B2 - Lock-free readers**
- Query reads `ACTIVE_SNAPSHOT` once and pins snapshot for request lifetime.
- Acceptance tests: queries during publish return consistent old/new snapshot only.

**B3 - Watcher queue + debounce + reconciliation**
- Coalescing queue; debounce window; time-based reconciliation (git diff or audit) with idle-triggered runs.
- Acceptance tests: rapid edits trigger bounded syncs; reconciliation catches misses.
- Acceptance tests: symlink loops do not hang the watcher or event pipeline.

**B4 - Namespaced daemon sockets**
- Namespace sockets by store id + config fingerprint.
- Acceptance tests: two configs run daemons concurrently without collision.

**B5 - Low-impact mode enforcement**
- Enforce debounce/batch size/workers/reconciliation interval.
- Acceptance tests: verify throttles are applied when enabled.

**B6 - Concurrent-safe artifact downloads (models + grammars)**
- Add per-artifact locks + download-to-temp + validate/checksum + atomic rename.
- Pin grammar/model versions (avoid `latest`) and include artifact identity in the fingerprint.
- Acceptance tests:
  - Two processes downloading the same artifact cannot corrupt the cache.
  - Corrupt download is rejected and retried cleanly.

**B7 - CLI <-> daemon handshake + version negotiation**
- Expose daemon `binary_version`, `protocol_version`, and supported schema versions (query/status/health).
- CLI checks compatibility and fails fast on mismatch.
- Acceptance tests:
  - old/new CLI vs daemon mismatch produces deterministic "incompatible" behavior.
  - protocol range negotiation selects highest common version; no common version fails fast.

**B8 - IPC framing + socket permissions**
- Implement length-prefixed JSON framing with payload limits (request/response caps).
- Enforce socket/pipe permissions (0700 single-user, 0770 shared-store).
- Acceptance tests:
  - oversized payload dropped without daemon crash or high memory usage.
  - permission mismatch is rejected deterministically.

**B9 - Shared-store capability checks**
- On shared-store startup, verify exclusive-create, rename, and read-after-write semantics on the target filesystem.
- Fail fast with a clear error if the filesystem does not meet requirements (prefer per-worktree store fallback).
- Acceptance tests: simulated weak FS -> shared-store refused with actionable error.

**B10 - IPC socket path length mitigation**
- Use short hashed socket paths (or `/tmp/ggrep-<uid>/` indirection) to avoid Unix socket length limits.
- Acceptance tests: very long `store_id` still yields a valid socket path on Linux/macOS.

**B11 - Offline reader locks**
- Introduce a shared reader lock file for offline/direct reads; GC must acquire exclusive lock to delete artifacts.
- Acceptance tests: offline reader holds lock -> GC defers deletion; reader release allows GC to proceed.

**B12 - Module boundaries + dependency lint**
- Enforce module ownership boundaries (snapshot/lease/changes/query/daemon/status).
- Add a lightweight dependency lint to prevent cross-layer imports (daemon depends on query engine, not snapshot internals).
- Acceptance tests: lint fails on forbidden imports.

### Stream D Tickets

**D1 - Health gates for growth + store lifecycle**
- Extend `ggrep health --json` to report segment/tombstone growth and compaction overdue status by policy.
- Add store inventory and store GC commands (`ggrep stores --json`, `ggrep gc --stores`) with conservative defaults.
- Store discovery is a single-level scan of `~/.ggrep/data/<store_id>` (no deep nesting).
- Acceptance tests: health emits growth metrics; store inventory lists stores; store GC refuses by default unless safe.

**D2 - Repair mapping + corruption recovery**
- Define and implement `segment_file_index.jsonl` (path_key -> segment_id) for incremental repair.
- Avoid full rewrites of large mappings on every small sync (allow append-only deltas with periodic compaction).
- `ggrep repair` uses the mapping when present; otherwise returns "reindex required".
- If `ACTIVE_SNAPSHOT` is missing/corrupt, fallback to newest valid manifest that passes integrity checks.
- Acceptance tests:
  - missing manifest -> fallback selects only fully valid snapshot or fails with "store corrupt".
  - missing segment -> repair rebuilds affected paths when mapping exists; otherwise requests full reindex.

**D3 - Store explosion mitigation**
- Implement `ignore_fingerprint` separation (ignore-only changes publish new snapshots in the same store).
- If index-critical settings change, reuse immutable segments across stores when safe (hardlinks/shared blobs).
- Acceptance tests: ignore-only edits do not create a new store; segment reuse avoids full reindex when allowed.

**D4 - Audit counts vs segments**
- Add `ggrep audit` to verify `sum(segment.rows)` and manifest counts agree (or report drift deterministically).
- Acceptance tests: corrupted counts are detected and reported.

**D5 - JSON schema parity + golden fixtures**
- Check in JSON Schemas for manifest/status/health/query/handshake.
- Validate doc examples and golden fixtures against schemas in CI.
- Acceptance tests: doc examples validate; forward-compat fixtures fail fast.

**D6 - Performance + operability budgets**
- Define p95 query latency targets, max segments touched, max publish time, and GC time budgets.
- Surface budgets + current values in `status`/`health` and add perf smoke checks.
- Acceptance tests: perf budgets reported; regressions flagged in CI or nightly runs.

### Stream E Tickets

**E1 - SnapshotView caching**
- Cache SnapshotView per `snapshot_id` in the daemon (segment handles + tombstone filters).
- Eviction policy: drop cached SnapshotView when `active_snapshot_id` changes and `ref_count==0`.
- Acceptance tests: repeated queries reuse cached view; publish swaps to a new cached view without mixed snapshots.

**E2 - Background compaction publish lease**
- Allow compaction to build new segments without holding the writer lease.
- Require lease acquisition for publish; if the active snapshot changes, abort and retry/rebase.
- Compaction must also consolidate/prune tombstones.
- Implement rebase logic or a short commit window to avoid compaction livelock under high churn.
- Acceptance tests: long compaction does not block small syncs; publish is still atomic.

**E3 - Host-wide embed limiter + disk budgets**
- Add a host-wide embed limiter with lease+heartbeat stale lock recovery (avoid static lock files).
- Enforce store/log/cache budgets and surface warnings in status/health; refuse publishes when budgets are exceeded.
- Acceptance tests:
  - stale limiter lock is recovered after crash.
  - disk budget exceeded -> publish fails cleanly, last-good snapshot remains active.

**E4 - Compaction hard limits**
- Define `max_segments_per_snapshot` and `max_total_segments_referenced`.
- Background compaction SHOULD run before hard limits are reached; hard limits are a fail-safe.
- If limits are exceeded at end of sync, trigger immediate compaction (or fail publish if compaction is disabled).
- Acceptance tests: segment count stays bounded under churn; compaction correctness preserved.

### Stream H Tickets

**H1 - Query concurrency limiter + bounded queue**
- Add `max_concurrent_queries` semaphore and `max_query_queue_depth` admission control.
- Return "busy" when saturated; avoid unbounded memory growth.
- Acceptance tests:
  - Spawn >limit concurrent queries -> extras get busy response; daemon remains responsive.

**H2 - Per-query timeout + cancellation propagation**
- Enforce `query_timeout_ms` end-to-end.
- Ensure lexical/vector/rerank/format stages honor cancellation.
- Acceptance tests:
  - Inject artificial delay in a query stage -> query times out and downstream work cancels.

**H3 - Sync anti-starvation under heavy query load**
- Ensure sync tasks have reserved capacity (separate pools or explicit priority/quotas).
- Acceptance tests:
  - Saturate query concurrency -> sync can still publish a snapshot.

**H4 - Query observability**
- Add queue/in-flight/timeout/slow-query counters to `ggrep status --json`.
- Emit per-stage query timings in structured logs.
- Acceptance tests:
  - Under load, status reports queue depth/in-flight; timeout counters increment on forced timeouts.

**H5 - Explainability (`--explain` + JSON meta)**
- Add `--explain` output (and JSON meta) including snapshot id, fingerprint, profile/quotas/weights, and stage timings.
- Acceptance tests:
  - `--explain --json` includes required fields and is stable across runs.

**H6 - Query caps + interruptible cancellation**
- Enforce per-query caps (`max_candidates`, `max_total_snippet_bytes`, `max_snippet_bytes_per_result`) and surface cap
  hits in `--explain`/JSON metadata.
- Ensure cancellation checks occur inside long-running retrieval loops (not only at stage boundaries).
- Acceptance tests: artificial large candidate sets hit caps deterministically; cancellation stops work promptly.

**H7 - IPC robustness + fuzzing**
- Implement IPC framing validation and schema validation with structured `invalid_request` errors.
- Add fuzz tests for framing (garbage bytes, truncated frames) and oversized payload DoS.
- Acceptance tests: malformed messages never panic; oversized payloads are dropped immediately.

**H8 - Output sanitization**
- Strip or escape control characters and ANSI sequences in CLI output by default.
- Provide `--raw` to opt out for trusted workflows.
- Acceptance tests: malicious snippet does not inject terminal control sequences.

**H9 - Open handle budgets**
- Enforce `max_open_segments_per_query` and `max_open_segments_global`.
- Surface handle usage/budgets in `status`/`health`.
- Acceptance tests: low `ulimit -n` environment triggers busy/timeout without corruption.

**H10 - Per-client fairness**
- Require `client_id` in handshake (or derive a stable id) and enforce per-client concurrency caps.
- Implement optional weighted fair queueing for mixed workloads.
- Acceptance tests: single noisy client cannot starve others.

### Stream G Tickets

**G1 - Snapshot Manager (staging -> manifest -> pointer swap)**
- Build staging, write manifest, publish via `ACTIVE_SNAPSHOT` atomic rename.
- Retain N previous snapshots for rollback.
- Acceptance tests: crash mid-staging leaves last-good snapshot intact.

**G2 - Integrity checks at publish**
- Validate row counts, segment existence, and changed-path coverage.
- Acceptance tests: corrupted staging fails publish; old snapshot remains active.

**G3 - Failure handling + retry policy**
- Per-item batch results, bounded retries, backoff.
- Acceptance tests: partial failures do not abort other batches; publish behavior matches D9.

**G4 - Optional degraded snapshots**
- `--allow-degraded` publishes with `degraded=true`; status/query surfaces it.
- Acceptance tests: degraded flag shown in status and query metadata.

**G5 - Property-based sync fuzz tests (invariant hardening)**
- Add property-based tests generating random sequences of add/modify/delete/rename and editor-style
  save patterns.
- Use `proptest` (preferred) for fixed-seed reproducible cases.
- Use a deterministic fake embedder to keep tests fast and reproducible.
- Acceptance tests:
  - Fixed-seed suite runs in CI and preserves invariants:
    - deleted files have 0 hits
    - snapshot isolation (no mixed snapshot reads)
    - tombstones applied consistently
    - no out-of-root indexing

**G6 - Durable publish (fsync semantics)**
- Ensure manifest/pointer swap is durable across crashes by fsyncing written files and parent directories.
- Acceptance tests:
  - Simulated crash after publish does not produce missing/partial manifests or broken pointers.
  - Windows: retry `ACTIVE_SNAPSHOT` rename on transient `AccessDenied`.
  - Disk full during staging/manifest write -> publish fails cleanly; last-good snapshot remains active.

**G7 - Crash-point injection publish tests**
- Add deterministic crash-point injection tests across publish boundaries (after segment write, after tombstone write,
  after manifest write, during pointer swap, after pointer swap before cleanup).
- Acceptance tests: last-good snapshot remains queryable; no partial state becomes active.

**G8 - Concurrency model checking (loom)**
- Add loom (or equivalent) tests for the snapshot pin + publish + GC/cache interactions.
- Acceptance tests: model checks pass for the concurrency boundary conditions under bounded exploration.

**G9 - Forward-compat schema tests**
- Add fixtures with `schema_version` greater than current (e.g., 99) for manifest/status/health.
- Acceptance tests: older binaries fail fast with `incompatible` and do not crash.

**G10 - Spec index + conformance map**
- Create a Spec Index that declares normative ownership and schema versions.
- Add requirement IDs to critical MUSTs in specs.
- Add a conformance map linking critical MUSTs to tests/assertions.
- Add a doc-lint that validates JSON examples against checked-in schemas.
- Acceptance tests: doc examples validate; conformance map exists and references tests.

**G11 - Maintenance stress harness**
- Interleave publish/query/GC/compaction with pinned readers and lease steals.
- Include multi-process torture runs that kill writer/reader processes mid-flight.
- Acceptance tests: no partial state observed; GC never deletes reachable artifacts.

**G12 - Filesystem failure matrix**
- Inject EACCES, read-only filesystem, EMFILE, rename failures, and sharing violations.
- Acceptance tests: strict publish fails cleanly; daemon remains responsive.

### Stream I Tickets

**I1 - Repo config validation + safety caps**
- Validate `.ggrep.toml` as untrusted input (scope cannot expand outside root; cannot disable safety caps).
- Enforce hard caps for file size, max chunks per file, and max bytes per sync.
- Acceptance tests: invalid configs are rejected with clear errors; caps are enforced consistently.

## Phase II Milestones

**M0 - Decision Freeze + Contracts**
- Gate: Decision Record locked (D1-D49).
- Gate: manifest schema + `ACTIVE_SNAPSHOT` semantics documented.
- Gate: CLI contract for `status/health/audit/repair` drafted.
- Gate: Spec Index + Conformance Map drafted.

**M1 - Snapshot Core (Atomic Publish + Lock-Free Reads)**
- Gate: snapshot manager implemented.
- Gate: lease-based writer lock implemented.
- Gate: concurrency test proves readers never see partial state.

**M2 - Incremental Sync (Filesystem-first + Watcher Discipline)**
- Gate: filesystem-first ChangeDetector + git reconciliation implemented.
- Gate: watcher uses ChangeSet path; no full scan on save.
- Gate: add/modify/delete/rename touches changed paths only.
- Gate: path normalization/out-of-root enforcement is active (no traversal; collisions detected).

**M3 - Reliability + Recovery**
- Gate: crash mid-sync leaves last-good snapshot intact.
- Gate: `ggrep status --json` + `ggrep health` implemented and documented.
- Gate: `audit/repair` reconciles and rebuilds missing/stale files only.
- Gate: query backpressure/timeouts + sync anti-starvation implemented and validated under load.
- Gate: CI includes fixed-seed property-based fuzz suite for sync invariants.
- Gate: protocol handshake/version negotiation is implemented and validated.
- Gate: crash-point injection publish tests are implemented.
- Gate: IPC framing + payload limits are implemented and fuzzed.
- Gate: forward-compat schema tests fail fast on newer versions.

**M4 - Performance + Quality Gates**
- Gate: embedding cache keyed by chunk hash + config fingerprint.
- Gate: compaction thresholds + compaction command.
- Gate: eval suite baseline JSON + regression gates.
- Gate: segment/tombstone growth health gates and SnapshotView caching are implemented.
- Gate: store inventory + store GC commands exist with conservative defaults.
- Gate: host-wide embed limiter + disk budgets are enforced.
- Gate: compaction hard limits prevent segment explosion.

## Deliverables And Acceptance Criteria

- Deterministic sync: index reflects repo within one sync cycle.
- Atomic snapshots: no partial results during sync; rollback is safe.
- Reproducible eval: baseline JSON report and regression thresholds.
- Multi-agent safe: no daemon thrash, no store corruption under parallel use.
- Clear ops: status + logs + audit/repair commands exist and are documented.
- Drift detection: `ggrep health` reports consistent MetaStore vs vector store.
- Incremental sync: watcher updates only changed files (no full scan on save).
- Lock-free reads: concurrent searches never block on sync.
- Snapshot metadata: query results indicate snapshot HEAD and dirty status.
- Query QoS: daemon enforces max concurrency/queue depth and per-query timeouts; sync continues to
  publish under heavy query load.
- Path safety: no out-of-root indexing; casefold collisions are detected and surfaced (and fail strict
  publish by default).
- Invariant hardening: property-based fuzz tests exist and gate regressions.
- Deterministic artifacts: grammar/model downloads are pinned, validated, and concurrency-safe.
- Repo SSOT config: repo config is hashed into `config_fingerprint`; ignore inputs are hashed into `ignore_fingerprint`.
- Durable publish: pointer swap is crash-durable (fsync semantics), not just atomic.
- Explainability: `--explain`/JSON meta makes ranking/profile behavior inspectable for tuning.
- Protocol compatibility: CLI/daemon version negotiation is deterministic and fails fast on incompatibility.
- Ignore conformance: nested `.gitignore` semantics are regression-protected by golden tests.
- Resource safety: per-query caps prevent runaway memory/IO; cancellation is interruptible in long loops.
- Store lifecycle: store inventory and conservative store GC prevent disk churn over time.
- Contract enforcement: Spec Index and Conformance Map exist and are kept in sync with tests.

## Validation Plan

- Unit tests for change detection, delete handling, and ignore parity.
- Integration test: simulate add/modify/delete/rename and confirm index diff.
- Eval suite run with fixed config and stored baseline results.
- Daemon soak test with rapid file changes and concurrent searches.
- Health check test: induced drift is detected and reported.
- Crash test: kill sync mid-commit and confirm last-good snapshot survives.
- Concurrency test: multi-agent queries during sync with lock-free reads.
- Load test: sustained concurrent queries while file changes occur; verify backpressure activates and
  sync still publishes.
- Path safety tests: out-of-root symlinks and traversal attempts are excluded and reported.
- Property-based fuzz tests: random add/modify/delete/rename sequences preserve invariants (fixed
  seed in CI; longer soak optional).
- Ignore conformance tests: golden `.gitignore` fixtures validate nested semantics.
- Protocol mismatch tests: CLI/daemon handshake incompatibilities fail fast deterministically.
- IPC fuzz tests: malformed/oversized payloads do not crash the daemon.
- IPC socket path length tests: long store_id still yields a valid socket path.
- Crash-point injection tests: publish boundary crashes never produce partial active snapshots.
- Concurrency model checking (loom): snapshot pin/publish/GC/cache races are explored under bounded schedules.
- Forward-compat tests: newer schema versions fail fast with `incompatible`.
- Corruption recovery tests: missing/corrupt `ACTIVE_SNAPSHOT` falls back to newest valid manifest or fails cleanly.
- Embed limiter tests: stale host-wide limiter lock recovers after crash.
- Shared-store capability tests: weak filesystem semantics are detected and refused.
- Store explosion tests: ignore-only changes do not create new stores when `ignore_fingerprint` separation is enabled.
- Compaction livelock tests: compaction eventually publishes under high churn (rebase/commit window).
- Open handle tests: low `ulimit -n` triggers busy/timeout without corruption.
- Doc schema validation: JSON examples in docs validate against checked-in schemas.
- Conformance map check: critical MUSTs are mapped to tests/assertions.
- Requirement ID check: all critical MUSTs have stable IDs referenced by the conformance map.
- Maintenance stress harness: publish/query/GC/compaction interleavings hold invariants under kill/restart.
- Filesystem failure matrix: EACCES/EMFILE/rename/share violations do not publish partial state.
- Performance budgets: p95 latency/publish time/GC time are tracked and regressions flagged.
- Fairness tests: per-client caps prevent starvation under mixed loads.

## Mermaid Diagram Checklist

- Reader lane: read `ACTIVE_SNAPSHOT` once, open manifest, query snapshot segments/tables,
  rank + profile output.
- Query admission: semaphore/queue/backpressure occurs before snapshot read; timeout/cancellation is
  enforced end-to-end.
- Writer lane: acquire lease, compute ChangeSet, build staging, embed/write segments,
  integrity checks, revalidate lease epoch, write manifest, atomic rename `ACTIVE_SNAPSHOT`, release lease, cleanup.
- Pointer swap link: readers may still see the old pointer until rename completes.
- Maintenance lane: compaction/GC acquires writer lease to publish/delete artifacts.

## Workflow Diagram

See `Tools/ggrep/Docs/Plan/ggrep-sync-index-lifecycle.mmd`.
