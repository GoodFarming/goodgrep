# GGREP Snapshot + Index Contracts (v0.1)

Status: Draft v0.1  
Scope: `Tools/ggrep` (goodgrep repo)  
Purpose: Lock the correctness/reliability contracts before implementing Wave II hardening.

This document is **normative**: it uses MUST / SHOULD / MAY language to reduce implementation drift.

## Goals

- Provide snapshot isolation for queries (no partial state).
- Support incremental sync driven by bounded change sets.
- Be safe under multi-agent concurrent use (many readers, single writer).
- Be deterministic and crash-safe (last-good snapshot always usable).

## Phase II Scope (Execution)

- Supported OS: Linux-only (Windows/macOS deferred).
- Embedding runtime: CPU-only (no CUDA/GPU requirements).

Platform-specific notes for Windows/macOS may appear later in this document, but are **informative** and not Phase II
gating requirements.

## Requirement IDs (Critical)

Critical MUST requirements in this spec carry stable IDs for conformance tracking:

- `IDX.MUST.001` Tombstone enforcement is structural (no bypass).
- `IDX.MUST.002` Publish preflight checks lease_epoch + owner.
- `IDX.MUST.003` Durability barrier before manifest write.
- `IDX.MUST.004` GC requires writer lease.
- `IDX.MUST.005` Offline readers take shared lock; GC takes exclusive.
- `IDX.MUST.006` Checksums verified before query.
- `IDX.MUST.007` Compaction prunes tombstones.

## Non-goals

- Changing embedding model families.
- Adding remote/cloud indexing.

## Glossary

- **Canonical root**: the repository root used for normalization and store identity.
- **Store**: on-disk index artifacts for one canonical root + config fingerprint.
- **Snapshot**: an immutable, queryable view of the index at a point in time.
- **Segment**: immutable chunk rows produced by a sync transaction (delta data).
- **Tombstone**: a deletion marker that removes older rows from the live view.
- **Manifest**: JSON document describing how to assemble a snapshot (segments + tombstones + metadata).
- **ACTIVE_SNAPSHOT**: a tiny file holding the current published snapshot id.
- **Eligible file**: a file that is in-scope for indexing given ignore rules + safety policies.
- **path_key**: canonical repo-relative key for a file path (used for identity and filtering).
- **path_key_ci**: casefolded `path_key` used to detect collisions.

## Canonical Root And Store Identity

1) Canonical root selection
   - Canonical root MUST be the git repo root when `.git` is present (use `git rev-parse --show-toplevel`).
   - If no git repo exists, canonical root MUST be the resolved absolute path of the requested `--path`.
   - Canonical root MUST be resolved via `realpath` to eliminate symlink ambiguity.

2) Store id
   - Default store id MUST be per-worktree and MUST include:
     - a hash of the canonical root path (prevents collisions across worktrees), and
     - the config fingerprint (prevents mixed-config indexes).
   - Ignore-only changes MUST NOT change the store id; they update `ignore_fingerprint` and publish a new snapshot.
   - Shared stores MUST be opt-in only and MUST reject use when config fingerprint differs.

## Repo SSOT Config And Fingerprint Inputs

The repo SHOULD provide a tracked SSOT config so all agents use the same index-critical settings.

1) Repo config file
   - ggrep SHOULD support a repo-root config file (recommended name: `.ggrep.toml`).
   - `.ggrep.toml` SHOULD be tracked in git.
   - Repo config SHOULD define index-critical settings (models/revisions, prefixes, chunking knobs) and query
     profiles/quotas.
   - Repo config SHOULD separate index settings from query-only settings so ranking tweaks do not force reindex.
   - Repo config MUST be treated as untrusted input:
     - it MUST NOT be able to expand indexing scope outside canonical root,
     - it MUST NOT be able to bypass out-of-root enforcement, symlink policy, or ignore rules, and
     - it MUST be validated to prevent runaway resource usage (file size caps, max chunks per file, max total
       bytes per sync, etc.).

2) Ignore SSOT
   - The eligible file set MUST be derived from repo inputs:
     - `.gitignore` files under canonical root (root + nested)
     - `.ggignore` (repo root; tracked; ggrep-specific ignore rules)
   - Indexing MUST NOT depend on per-user global gitignore rules or `.git/info/exclude` because they create
     agent-to-agent drift.

3) Fingerprint composition
   - `config_fingerprint` MUST incorporate (at minimum):
     - all effective index-critical config values,
     - the content hash of `.ggrep.toml` (if present),
     - pinned embedding model identifiers (including revisions),
     - pinned grammar identifiers (including versions/checksums), and
     - a chunker algorithm version constant defined in the binary, and
     - schema/index version identifiers.
   - `ignore_fingerprint` MUST incorporate the content hash of `.gitignore` files + `.ggignore` (if present).
     - When multiple ignore files exist, hashing MUST be deterministic.
     - MUST sort ignore file paths by byte value of `path_key`, then hash
       `(path_key + NUL + file_bytes)` concatenation.

4) Index vs query fingerprint (required)
   - `config_fingerprint` is the **index fingerprint**: anything that changes what embeddings represent MUST
     change it (chunker, preprocess, model ids/revisions, schema versions).
   - A separate `query_fingerprint` SHOULD exist for ranking profiles, quotas, and output formatting knobs.
   - Ranking schema versions MUST be included in `query_fingerprint`, not `config_fingerprint`.
   - `ignore_fingerprint` tracks ignore inputs; ignore-only changes SHOULD publish a new snapshot within the same
     store rather than forcing a new store id.
   - Store identity and snapshot manifests MUST be keyed to the index fingerprint only.
   - Query responses SHOULD include both fingerprints so ranking/profile changes are traceable without reindex.

## Hard Safety Limits (Wave II Defaults)

These limits are hard upper bounds compiled into the binary and MUST NOT be exceeded by repo config.
Config MAY set lower limits.

- `MAX_FILE_SIZE_BYTES = 10_485_760` (10 MiB)
- `MAX_CHUNKS_PER_FILE = 2000`
- `MAX_BYTES_PER_SYNC = 268_435_456` (256 MiB)

## Defaults (Wave II)

Defaults MUST be stable once shipped. Config MAY set lower values (or higher values only where explicitly allowed).

- `retain_snapshots_min = 5`
- `retain_snapshots_min_age = 10m`
- `staging_ttl = 30m`
- `gc_safety_margin_ms = 120_000`
- `lease_ttl_ms = 120_000`
- `max_segments_per_snapshot = 64`
- `max_total_segments_referenced = 256`
- `max_tombstones_per_snapshot = 250_000`
- `compaction_overdue_segments = 48`
- `compaction_overdue_tombstones = 200_000`

## On-Disk Layout (Contract)

All artifacts live under `~/.ggrep/data/<store_id>/`.

Required files/directories:

```text
~/.ggrep/data/<store_id>/
  ACTIVE_SNAPSHOT                 # text: snapshot_id + "\n" (updated by atomic rename)
  index_state.json                # minimal state (head, config fingerprint, timestamps)
  snapshots/
    <snapshot_id>/
      manifest.json               # snapshot manifest (schema_versioned)
  staging/
    <txn_id>/...                  # staging output for in-progress sync (safe to delete if stale)
  locks/
    writer_lease.json             # cooperative lease + heartbeat for the single writer
    lease_guard.lock              # short-lived exclusive-create guard for lease updates
    readers.lock                  # shared lock for offline readers (GC uses exclusive lock)
```

Optional (but expected in Wave II):

```text
  logs/
    sync.log.jsonl
  cache/
    embeddings/...
  snapshots/
    <snapshot_id>/
      segment_file_index.jsonl   # path_key -> segment_id mapping for repair
```

Logging MUST be metadata-only and MUST support rotation/retention (max size + max count or max age).
Log rotation MUST be deterministic and MUST NOT affect publish behavior.

### Permissions (Single-User vs Shared Store)

- Default (single-user): store directories SHOULD be `0700`, files `0600`.
- Shared-store mode MUST be explicitly enabled and MUST use group/ACL permissions:
  - store directories `0770` with `setgid`, files `0660`, or
  - equivalent ACL inheritance on supported platforms.
- Mixed modes are not allowed: a store MUST be created as single-user or shared-store and keep that policy.
- Phase II scope: shared-store MUST be local filesystem only (same-host, same-group). Shared-store on network
  filesystems (NFS/SMB) MUST be refused; use per-worktree stores instead.
- Shared-store mode MUST perform a capability check at startup (exclusive-create, rename, read-after-write) and
  fail fast if the filesystem does not meet required semantics.
  - The capability check MUST attempt: exclusive-create (`O_EXCL`) guard acquisition, write+fsync+read-after-write,
    and atomic rename of a pointer file under the store root.
- When ACLs are used, implementations SHOULD prefer ACL inheritance over chmod that could mask/remove ACLs.

### Artifact caches (models + grammars)

Artifact caches live outside the per-store data directory and MUST be concurrency-safe:

- Models: `~/.ggrep/models/`
- Grammars: `~/.ggrep/grammars/`

Downloads MUST use per-artifact locks + download-to-temp + validation/checksum + atomic rename.

### Atomic publish

- Publishing a snapshot MUST be implemented as: write `ACTIVE_SNAPSHOT.tmp` then atomic rename to `ACTIVE_SNAPSHOT`.
- Readers MUST read `ACTIVE_SNAPSHOT` once at request start and pin that snapshot id for the lifetime of the request.
- Publish MUST be crash-durable, not just atomic:
  - fsync `manifest.json` after write
  - fsync the directory containing `ACTIVE_SNAPSHOT` after rename
  - fsync the snapshot directory after creating/updating `manifest.json`
- This durability barrier is required (`IDX.MUST.003`).
- Windows/macOS-specific atomic swap caveats are out of scope for Phase II (Linux-only); keep any future notes in an
  appendix section.

## Manifest Schema (v1)

Each snapshot MUST have `snapshots/<snapshot_id>/manifest.json` with at least:

- `schema_version` (integer, starts at 1)
- `chunk_row_schema_version` (integer, starts at 1)
- `snapshot_id` (string)
- `parent_snapshot_id` (string or null)
- `created_at` (RFC3339 string)
- `canonical_root` (string)
- `store_id` (string)
- `config_fingerprint` (string)
- `ignore_fingerprint` (string)
- `lease_epoch` (u64; fencing token from writer lease)
- `git` object:
  - `head_sha` (string or null)
  - `dirty` (bool)
  - `untracked_included` (bool)
- `segments` (array of segment references)
- `tombstones` (array of tombstone references)
- `counts` object:
  - `files_indexed` (u64)
  - `chunks_indexed` (u64)
  - `tombstones_added` (u64)
- `degraded` (bool) + `errors` array (empty if not degraded)

Segment and tombstone references MUST include integrity metadata:

- `size_bytes` (u64; total bytes for the artifact)
- `sha256` (string; hex digest of a canonical artifact hash)

Example (illustrative):

```json
{
  "schema_version": 1,
  "chunk_row_schema_version": 1,
  "snapshot_id": "01HZZ5X9T2K3R7G9FQ2A5V0J1M",
  "parent_snapshot_id": "01HZZ5WQK9P6M0C2RZ0Y0X8H4S",
  "created_at": "2026-01-01T12:00:00Z",
  "canonical_root": "/path/to/workspace",
  "store_id": "goodfarmingai__<root_hash>__<cfg_fp>",
  "config_fingerprint": "<sha256-hex>",
  "ignore_fingerprint": "<sha256-hex>",
  "lease_epoch": 42,
  "git": { "head_sha": "abc123...", "dirty": true, "untracked_included": false },
  "segments": [
    {
      "kind": "delta",
      "ref_type": "lancedb_table",
      "table": "seg_01HZZ5X9T2",
      "rows": 18234,
      "size_bytes": 123456,
      "sha256": "<sha256-hex>"
    }
  ],
  "tombstones": [
    {
      "ref_type": "jsonl",
      "path": "snapshots/01HZZ5X9T2K3R7G9FQ2A5V0J1M/tombstones.jsonl",
      "count": 42,
      "size_bytes": 2048,
      "sha256": "<sha256-hex>"
    }
  ],
  "counts": { "files_indexed": 1298, "chunks_indexed": 18234, "tombstones_added": 42 },
  "degraded": false,
  "errors": []
}
```

### Manifest is self-contained

- The manifest MUST be sufficient to query a snapshot without walking parent chains.
- `parent_snapshot_id` is informational; `segments[]` and `tombstones[]` MUST fully define the live view.

### Degraded errors schema

When `degraded=true`, the manifest `errors` array MUST include objects with:

- `code` (string)
- `message` (string)
- `path_key` (string; the file that failed to index)

## Artifact Integrity (Checksums)

- Manifest entries MUST include `sha256` + `size_bytes` for all segment/tombstone artifacts.
- Snapshot open and `ggrep health` MUST validate these checksums.
- Checksum mismatches MUST be treated as hard errors for strict publish and query.
- Queries MUST verify checksums before reading referenced artifacts (`IDX.MUST.006`).

## Schema Evolution And Store Upgrades

Schema versions are not just tags; they are compatibility contracts.

### Compatibility matrix (required)

For each artifact type, the binary MUST declare:

- minimum readable `schema_version`
- maximum readable `schema_version`
- current writable `schema_version`

Artifact types include:

- snapshot manifest
- index_state.json
- writer_lease.json
- chunk row schema (segment tables)
- query/status/health JSON schemas (see Query/Daemon contracts)

If any required artifact is outside the readable range, the CLI/daemon MUST fail fast with an
`incompatible` or `invalid_request` error that includes the offending artifact name + version.

### Upgrade workflow (required)

Wave II MUST define an explicit upgrade path (even if it is a placeholder command):

`ggrep upgrade-store` (or equivalent) MUST:

- acquire the writer lease
- validate existing artifacts (schema + integrity)
- migrate in place OR declare "reindex required"
- publish a new snapshot if migration produces new segments/metadata
- preserve the last-good snapshot if upgrade fails

### Downgrade policy (required)

- If the store schema is newer than the running binary, the daemon/CLI MUST refuse to operate and return
  a clear error. If read-only access is supported for newer schemas, it MUST be explicit (e.g. `--read-only`).
- Old snapshots SHOULD remain readable after upgrades unless a migration explicitly requires reindex.

## LanceDB Physical Strategy (Contract)

Wave II MUST choose and implement exactly one physical strategy. Default (recommended):

- **Per-segment tables** in the store's LanceDB database.
  - Each sync transaction creates a new immutable table (segment) containing chunk rows for eligible changed files.
  - Segment tables MUST be append-only after publish.
  - Compaction creates a new consolidated segment and publishes a snapshot that references it, enabling GC of old segments.
  - Compaction MUST also consolidate/prune tombstones to prevent unbounded tombstone growth (`IDX.MUST.007`).
  - Compaction MAY build new segments without holding the writer lease, but it MUST acquire the lease to publish.
    If the active snapshot changes during compaction, publish MUST abort and retry/rebase.

The manifest `segments[]` MUST fully define which tables/segments are queryable for a snapshot (no implicit "latest").

### Segment naming

- Segment table names MUST be deterministic for a given published snapshot (no random names in published references).
- Recommended naming: `seg_<snapshot_id>_<seq>` where `<seq>` is a 0-based segment sequence within the snapshot.

## Storage Engine Semantics (Concurrency + Durability)

The storage engine MUST preserve snapshot isolation under concurrent multi-process access:

- Creating a new segment MUST NOT block readers of the active snapshot.
- Readers MUST observe either the previous snapshot or the new snapshot, never partial segment state.
- Segments MUST be immutable after publish; compaction must create new segments rather than mutating old ones.

Durability barrier requirements:

- The storage engine MUST expose a commit/flush boundary for segment writes.
- The publish workflow MUST wait for this boundary before writing the manifest.
- Artifact hashes and sizes MUST be computed after the durability barrier.

If the storage engine cannot meet these properties directly, the implementation MUST stage writes in
a separate directory or database and only make them visible by manifest pointer swap.

## Chunk Row Schema (v1, SSOT)

Segment tables MUST conform to the ChunkRow schema below. This is the SSOT for physical row storage and the
primary drift prevention contract.

### Required columns

| column | type | notes |
| --- | --- | --- |
| row_id | string | deterministic row identity (see Deterministic IDs) |
| chunk_id | string | deterministic content identity (see Deterministic IDs) |
| path_key | string | canonical repo-relative path |
| path_key_ci | string | casefolded `path_key` for collision detection |
| ordinal | u32 | 0-based extraction order within the file |
| file_hash | bytes or hex string | full content hash of the source file |
| chunk_hash | bytes or hex string | full content hash of the chunk payload after preprocess |
| chunker_version | string | version string for chunker + preprocess rules |
| kind | string | `text` or `anchor` (extend by versioned enum) |
| text | string | chunk text as embedded (post-preprocess) |
| embedding | f32[] (fixed length) | dense embedding vector; dim MUST match model |

### Optional columns (recommended)

| column | type | notes |
| --- | --- | --- |
| start_line | u32 | 1-based line start (when line offsets are available) |
| end_line | u32 | 1-based line end (inclusive) |
| byte_start | u64 | byte start offset (when byte offsets are available) |
| byte_end | u64 | byte end offset (exclusive) |
| language | string | language tag or file type hint |
| mime | string | MIME type if known |
| anchors | json | structured anchor metadata |
| context_prev | string | optional preceding context |
| context_next | string | optional following context |
| colbert | bytes | optional ColBERT token matrix (quantized) |
| colbert_scale | f64 | required if `colbert` is present |
| repo_root_hash | string | optional canonical root hash for debugging |
| snapshot_id | string | optional denormalized snapshot id for debugging |
| created_at | RFC3339 string | optional row creation timestamp |

### Invariants

- `(path_key, ordinal)` MUST be unique within the live snapshot view.
- `row_id` MUST be unique within a segment and SHOULD be unique across the store.
- Every row MUST carry the `file_hash` for the bytes used to produce the row.
- `embedding` length MUST equal the configured model dimension.
- `colbert_scale` MUST be present when `colbert` is present.
- If both line and byte offsets exist, they MUST describe the same range in the source text.

### Schema versioning

The chunk row schema version MUST be tracked (e.g., in segment table metadata or manifest segment entries).
Schema conformance MUST be validated at publish time and surfaced in `ggrep health`.

If the physical store uses legacy column names, the store adapter MUST provide a one-to-one mapping to the
logical ChunkRow schema for conformance checks and query output.

## Tombstones And The Live View

### Tombstone keys

- Tombstones MUST support at minimum tombstoning by `path_key` (delete entire file).
- Modifications SHOULD tombstone by `path_key` (replace all prior chunks for that file).
- Tombstones MAY additionally support `row_id` tombstones if partial replacement is later required.

### Enforcement

- Tombstone application MUST be structural, not optional:
  - There MUST be exactly one blessed query entrypoint that produces a `SnapshotView` (segments + tombstone filter).
  - All search paths (CLI, daemon, MCP) MUST query via `SnapshotView`.
  - Lower-level store querying without tombstones MUST be private/internal and MUST NOT be used by request handlers.
- This requirement is tracked as `IDX.MUST.001`.
- Tests MUST prove: deleted files return 0 results even if older segments still contain their rows.

### Tombstone artifact format (default)

Default `ref_type=jsonl` tombstones MUST be newline-delimited JSON objects:

```json
{ "path_key": "Engine/Docs/Plan/foo.md", "reason": "delete" }
{ "path_key": "Tools/ggrep/src/sync.rs", "reason": "replace" }
```

Rules:

- `path_key` MUST be the canonical normalized key.
- `reason` MUST be one of: `delete`, `replace`, `rename_from`.

## File Eligibility (Perfect Reflection Scope)

"Perfect reflection" applies to the **eligible file set**, defined as:

- Under canonical root (passes out-of-root checks)
- Not ignored (gitignore + ggrep ignore rules + configured excludes)
- Supported extension OR explicitly included by config
- Not excluded by size/binary/encoding policy
- Readable (permissions)

The snapshot MUST reflect this eligible set exactly at publish time.

## Path Normalization And Safety

### Canonical `path_key`

`path_key` MUST be:

- repo-relative to canonical root
- normalized separators (`/`)
- no leading `./`
- no `..` segments
- UTF-8 (lossless where possible; otherwise file is ineligible by policy)

### Out-of-root enforcement

- Every candidate file MUST be resolved to a real path.
- The real path MUST satisfy `realpath.starts_with(canonical_root_realpath)`.
- Default symlink policy: index symlinks only if the target resolves inside canonical root; otherwise skip.
- Implement TOCTOU hardening for out-of-root checks:
  - On Linux, implementations MUST attempt `openat2` (or equivalent) with `RESOLVE_BENEATH` to prevent escaping the
    canonical root during path traversal.
  - If `openat2` is unavailable (`ENOSYS`/`EOPNOTSUPP`), fall back to open-then-verify: open the file first and then
    verify the opened FD resolves under root (`/proc/self/fd/<fd>`).
  - Deferred Windows note: use `GetFinalPathNameByHandle` (or Rust equivalent) for the open-then-verify step.

### Symlink loop defense

- The filesystem walker and Change Detector MUST detect symlink cycles and MUST enforce a max hop depth
  (default 32) to prevent infinite traversal.
- If a loop is detected or max depth exceeded, the path MUST be treated as ineligible and surfaced in health.

### Casefold collision detection

- Compute `path_key_ci` as a casefolded form of `path_key` for collision detection.
- If two distinct eligible files share the same `path_key_ci`, strict publish MUST fail by default (D12).
- Collisions MUST be surfaced in `ggrep health` with the colliding paths.

### Unicode normalization (portability)

- `path_key` SHOULD be normalized to Unicode NFC prior to storage and comparison.
- If NFC normalization is not available on a platform/build, the implementation MUST still be deterministic and MUST
  surface a warning in `ggrep health` that Unicode normalization is not enforced.

### Deferred: Windows path prefix normalization (informative)

- Windows final paths may include extended-length prefixes (`\\?\` or `\\?\UNC\`).
- Implementations should strip these prefixes before hashing/storing `path_key` to preserve cross-platform portability.

## Repo State Semantics (Git)

Snapshots MUST record:

- `git.head_sha` (HEAD at sync time)
- `git.dirty` (whether working tree differs from HEAD)
- `git.untracked_included`

Defaults:

- Indexing targets working tree content (not just HEAD) for eligible files.
- Untracked inclusion default SHOULD be on for per-worktree stores and MUST be configurable.
- Repos SHOULD keep non-ignored untracked files near-zero by promptly `git add`-ing or `.gitignore`-ing new files.
- Branch switches MUST be detected (head_sha change) and trigger ChangeSet recomputation.
- Shared-store publishing SHOULD exclude untracked files by default to avoid cross-user thrash.
- When shared-store publishes with dirty/untracked state (explicitly enabled), it MUST set `git.dirty=true` and
  `git.untracked_included=true` and MUST surface warnings in `status`/`health`.
- An explicit `--shared-store-include-untracked` (or config) MAY enable untracked inclusion for trusted shared
  environments; `--shared-store-clean` MAY enforce clean-only publishing.

## Change Detection (Contract)

The Change Detector is authoritative; watcher events are hints.

Default behavior is filesystem-first ChangeSet generation using watchers/metadata with periodic reconciliation.
Git diff is used for reconciliation when available; git-first ChangeSet generation is optional and MUST be
explicitly enabled.
Reconciliation cadence MUST be explicit; default SHOULD be time-based (e.g., every 5 minutes) and MAY also run
after sync idle to prevent missed watcher events from leaving the index stale.
When metadata indicates a change but content may be unchanged (mtime-only/touch), implementations SHOULD run a
fast pre-check (e.g., file size + hash of first 4096 bytes) to filter false positives before full hashing.
Full-content hashing remains required for publish candidates.

If git is present:

- Compute committed deltas using `git diff --name-status <last_head> <head>` (or merge-base strategy; see below).
- Optionally include dirty/untracked deltas using `git status --porcelain -z`.
- Git commands that emit paths MUST use `-z` and be parsed as NUL-delimited to handle
  paths with spaces/newlines safely.
- Git backends MAY use the CLI or a library implementation, but MUST be byte-safe and MUST preserve the same
  rename/modify/delete semantics and NUL-delimited path safety guarantees.
- Git command failures caused by concurrent repository operations (e.g. `.git/index.lock`) MUST be treated as
  transient: retry with bounded backoff and do not publish partial state.
- Confirm adds/modifies by hashing file bytes before indexing (prevents missed updates).
- If `last_head` is missing/unresolvable or `git diff` fails, the implementation MUST fall back to a filesystem
  scan ChangeSet (full hash confirm) before publish.
- If `last_head` is not an ancestor of `head`, the implementation MUST use a deterministic strategy:
  - Preferred: compute `merge_base(last_head, head)` and diff from merge-base to head.
  - If merge-base is missing/unresolvable, fall back to filesystem scan ChangeSet.
- Submodules and nested repos MUST have explicit policy:
  - Default: treat submodules as ineligible (do not recurse into their contents).
  - If enabled, submodule contents MUST be indexed under a distinct root namespace to avoid collisions.

If git is absent:

- Use metadata scan to find candidates (mtime/size) and confirm with full hash for publish candidates.
- Periodic reconciliation MUST still be metadata-first; a full-repo hash audit every cycle is not required.

Output MUST be:

```text
ChangeSet { add: [...], modify: [...], delete: [...], rename: [(old,new)...] }
```

## Stable File Reads (Mid-Write Safety)

Indexing MUST avoid reading partially-written files (common in multi-agent/editor save patterns).

- Implement stable reads for hashing/chunking (e.g. stat-before + read-bytes + stat-after).
- If size/mtime changes during read, retry a small bounded number of times with backoff.
- If a file disappears during sync, treat it as a delete (tombstone) if it was in-scope; otherwise ignore.
- If an eligible changed file cannot be read stably within the retry budget, strict publish MUST fail by default
  (unless `--allow-degraded` is enabled).

## Deterministic IDs And Caching

### Hashes

- `file_hash` MUST be a full-content hash of the file bytes (current implementation uses SHA-256).
- `chunk_hash` MUST be a full-content hash of the chunk payload as embedded (after preprocess).

### IDs

IDs MUST be deterministic and stable for identical inputs.

Recommended scheme (two-layer):

- `chunk_id`: content identity (does not include path) = `hex(SHA256(chunk_hash || chunker_version || kind))`
- `row_id`: row identity (includes path) = `hex(SHA256(path_key || chunk_id || ordinal))`

Notes:

- `ordinal` is the chunk's extraction order within the file, including anchors.
- Rename-only changes preserve `chunk_id` (and therefore enable embedding reuse), while `row_id` changes with `path_key`.

### Embed cache key

- `embed_config_fingerprint` MUST hash the effective embedding/index config (models, dims, prefixes, max lengths,
  chunker_version, ignore rules hash).
- Cache key MUST be `(embed_config_fingerprint, chunk_hash)`.

## Artifact Determinism (Models + Grammars)

- Grammar downloads MUST NOT use moving targets like `releases/latest` for published/indexed behavior.
- Grammar artifacts SHOULD be pinned by version and SHOULD have an expected checksum recorded (in code or repo config).
- Embedding models SHOULD be pinned by revision (commit hash or immutable identifier) where supported.
- Any change to pinned artifact identity MUST change `config_fingerprint`.

## Strict Publish: Fail vs Skip Policy (Contract)

Strict publish means: do not publish a new snapshot if any **eligible changed file** cannot be fully indexed.

Classify outcomes:

- **Hard error (blocks publish)**:
  - manifest write/publish failure
  - integrity check failure
  - casefold collision (default)
  - embed failure for eligible changed file (default)
  - schema/config fingerprint mismatch with shared store
- **Soft skip (allowed, but must be surfaced)**:
  - out-of-root excluded (policy)
  - ignored paths (policy)
  - binary/size excluded (policy)
  - unsupported encoding excluded (policy)
- **Warning (publish allowed, surfaced)**:
  - transient read failures for non-changed files (should be rare; prefer treating unreadable as ineligible)

All skips MUST be deterministic and MUST be reported via `status/health`.

## `index_state.json` Schema (v1)

`index_state.json` is minimal and MUST be rebuildable from manifests + store.
It MUST include both `config_fingerprint` (index) and `ignore_fingerprint` (ignore inputs).

```json
{
  "schema_version": 1,
  "store_id": "<store_id>",
  "canonical_root": "/abs/path",
  "config_fingerprint": "<sha256-hex>",
  "ignore_fingerprint": "<sha256-hex>",
  "active_snapshot_id": "01HZZ...",
  "git": { "last_head_sha": "abc123...", "dirty": true, "untracked_included": false },
  "last_sync_at": "2026-01-01T12:00:00Z",
  "last_result": "ok"
}
```

## `writer_lease.json` Schema (v1)

Lease file MUST be cooperative (heartbeat-based) and MUST be safe to reclaim when stale.

```json
{
  "schema_version": 1,
  "owner_id": "uuid-or-ulid",
  "pid": 1234,
  "hostname": "host",
  "started_at": "2026-01-01T12:00:00Z",
  "last_heartbeat_at": "2026-01-01T12:00:03Z",
  "lease_epoch": 42,
  "lease_ttl_ms": 120000,
  "staging_txn_id": "01HZZ..."
}
```

Rules:

- Writer MUST heartbeat at least every `lease_ttl_ms/3`.
- Lease acquisition MUST use atomic create or write+rename semantics (e.g., O_EXCL) rather than advisory locks.
- Another writer MAY steal the lease only if `now - last_heartbeat_at > lease_ttl_ms`.
- Lease stealing MUST be compare-and-swap:
  - read the current lease (`owner_id`, `last_heartbeat_at`),
  - verify staleness,
  - write a new lease to a temp file, and
  - atomically replace `writer_lease.json` (the write is atomic, the condition is enforced by the guard).
- Deferred Windows note: lease swaps should use `MoveFileEx` with
  `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH` and bounded retries on `AccessDenied`.
- All lease updates (acquire, heartbeat, steal) MUST be serialized by an exclusive-create guard
  (`locks/lease_guard.lock` or equivalent):
  - acquire guard via `O_EXCL` create with a short TTL,
  - perform the lease read/verify/update,
  - release guard immediately.
  This prevents simultaneous steal attempts from overwriting each other while keeping the guard hold time minimal.
- Writers MUST re-validate lease ownership before expensive stages (embedding batches, staging writes) and MUST
  abort cleanly if the lease is lost.
- Lease acquisition MUST increment `lease_epoch` and the writer MUST retain the acquired value for publish.
- Before pointer swap, the writer MUST re-read `writer_lease.json` and confirm:
  - `owner_id` matches, and
  - `lease_epoch` matches the value acquired at lease grant time.
- If either check fails, the publish MUST abort and MUST NOT update `ACTIVE_SNAPSHOT`.
- This preflight is required (`IDX.MUST.002`).
- The published manifest MUST include the `lease_epoch` used for publish (fencing token).
- Publish step MUST still be safe under rare double-writer races (pointer swap is the final arbiter).

## Integrity Checks (Publish Gate)

Before pointer swap, the snapshot manager MUST verify:

- manifest schema_version supported
- all referenced segments/tombstone artifacts exist
- counts are internally consistent (rows, files, tombstones)
- all eligible changed files are either:
  - present in new segments, or
  - tombstoned (for deletes), and
  - not missing due to embed/chunk failures (unless `--allow-degraded`)

## Audit (Contract)

`ggrep audit` MUST validate denormalized counts and report drift deterministically:

- `sum(segment.rows)` MUST equal `manifest.counts.chunks_indexed` (or equivalent).
- For any mismatch, audit MUST emit a structured error and SHOULD recommend repair/reindex.

## Corruption Handling And Repair (Contract)

- If `ACTIVE_SNAPSHOT` is missing or corrupt, ggrep MUST scan `snapshots/` for candidates but MUST only
  consider manifests that pass full integrity checks (parse + artifact existence + checksum validation).
  If no valid snapshot exists, ggrep MUST fail with a clear "store corrupt" error.
- `ggrep repair` MUST declare its scope:
  - If an incremental repair mapping exists, repair SHOULD rebuild only affected files.
  - If no mapping exists, repair MUST fall back to a full reindex or return a "reindex required" error.

### Incremental repair mapping (recommended)

To support targeted repair, the store SHOULD maintain a mapping from `path_key` to the most recent segment id.
If present, it MUST be updated on every publish and used by `ggrep repair` to determine which files to rebuild.
Implementations SHOULD avoid full-file rewrites of large mappings on every small sync; append-only deltas with
periodic compaction are acceptable if the lookup remains deterministic.
`ggrep repair` MUST treat the mapping as a hint: it SHOULD verify the target segment actually lacks the file or
validate the file hash before declaring repair complete. If verification fails, repair MUST fall back to a full
reindex or a deep scan.

Recommended location/format:

- `snapshots/<snapshot_id>/segment_file_index.jsonl`
  - one JSON object per line: `{ "path_key": "...", "segment_id": "seg_<snapshot_id>_<seq>" }`

## Reader Model And Cross-Process GC Safety

Snapshot retention and GC must remain safe even when multiple processes exist in a multi-agent workspace.

Wave II default is **daemon-coordinated reads**:

- When the daemon is running, all public query paths (CLI, daemon API, MCP) MUST route through the daemon so it can
  track in-flight queries and apply consistent snapshot pinning.
- Direct store queries (opening LanceDB tables directly) MUST be treated as internal-only. If an "offline" direct
  read mode exists, it MUST be explicitly requested (e.g. `--offline`) and MUST pin `ACTIVE_SNAPSHOT` at request
  start.
- GC MUST be conservative in the absence of cross-process reader leases:
  - it MUST retain artifacts for at least `query_timeout_ms + gc_safety_margin_ms` (time-based safety),
  - and it MUST NOT delete artifacts referenced by retained manifests.
- `ggrep gc` SHOULD be daemon-driven when possible; if invoked while no daemon is running, it MUST either:
  - acquire the writer lease and run in safe offline mode, or
  - refuse by default (preferred) unless `--force`.
- Offline readers MUST acquire a shared lock (`locks/readers.lock`) and GC MUST acquire an exclusive lock
  before deleting artifacts (`IDX.MUST.005`). If locks are unavailable on the platform, behavior is best-effort
  and MUST be documented in `status`.
- Offline direct reads are best-effort: if a process is paused beyond the safety margin (debugger/SIGSTOP),
  a concurrent GC MAY delete artifacts it is reading. This limitation MUST be documented and surfaced in `status`
  when `--offline` mode is used.

Optional future hardening (not required for Wave II): add reader leases (heartbeat files) so GC can be precise across
processes.

## Retention, GC, And Orphan Cleanup (Contract)

1) Retention
   - Store MUST retain at least `retain_snapshots_min` snapshots (default 5).
   - Store SHOULD retain snapshots younger than `retain_snapshots_min_age` (default 10 minutes) to avoid deleting
     snapshots still in-flight.

2) GC safety
  - GC MUST NEVER delete:
    - the active snapshot,
    - snapshots currently in use by in-flight queries (daemon can track counts), or
    - any artifacts referenced by retained manifests.
  - GC MUST acquire the writer lease before deleting artifacts.
  - This requirement is tracked as `IDX.MUST.004`.

3) Staging cleanup
   - Staging transactions older than `staging_ttl` (default 30 minutes) MAY be deleted if no active writer lease
     references them.
   - A new lease owner SHOULD clean up the previous owner's `staging_txn_id` if it is stale and not referenced by
     any retained manifest (fast cleanup before starting a new staging run).

4) Cache eviction
   - Embedding cache MUST have a max size and eviction policy (LRU recommended).

## Store Lifecycle Management (Contract)

Because store identity includes config fingerprints, multiple stores will accumulate over time (config evolution,
ignore changes, pinned artifact changes). Wave II MUST provide safe lifecycle management to avoid disk churn.

- The CLI SHOULD provide a store inventory command (e.g. `ggrep stores --json`) that enumerates stores for a
  canonical root with:
  - `store_id`, `config_fingerprint`, `canonical_root`, `created_at`, `last_used_at`, `size_bytes`, and
    `active_snapshot_id` (if available).
- Store discovery MUST be a single-level scan of `~/.ggrep/data/<store_id>` (no deep nesting or recursive walks).
- The system SHOULD update `last_used_at` on successful queries and sync publishes (low overhead; coalesced is fine).
- The system SHOULD update `last_used_at` on successful queries and sync publishes, but MUST NOT require a disk
  write on every query; coalesce updates (e.g., at most once per hour) or buffer in the daemon.
- The CLI SHOULD provide a safe store GC command (e.g. `ggrep gc --stores`) with conservative defaults:
  - only delete stores unused for N days (default 30),
  - never delete the active store for the current canonical root unless `--force`.
- Store GC MUST NOT delete artifacts referenced by any retained snapshot manifest within a store.

## CI And Test Budgets (Contract)

- Fixed-seed property-based fuzz tests MUST run in PR CI with a bounded runtime budget.
- Longer randomized soak runs MAY execute on a nightly schedule.
- A deterministic fake embedder MUST be available for unit/integration/fuzz testing so CI does not depend on model
  downloads or GPU availability.
- CI SHOULD include ignore-conformance tests (golden cases) that verify nested `.gitignore` semantics for tricky
  patterns (negation, double-star, anchored rules, precedence).
- CI SHOULD include crash-point injection tests for snapshot publish boundaries and MAY include concurrency model
  checking (e.g. loom) for snapshot pinning/publish/GC/cache interactions.
- CI SHOULD include zombie writer tests (stale writer loses lease mid-sync and aborts without corrupting staging).
