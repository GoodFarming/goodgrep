# GGREP Query + Daemon Contracts (v0.1)

Status: Draft v0.1  
Scope: `Tools/ggrep` (goodgrep repo)  
Purpose: Lock query/daemon behavior for multi-agent reliability (QoS, errors, status/health schemas).

This document is **normative**: it uses MUST / SHOULD / MAY language to reduce implementation drift.

## Goals

- Keep the daemon responsive under heavy multi-agent load.
- Provide predictable query admission (backpressure), deadlines, and cancellation.
- Guarantee lock-free reads pinned to a snapshot (snapshot isolation).
- Provide stable machine-readable status/health contracts for agents and operators.

## Phase II Scope (Execution)

- Supported OS: Linux-only (Windows/macOS deferred).
- IPC transport: Unix domain sockets (Windows named pipes deferred).

## Requirement IDs (Critical)

Critical MUST requirements in this spec carry stable IDs for conformance tracking:

- `IPC.MUST.001` Protocol handshake selects highest common version.
- `IPC.MUST.002` IPC framing + payload caps enforced.
- `IPC.MUST.003` Output sanitization (no ANSI/control chars by default).
- `IPC.MUST.004` Deterministic ordering + tie-break rules.
- `IPC.MUST.005` Open handle budgets enforced.

## Protocol Compatibility (CLI <-> Daemon)

Multi-agent environments will routinely have mismatched CLI/daemon binaries. Wave II MUST define a deterministic
handshake and mismatch behavior.

- The daemon MUST expose:
  - `binary_version` (string; informational)
  - `protocol_version` (integer; breaking changes only)
  - supported `schema_version` ranges for: query success/error, status, and health
- The CLI SHOULD perform a handshake before issuing a query when talking to a daemon.
- If protocol versions are incompatible, the CLI MUST fail fast with a clear error message (do not attempt a query).

### Handshake schema (required)

Client -> daemon:

```json
{
  "protocol_versions": [1],
  "store_id": "<store_id>",
  "config_fingerprint": "<sha256-hex>",
  "client_id": "agent-123",
  "client_capabilities": ["json", "explain"]
}
```

Daemon -> client:

```json
{
  "protocol_version": 1,
  "protocol_versions": [1],
  "binary_version": "0.6.0",
  "supported_schema_versions": {
    "query_success": [1],
    "query_error": [1],
    "status": [1],
    "health": [1]
  },
  "store_id": "<store_id>",
  "config_fingerprint": "<sha256-hex>"
}
```

Rules:

- If `store_id` or `config_fingerprint` mismatches, the daemon MUST reject with `invalid_request`.
- Protocol negotiation MUST select the highest common version between client and daemon (`IPC.MUST.001`).
- If no common version exists, the daemon MUST reject with `incompatible`.
- For backward compatibility, a single `protocol_version` MAY be accepted as an implicit singleton list.
- `client_id` SHOULD be provided by clients; the daemon MAY reject missing ids when fairness is enabled.

## IPC Transport, Framing, And Permissions

- The daemon MUST expose a local IPC endpoint:
  - Unix: domain socket under `~/.ggrep/daemon/<store_id>/` (recommended).
  - Deferred Windows note: named pipe with equivalent ACLs.
- Socket/pipe permissions MUST be restrictive by default:
  - single-user mode: directory `0700`, socket/pipe owner-only.
  - shared-store mode: directory `0770` with group ACLs.
- The daemon MUST reject connections from clients without read/write permissions on the IPC endpoint.

### Message framing (required)

- IPC messages MUST be length-prefixed JSON:
  - 4-byte unsigned big-endian length prefix
  - UTF-8 JSON payload of that exact length
- The daemon MUST enforce a maximum payload size and MUST drop oversized requests immediately without allocating
  the full payload.
- Framing and payload caps are mandatory (`IPC.MUST.002`).
- Defaults (configurable, with hard caps):
  - `max_request_bytes` default 1 MiB
  - `max_response_bytes` default 10 MiB
- Handshake MUST be the first message; if a client sends any other message, the daemon MUST respond with
  `invalid_request` and close the connection.

## Daemon Lifecycle + Store Transitions

- The daemon MUST pin `store_id` + `config_fingerprint` at startup and MUST NOT silently switch stores.
- If repo config/ignore inputs change and the computed fingerprint differs, the daemon MUST:
  - exit and allow a new daemon to start, or
  - enter a "stale" state that stops watching/syncing and surfaces a warning in status.
- In stale state, the daemon MAY continue serving read-only queries against the last-good snapshot but MUST:
  - refuse sync/write operations, and
  - include a deterministic warning (e.g., `stale_config_warning`) in JSON responses and status.
- The daemon SHOULD debounce config-change detection (separate from file watching) to avoid rapid restart loops;
  default debounce SHOULD be 2000ms.
- The CLI MUST refuse to use a daemon whose `store_id`/`config_fingerprint` does not match the CLI's effective
  fingerprint (unless explicitly forced).
- When the CLI detects a stale daemon (warning or mismatch), it SHOULD attempt to spawn a new daemon with the
  current fingerprint and retry once before returning an error (unless auto-restart is explicitly disabled).
- Status SHOULD surface when multiple daemons are running for the same canonical root and SHOULD warn about
  watcher fanout risk.

## Query Lifecycle (Contract)

For each query request:

1) **Admission** (Stream H)
   - Apply backpressure limits before any snapshot read.
2) **Snapshot pin**
   - Read `ACTIVE_SNAPSHOT` once and pin that `snapshot_id` for the request lifetime.
3) **Retrieve**
   - Execute retrieval across the pinned snapshot (segments + tombstones).
4) **Rank**
   - Apply hybrid ranking (dense + lexical) and profile quotas.
5) **Format**
   - Format output according to the requested profile/snippet mode.

## Path Scoping (Folder Search)

To support “search within a folder”, query requests MAY include an optional `path` scope.

- If `path` is present, it MUST be an absolute path under the daemon’s `canonical_root`; otherwise the daemon MUST
  reject with `invalid_request`.
- If `path` is a directory, the daemon MUST restrict results to files whose stored path begins with that directory
  prefix.
- If `path` is a file, the daemon SHOULD treat it as an exact match (or a prefix with an added trailing separator)
  to avoid accidental overmatch.
- CLI UX recommendation: when invoked without an explicit path argument, the CLI SHOULD send the current working
  directory as `path` so `ggrep "query"` searches “here” by default while still using the repo-root store.

## Output Safety And Sanitization

CLI output MUST be safe by default (`IPC.MUST.003`):

- Control characters and ANSI escape sequences MUST be stripped or escaped in text snippets and paths.
- JSON output MUST be valid UTF-8. If bytes cannot be decoded, replace with `\uFFFD` and emit a warning.
- A `--raw` (or equivalent) flag MAY allow unsanitized output for trusted workflows.
- Truncation MUST be deterministic and MUST be surfaced via `limits_hit`/`warnings`.

## Deterministic Ordering And Output

Queries MUST provide deterministic ordering when scores are tied or near-tied (`IPC.MUST.004`):

- When scores are equal (or within an epsilon), results MUST be sorted by:
  1) secondary score descending (e.g., lexical score when available)
  2) `path_key` ascending
  3) byte/line start offset ascending (prefer byte if present; fall back to `ordinal` if offsets missing)
  4) `row_id` ascending

Quota and truncation logic MUST apply deterministically when scores are tied.

For evaluation and regression tests, the daemon MUST support a deterministic mode that:

- pins ANN settings to fixed values or uses exact search
- disables nondeterministic concurrency reordering
- produces byte-for-byte identical JSON output for the same snapshot + query
- de-duplicates and deterministically orders `warnings[]`/`limits_hit[]`
- omits wall-clock timestamps (or moves them to a non-deterministic metadata block)
- sets `timings_ms` to zeroed values or omits it entirely in deterministic mode
- formats floats deterministically (fixed precision; no locale or scientific notation)

## Backpressure (Admission Control)

### Controls

- `max_concurrent_queries` (default 8)
- `max_query_queue_depth` (default 32)

### Contract

- The daemon MUST enforce `max_concurrent_queries` via a semaphore (or equivalent).
- The daemon MUST bound the waiting queue to `max_query_queue_depth`.
- If the queue is full, the daemon MUST fail fast with a "busy" response.
- Busy responses SHOULD include a retry hint (`retry_after_ms`).

## Per-Client Fairness

When multiple agents share a daemon, fairness prevents a single client from starving others.

- The daemon SHOULD enforce per-client concurrency caps when `client_id` is provided.
- A weighted fair queue MAY be used for mixed workloads (interactive vs batch).
- Fairness must not bypass global backpressure limits.

## Deadlines, Timeouts, And Cancellation

### Controls

- `query_timeout_ms` (default 60000)

### Contract

- Every query MUST have a deadline enforced end-to-end.
- Cancellation MUST propagate to all stages:
  - lexical retrieval
  - vector retrieval across segments
  - reranking
  - output formatting
- Once a deadline expires, the daemon MUST stop doing work for that query (no runaway CPU).

## Sync Anti-Starvation (Fairness)

### Required behavior

- Sync MUST be able to publish snapshots even when query load saturates the daemon.

### Mechanism (Wave II default)

Wave II MUST implement **reserved permits**:

- Query admission is controlled by a query semaphore (`max_concurrent_queries`) + bounded queue.
- Sync work uses separate permits (or a separate pool) so publish-critical sync steps never wait behind the query queue.
- Query admission defaults MUST remain conservative to leave CPU headroom for sync.
- Low-impact mode MUST further reduce sync concurrency and increase debounce to avoid contention.

## Error Contract (CLI + JSON + MCP)

### Error codes

Wave II MUST standardize error codes:

- `busy`: admission rejected due to backpressure
- `timeout`: query deadline exceeded
- `cancelled`: query cancelled due to shutdown or explicit cancellation
- `invalid_request`: invalid flags or parameters
- `internal`: unexpected error
- `incompatible`: CLI/daemon protocol mismatch

### JSON error shape

When `--json` is requested (or when called via MCP), errors MUST be structured:

```json
{
  "error": {
    "code": "busy",
    "message": "daemon busy",
    "retry_after_ms": 250,
    "snapshot_id": "01HZZ...",
    "request_id": "01HZZ..."
  }
}
```

### CLI exit codes

CLI SHOULD use distinct exit codes for automation:

- `0` success
- `10` busy
- `11` timeout
- `12` cancelled
- `13` incompatible
- `1` other error

## Success Response JSON Schema (v1)

When `--json` is requested (or when called via MCP), successful search responses MUST include stable metadata so
agents can debug and tune behavior.

Minimum required shape:

```json
{
  "schema_version": 1,
  "request_id": "01HZZ...",
  "store_id": "<store_id>",
  "config_fingerprint": "<sha256-hex>",
  "ignore_fingerprint": "<sha256-hex>",
  "query_fingerprint": "<sha256-hex>",
  "embed_config_fingerprint": "<sha256-hex>",
  "snapshot_id": "01HZZ...",
  "git": { "head_sha": "abc123...", "dirty": true, "untracked_included": false },
  "mode": "planning",
  "limits": { "max_results": 20, "per_file": 3, "snippet": "short" },
  "limits_hit": [],
  "warnings": [],
  "timings_ms": { "admission": 1, "snapshot_read": 0, "retrieve": 12, "rank": 4, "format": 2 },
  "results": []
}
```

Notes:

- `schema_version` MUST be incremented only for breaking changes.
- `request_id` MUST be unique per request (ULID recommended).
- `snapshot_id` MUST match the pinned snapshot used for retrieval.
- `config_fingerprint` is the index fingerprint; `query_fingerprint` MUST reflect query-only knobs and must not
  force store reindexing.
- `ignore_fingerprint` reflects ignore inputs; ignore-only changes should not require a new store.

## Explainability (`--explain`)

Explainability is required for quality tuning and multi-agent trust.

- `--explain` MUST surface:
  - pinned `snapshot_id` and `head_sha`/dirty flags (when available),
  - chosen mode/profile and quotas/weights,
  - candidate mix (counts by source/bucket),
  - per-stage timings, and
  - any saturation/backpressure decisions (queued vs admitted, retry-after).
- `--explain --json` MUST embed the same information in machine-readable fields (prefer extending `timings_ms`
  and adding an `explain` object).
- Logs, `status`, and `health` outputs MUST be metadata-only (no raw file/chunk text and no embedding vectors).
  `--explain` MUST NOT emit raw embeddings and SHOULD avoid emitting large raw content beyond the requested
  snippet output.

## Query Resource Limits (Memory/IO/Candidate Caps)

Admission control prevents overload, but individual queries can still exhaust CPU/memory/IO if they retrieve or format
too much data. Wave II MUST enforce caps and surface when caps affect completeness.

Minimum required caps (configurable):

- `max_candidates` (pre-rerank cap; bounds memory and CPU)
- `max_total_snippet_bytes` (bounds response size)
- `max_snippet_bytes_per_result` (bounds per-result payload)
- `max_open_segments_per_query` (bounds per-query file handle usage)

Contract:

- The daemon MUST enforce these caps during retrieval/ranking/formatting.
- Caps MUST have hard upper bounds compiled into the binary; repo config MAY set stricter values only.
- If caps truncate work, the daemon MUST surface that fact in `--explain` and SHOULD surface it in JSON responses
  via `warnings[]` and/or `limits_hit[]` fields (shape defined below).

### `limits_hit[]` and `warnings[]` schema (v1)

When present, arrays MUST be deterministic and sorted by `code` (byte order). If `path_key` is present, it MUST be
used as a secondary sort key (byte order).

```json
{
  "limits_hit": [
    { "code": "max_candidates", "limit": 2000, "observed": 2450 }
  ],
  "warnings": [
    { "code": "stale_config", "message": "daemon config fingerprint is stale" }
  ]
}
```

Rules:

- `code` MUST be stable once shipped (no renames without schema bump).
- `limit` MUST be the configured cap that applied.
- `observed` SHOULD be the pre-cap count when known.
- Entries MUST be de-duplicated by `(code, path_key)` when `path_key` is present; otherwise by `code`.
- Warnings MUST NOT include raw chunk text or embeddings.

## Global Resource Governance (Host-Wide)

Multi-agent environments often run multiple daemons. Resource limits MUST be enforced at the appropriate scope:

- host-wide where resources are truly shared (e.g., embedding concurrency / API rate limits), and
- per-daemon-process where the OS enforces limits per process (e.g., file descriptors / open handles).

### Host-wide embed limiter (required)

- A global embed limiter MUST cap concurrent embed batches across all daemons on the host.
- The limiter MUST be implemented with a lease+heartbeat (reuse `writer_lease` semantics) rather than a static
  lockfile, to allow stale lock recovery after crashes.
- If the limiter cannot be acquired, the daemon MUST back off and retry with bounded jitter.

### Disk and cache budgets (required)

- The daemon MUST enforce disk budgets for:
  - store size (`max_store_bytes`)
  - logs size (`max_log_bytes`)
  - cache size (`max_cache_bytes`)
- When thresholds are exceeded or nearing limit, the daemon MUST:
  - surface warnings in `status`/`health`,
  - refuse new publishes if the store is out of budget (strict publish remains intact), and
  - keep the last-good snapshot active.

### Open handle budgets (required)

- The daemon MUST enforce a per-daemon-process budget for open segment handles (`max_open_segments_global`)
  (`IPC.MUST.005`).
- Queries MUST respect `max_open_segments_per_query` and close handles promptly after use.
- Status/health MUST surface current usage vs budgets to prevent `ulimit` exhaustion.

## Query Observability (Contract)

### Required metrics

The daemon MUST track and surface:

- in-flight queries
- queue depth
- busy rejections (counter)
- timeouts (counter)
- slow query count (counter) + threshold config (`slow_query_ms`, default 2000)
- per-stage timings: admission, snapshot read, retrieve, rank, format

### Logging

Queries SHOULD emit a structured log event with:

- `request_id`
- `snapshot_id`
- `profile`
- `limits` (m/per-file/snippet mode)
- stage durations
- final status (ok/busy/timeout/cancelled/error)

## IPC Robustness And Fuzzing

- The daemon MUST handle malformed JSON, invalid schemas, and partial frames without panicking.
- The daemon MUST reject oversized payloads before allocation (see framing limits).
- The daemon MUST return a structured `invalid_request` error for schema/validation issues whenever possible.
- IPC fuzz tests MUST cover:
  - framing fuzz (garbage bytes, truncated length prefix, oversized length)
  - logic fuzz (valid JSON with invalid types/values)
  - oversized payload DoS (daemon drops connection without excessive memory use)

## `ggrep status --json` Schema (v1)

`ggrep status --json` MUST emit a stable schema:

```json
{
  "schema_version": 1,
  "store_id": "<store_id>",
  "canonical_root": "/abs/path",
  "config_fingerprint": "<sha256-hex>",
  "ignore_fingerprint": "<sha256-hex>",
  "daemon": {
    "running": true,
    "pid": 1234,
    "started_at": "2026-01-01T12:00:00Z",
    "binary_version": "0.6.0",
    "protocol_version": 1,
    "stale": false,
    "supported_schema_versions": {
      "query_success": [1],
      "query_error": [1],
      "status": [1],
      "health": [1]
    }
  },
  "snapshot": {
    "active_snapshot_id": "01HZZ...",
    "head_sha": "abc123...",
    "dirty": true,
    "untracked_included": false,
    "degraded": false,
    "created_at": "2026-01-01T12:00:00Z"
  },
  "sync": {
    "state": "idle",
    "last_sync_at": "2026-01-01T12:00:00Z",
    "last_result": "ok",
    "last_duration_ms": 12345,
    "staging_txn_id": null
  },
  "queries": {
    "max_concurrent": 8,
    "max_queue_depth": 32,
    "timeout_ms": 60000,
    "in_flight": 2,
    "queue_depth": 0,
    "busy_total": 0,
    "timeouts_total": 0,
    "slow_total": 0
  },
  "resources": {
    "embed_global": { "max_concurrent": 2, "in_use": 1, "stale_lock": false },
    "disk": {
      "store_bytes": 123456789,
      "store_budget_bytes": 1073741824,
      "cache_bytes": 12345678,
      "cache_budget_bytes": 268435456,
      "log_bytes": 1048576,
      "log_budget_bytes": 10485760
    },
    "open_handles": {
      "segments_open": 64,
      "segments_budget": 512
    }
  }
}
```

Notes:

- `schema_version` MUST be incremented only for breaking changes.
- Unknown fields MUST be ignored by clients.

## `ggrep health --json` Schema (v1)

Health is a set of checks with severities.

```json
{
  "schema_version": 1,
  "store_id": "<store_id>",
  "active_snapshot_id": "01HZZ...",
  "ok": true,
  "checks": [
    { "code": "manifest_present", "severity": "ok", "message": "active manifest found" },
    { "code": "segments_present", "severity": "ok", "message": "all segment refs exist" },
    { "code": "tombstones_enforced", "severity": "ok", "message": "tombstone filter active" },
    { "code": "casefold_collisions", "severity": "ok", "message": "no collisions detected" }
  ]
}
```

Required checks (minimum):

- active manifest exists and parses
- all referenced artifacts exist (segments/tombstones)
- tombstone enforcement active (cannot be bypassed by public query paths)
- path safety (out-of-root excluded) and collision detection results
- drift check between index_state/manifest counts and store row counts (bounded sampling allowed)
- repo hygiene: non-ignored untracked file count/bytes (warn by default; fail shared-store publish by policy)
- artifact integrity: cached model/grammar artifacts validate and are not partially written
- segment/tombstone growth: report `segments_count`, `tombstones_count` (and size where available), and flag when
  compaction is overdue by policy (thresholds live in config)
- disk budgets: report store/cache/log usage vs budgets and warn when near or over limits
- embed limiter: warn if the global limiter lock appears stale or unrecoverable
- open handles: report open segment handles vs budget and warn when near or over limits

## Daemon Namespacing (Socket Contract)

The daemon endpoint MUST be namespaced at least by:

- `store_id`
- `config_fingerprint`

This prevents cross-config daemon collisions in multi-agent environments.

Implementations MUST ensure socket paths remain within OS limits (e.g., Unix `sockaddr_un` length). Use short
hash paths by default (recommended: `~/.ggrep/daemon/<hash(store_id+config_fingerprint)>.sock`) rather than long
human-readable paths. Temp-directory indirection may be used as a fallback.

## Defaults (Wave II)

Defaults MUST be conservative and safe:

- `max_concurrent_queries=8`
- `max_query_queue_depth=32`
- `query_timeout_ms=60000`
- `slow_query_ms=2000`
- `max_request_bytes=1_048_576`
- `max_response_bytes=10_485_760`
- `max_open_segments_per_query=64`
- `max_open_segments_global=512`

Low-impact mode MUST reduce contention further (see Decision Record D7).
