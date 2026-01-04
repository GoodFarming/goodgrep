# GGREP System Preamble (v0.1)

Status: Draft v0.1 (Phase III foundation)  
Scope: `Tools/ggrep` (goodgrep repo)  
Audience: engineers + operators + agent integrators  
Purpose: A high-fidelity description of how GGREP works *today* (internals + operator surfaces), so Phase III
usefulness work can build on a shared, precise mental model.

This document is primarily **descriptive** (“what exists”), not aspirational.
For normative correctness requirements, see:

- `Tools/ggrep/Docs/Spec/GGREP-Snapshot-Index-Contracts-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Query-Daemon-Contracts-v0.1.md`

---

## 0) One-sentence mental model

GGREP is a local semantic search system that indexes a repository into an isolated per-repo “store”, publishes
atomic **snapshots** of the index, and serves fast, concurrent, snapshot-pinned queries through a background
daemon (plus an MCP server for agents).

---

## 1) Key objects and invariants (as implemented)

**Repo / canonical root**

- The canonical root is the identity anchor for everything: ignore evaluation, store identity, path keys, and
  snapshot manifests.
- The canonical root is computed from the git repo root when possible (fallback: the provided `--path` / cwd),
  then canonicalized to a real path (symlink-resolved).

**Store**

- A store is the on-disk unit containing snapshot pointer + snapshot manifests + derived artifacts for a single
  (canonical_root × config_fingerprint) identity.
- Stores live under `~/.ggrep/data/<store_id>/`.

**Snapshot**

- A snapshot is an immutable view of a store’s index at a point in time.
- Queries pin to exactly one snapshot id for the duration of a request.
- Publish is “all-or-nothing”: either the new snapshot becomes active, or the previously-active snapshot remains.

**Segment (LanceDB table)**

- Each successful sync transaction writes a new segment table (named `seg_<snapshot_id>_<seq>`).
- A snapshot manifest references the set of segment tables that comprise the snapshot.

**Tombstones + visibility**

- Deletions and replacements are represented as tombstones (JSONL containing `path_key` entries).
- Query results are filtered through `SnapshotView::is_visible(path_key, segment_table)` to enforce tombstones
  structurally (results from old segments are hidden; results from the current segment remain visible).

**Single-writer / many-reader**

- Sync/maintenance operations acquire a writer lease (`writer_lease.json` + heartbeat).
- Queries do not take the writer lease; they are lock-free at the store level and rely on snapshot pinning.
- Offline reader locking exists as a separate mechanism (`readers.lock`) used to guard maintenance operations that
  delete artifacts (e.g. GC).

---

## 2) Repo identity and fingerprints

GGREP uses multiple hashes/fingerprints for correctness, isolation, and traceability.

### 2.1 Canonical root resolution

The canonical root is derived from:

- git repo root (when `.git` is present and resolvable), else
- the user-provided `--path`, else
- the process cwd,

then canonicalized to a real path.

Code: `Tools/ggrep/src/identity.rs` → `resolve_index_identity()` and `file::canonical_root()`.

### 2.2 Store id

The store id is a stable string derived from:

- a repo “slug” (from git metadata when available; else directory name), plus
- a truncated hash of the canonical root path, plus
- a truncated hash of the **config fingerprint**.

Code: `Tools/ggrep/src/identity.rs` → `build_store_id()`.

Implication: changing index-critical config changes the store id (new store); changing ignore rules keeps the same
store id but publishes a new snapshot.

### 2.3 Config fingerprint (index fingerprint)

The config fingerprint is the “this index is compatible” hash. It currently includes:

- `meta::INDEX_VERSION` (index format version string),
- chunker constants (max lines/chars, overlaps),
- embedding model ids (dense + ColBERT) and dimensions,
- embedding-time doc prefix and max sequence lengths,
- hard safety caps that affect index shape (file size cap, chunks/file cap, bytes/sync cap),
- repo config hash (`.ggrep.toml` content hash, if present),
- grammar URL list hash (tree-sitter wasm sources).

Code: `Tools/ggrep/src/identity.rs` → `compute_config_fingerprint_with_config()`.

### 2.4 Ignore fingerprint

The ignore fingerprint hashes the effective ignore inputs so ignore-only changes create a new snapshot without
creating a new store.

- It is computed from the ordered set of ignore files found under the canonical root.
- Ignore file discovery considers `.gitignore`, `.ggignore`, and legacy `.smignore`.
- Hash input is deterministic: sort by `path_key`, then hash `path_key + NUL + file_bytes` per ignore file.

Code: `Tools/ggrep/src/identity.rs` → `compute_ignore_fingerprint()` and
`Tools/ggrep/src/file/ignore.rs`.

### 2.5 Query fingerprint (traceability, not index identity)

Each query produces a query_fingerprint that records the effective query knobs:

- query string, mode, max results, per-file limit, rerank toggle, scope path, snippet mode label.

Code: `Tools/ggrep/src/identity.rs` → `compute_query_fingerprint()`.

This is included in JSON responses as provenance/debugging metadata; it does not affect store identity.

---

## 3) Configuration (global, repo, and environment variables)

GGREP config is loaded from:

1) `~/.ggrep/config.toml` (global, per-user), and
2) repo-root `.ggrep.toml` (repo SSOT), and
3) `GGREP_*` environment variables.

Repo config is treated as untrusted input and validated against hard caps.

Code: `Tools/ggrep/src/config.rs`:

- `Config::load_with_repo(root)` (global + repo config merge)
- `validate_repo_config()` (enforces caps)
- hard caps are constants like `MAX_FILE_SIZE_BYTES_CAP`, `MAX_CANDIDATES_CAP`, etc.

### 3.1 Directory locations

All base paths are under `~/.ggrep/` (resolved via `directories::BaseDirs` with a HOME fallback):

- config: `~/.ggrep/config.toml`
- models: `~/.ggrep/models/`
- grammars: `~/.ggrep/grammars/`
- store data: `~/.ggrep/data/<store_id>/`
- meta: `~/.ggrep/meta/<store_id>.json`
- sockets: `~/.ggrep/sockets/<store_id>.sock` (plus `.id` and `.pid` sidecars)

Code: `Tools/ggrep/src/config.rs` and `Tools/ggrep/src/usock/mod.rs`.

### 3.2 Important current knob: `fast_mode`

`fast_mode` exists today but is currently **semantically overloaded**:

- In embedding (`Embedder::encode_query`), it skips ColBERT query encoding.
- In indexing (`SyncEngine`), it builds anchor chunks and skips full chunking (no definition chunks).
- In search output, it is also used as the “include anchors” toggle.

This coupling is important to understand because it changes *index shape*, *retrieval*, and *formatting*.
It is a Phase III target to split into explicit knobs (rerank / include_anchors / index_profile).

Code:

- `Tools/ggrep/src/embed/candle.rs`
- `Tools/ggrep/src/sync.rs`
- `Tools/ggrep/src/cmd/search.rs`

---

## 4) Ignore rules and eligible files

### 4.1 Inputs

File eligibility is controlled by:

- default ignore patterns compiled into GGREP (e.g. `**/Datasets/**`, `node_modules`, `target`, etc.),
- repo `.gitignore` hierarchy,
- repo `.ggignore`,
- legacy `.smignore` (supported for backwards compatibility).

Code: `Tools/ggrep/src/file/ignore.rs`.

### 4.2 Current “Datasets are excluded” behavior

GGREP’s default ignore patterns include `**/Datasets/**` and the directory name `Datasets` in the default ignore
dir list.

This makes the default posture:

- high-signal indexing for code/docs/diagrams, and
- avoid indexing large evaluation fixtures / raw datasets by default.

Phase III may introduce **selective indexing profiles** so “Datasets included” becomes an explicit, budgeted,
operator-visible decision (see the Phase III plan/spec).

---

## 5) Indexing pipeline (sync) — from files to a published snapshot

At a high level, sync constructs a staging transaction, writes a new segment table, writes tombstones and a
segment-file index for visibility, then publishes a manifest and swaps `ACTIVE_SNAPSHOT`.

Reference diagram: `Tools/ggrep/Docs/Spec/Diagrams/ggrep-sync-index-lifecycle.mmd`.

### 5.1 File discovery and safety

Indexing reads file content from disk with safety checks:

- Out-of-root enforcement: verify after open that the resolved fd target is still under canonical root.
- Stable read retries: detect files changing while reading and retry a bounded number of times.
- File caps: skip/fail based on size and configured safety caps.

Code: `Tools/ggrep/src/sync.rs` → `open_verified()` and `read_head_hash()` and stable read logic.

### 5.2 Chunking model

For each eligible file, GGREP produces one or more chunks:

1) **Anchor chunk** (always created):
   - file metadata (path),
   - imports/exports (best-effort heuristics),
   - top comments,
   - a short “preamble” from the top of the file.
2) **Definition / content chunks** (created when `fast_mode` is false):
   - tree-sitter-based chunking for code,
   - Markdown chunking by headings (breadcrumbs recorded as context),
   - fallback line/char chunking when no definitions exist.

Code:

- Anchor: `Tools/ggrep/src/chunker/anchor.rs`
- Chunker: `Tools/ggrep/src/chunker/mod.rs`

### 5.3 Index-time preprocessing for embeddings

Before embedding a chunk, GGREP may augment the text passed to the embedding model:

- Mermaid diagrams are summarized into text edge/message lists for *embedding-time recall*.
- A `doc_prefix` can be applied for doc/diagram-like extensions at embedding-time.

The stored snippet content remains the original chunk content plus optional neighbor context; the augmentation is
for embedding input.

Code: `Tools/ggrep/src/preprocess.rs`.

### 5.4 Chunk identity and determinism

Chunk identity is deterministic and is derived from content and fixed version constants:

- `chunk_hash`: SHA-256 of the prepared-for-embedding text
- `chunk_id`: SHA-256 of `(chunk_hash + chunker_version + kind)`
- `row_id`: SHA-256 of `(path_key + chunk_id + ordinal)`

This helps stable updates and deterministic behavior across runs.

Code: `Tools/ggrep/src/sync.rs` → `build_chunk_hash()`, `build_chunk_id()`, `build_row_id()`.

### 5.5 Segment tables, tombstones, and segment-file index

Each sync writes a new segment table and publishes visibility metadata:

- Segment table name: `seg_<snapshot_id>_<seq>`.
- Tombstones file: `snapshots/<snapshot_id>/tombstones.jsonl` (JSONL lines with `path_key` and a reason).
- Segment-file index: `snapshots/<snapshot_id>/segment_file_index.jsonl` mapping `path_key -> segment_table`.

The segment-file index is how GGREP allows “tombstoned” paths to be visible again when a newer segment contains
the current version of a file.

Code:

- Table naming: `Tools/ggrep/src/snapshot/manager.rs` → `segment_table_name()`
- Segment-file index: `Tools/ggrep/src/snapshot/segment_index.rs`
- Building/updating mappings: `Tools/ggrep/src/sync.rs` (segment index + tombstone merge logic)

### 5.6 Snapshot manifest and publish

Publishing a snapshot means:

1) Create `snapshots/<snapshot_id>/manifest.json` (atomic write).
2) Verify manifest integrity against store state (rows + checksums).
3) Atomically swap `ACTIVE_SNAPSHOT` to the new snapshot id.

Crash safety:

- If `ACTIVE_SNAPSHOT` is missing or points to an invalid manifest, GGREP scans snapshot directories by
  `created_at` and selects the newest valid manifest.

Code:

- Manifest type: `Tools/ggrep/src/snapshot/manifest.rs`
- Publish + verify + fallback open: `Tools/ggrep/src/snapshot/manager.rs`
- Snapshot view construction: `Tools/ggrep/src/snapshot/view.rs`

### 5.7 Lease epoch fencing

Snapshot publish performs a “preflight” that verifies the writer lease owner + epoch (fencing token) right before
publication. This prevents a stale writer from publishing after losing the lease.

Code:

- Lease: `Tools/ggrep/src/lease.rs`
- Preflight assert: `Tools/ggrep/src/assert.rs` → `lease_epoch_preflight()`
- Publish path uses the preflight: `Tools/ggrep/src/snapshot/manager.rs` → `publish_manifest()`

---

## 6) Query pipeline — from a query string to ranked results

### 6.1 Query entrypoints

GGREP serves queries through:

- CLI direct / in-process: `ggrep search ...` (can fall back to in-process when daemon is unavailable).
- Daemon: `ggrep serve` handles IPC requests over a Unix socket.
- MCP server: `ggrep mcp` implements MCP JSON-RPC on stdin/stdout and forwards to the daemon.

### 6.2 Snapshot pinning and visibility filtering

Every query pins a snapshot:

- Daemon obtains a `SnapshotView` via `SnapshotManager::open_snapshot_view()` and pins it for the query.
- Search retrieves candidates from the segment tables referenced by the snapshot.
- Results are filtered through `snapshot.is_visible(path_key, segment_table)` to enforce tombstones.

Code:

- Snapshot open: `Tools/ggrep/src/snapshot/manager.rs`
- Tombstone filtering: `Tools/ggrep/src/snapshot/view.rs`
- Query filtering: `Tools/ggrep/src/search/mod.rs` (`snapshot.is_visible(...)`)

### 6.3 Retrieval: vector + lexical

Within each segment table, GGREP performs:

- vector ANN search over embeddings (separately for “code/doc/graph” filters),
- optional full-text search (FTS) as a lexical backstop,
- candidate dedup by `(path_key, start_line)`,
- scoring: cosine similarity on dense embedding, with optional ColBERT rerank for top candidates.

Code: `Tools/ggrep/src/store/lance.rs`:

- per-bucket filters (`code_filter`, `doc_filter`, `graph_filter`)
- FTS query
- candidate union + dedup
- cosine scoring + optional rerank

### 6.4 Ranking and intent modes

GGREP then applies:

- structural boosts (definitions get boosted; tests penalized; docs/graphs scaled by mode),
- deterministic sorting (score, secondary score, path, line, row id),
- mode-based selection quotas across Code/Docs/Graph buckets.

Code:

- structural boost: `Tools/ggrep/src/search/ranking.rs`
- mode quotas + bucket selection: `Tools/ggrep/src/search/profile.rs`
- deterministic compare: `Tools/ggrep/src/types.rs` (`cmp_results_deterministic`)

Modes (as implemented):

- `balanced`: mostly score-sorted results
- `discovery`: quotas favor breadth across code + docs + graphs
- `implementation`: favors code
- `planning`: favors docs/graphs
- `debug`: favors debugging/incident paths

### 6.5 Snippet content and caps

Search results carry:

- `path`, `start_line`, `num_lines`, `chunk_type`, `is_anchor`, `score`,
- `content` which may include:
  - the chunk’s content, plus
  - optional neighbor context (`context_prev` and `context_next`) stored in the vector table.

Output is then capped deterministically:

- `max_snippet_bytes_per_result`
- `max_total_snippet_bytes`
- `max_candidates` (pre-ranking truncate)

Code:

- snippet caps: `Tools/ggrep/src/search/mod.rs` → `apply_snippet_caps()`
- neighbor context assembly: `Tools/ggrep/src/store/lance.rs` (build `full_content`)

### 6.6 Output formats

CLI supports:

- default human output (TTY): colored, numbered results + snippet previews
- `--compact`: paths only
- `--no-snippet`: file + line only
- `--short-snippet` / `--long-snippet` / `--content`: progressive disclosure
- `--json`: structured output with meta and optional explainability

Code: `Tools/ggrep/src/cmd/search.rs` and `Tools/ggrep/src/format/*`.

---

## 7) Daemon model (serve) — lifecycle, sockets, admission, and QoS

### 7.1 Process lifecycle

The CLI can auto-spawn a daemon when needed:

- `ggrep search ...` tries to connect to an existing daemon for the store id.
- If none is compatible, it spawns `ggrep serve --path <repo_root>` as a background process and waits for the
  socket to be ready.

Code: `Tools/ggrep/src/cmd/daemon.rs`.

### 7.2 IPC endpoint + metadata sidecars

Unix sockets are created under:

- `~/.ggrep/sockets/<store_id>.sock`
- `~/.ggrep/sockets/<store_id>.id` (store id string)
- `~/.ggrep/sockets/<store_id>.pid` (daemon pid)

A path-length mitigation exists: if `<store_id>.sock` would exceed a limit, the daemon uses a short hashed stem
and may place sockets under `/tmp/ggrep-<uid>/`.

Code: `Tools/ggrep/src/usock/mod.rs` and `Tools/ggrep/src/usock/unix.rs`.

### 7.3 Handshake and compatibility

The daemon and client perform a handshake to ensure:

- protocol version compatibility,
- store id match,
- config fingerprint match.

Incompatibility triggers client-side shutdown of the old daemon and spawn of a new one.

Code: `Tools/ggrep/src/ipc.rs` and `Tools/ggrep/src/cmd/daemon.rs`.

### 7.4 Admission control and timeouts

The daemon enforces:

- `max_concurrent_queries` and `max_query_queue` (bounded waiting queue),
- per-client concurrency caps (when client_id is provided),
- per-query deadline (`query_timeout_ms`),
- payload caps (`max_request_bytes`, `max_response_bytes`).

These are observable via `ggrep status --json` and `ggrep health --json`.

Code: `Tools/ggrep/src/cmd/serve.rs`, `Tools/ggrep/src/cmd/status.rs`, `Tools/ggrep/src/cmd/health.rs`.

---

## 8) Operator and agent surfaces (what you can call)

### 8.1 Core UX

- `ggrep "<query>"`: shorthand search with defaults (balanced mode).
- `ggrep search ...`: full control over mode, output, snippet depth, sync behavior.
- `ggrep serve`: start daemon for fast searches + incremental indexing.
- `ggrep stop` / `ggrep stop-all`: stop daemon(s).

### 8.2 Maintenance and observability

- `ggrep status` / `ggrep status --json`
- `ggrep health` / `ggrep health --json`
- `ggrep audit` (drift checks between metadata and store)
- `ggrep compact` (segment compaction + tombstone pruning)
- `ggrep repair` (repair missing metadata/segments when possible)
- `ggrep gc` (snapshot/store garbage collection with safety rules)
- `ggrep list` / `ggrep stores` (inventory of indexed roots)
- `ggrep clean` (delete store data/metadata)

### 8.3 Quality regression tooling

- `ggrep eval` runs the curated query suite and emits a JSON report.
  - Dataset lives under `Datasets/ggrep/` and is currently oriented around GoodFarmingAI-style queries.

### 8.4 MCP server

`ggrep mcp` exposes:

- `search` (and deprecated alias `good_search`): returns the same JSON schema as `ggrep search --json`
- `ggrep_status`: `ggrep status --json`
- `ggrep_health`: `ggrep health --json`
- resources: `ggrep://status` and `ggrep://health`

Code: `Tools/ggrep/src/cmd/mcp.rs`.

---

## 9) GoodFarmingAI as the tuning target (current posture)

GGREP is tuned for hybrid corpora like GoodFarmingAI that contain:

- implementation code (Engine/Apps/**, etc),
- plans/specs/runbooks (Engine/Docs/**),
- diagrams (Docs/diagrams/**; Mermaid),
- scripts and operational helpers.

Today’s best practice for agents/operators in that environment is:

1) Choose an intent mode (`-d/-i/-p/-b`) to control breadth vs implementation bias.
2) Start with low leakage (`--compact` or `--no-snippet`) when unsure about exposure.
3) Use snippets when you need selection signal, then open the file at the returned line range.
4) Keep the daemon running (`ggrep serve --path <repo>`) for consistently fast searches and stable status/health.

Phase III will focus on making GGREP *not just a file finder*, but a “working slate generator” for agents (structured
packs with coverage across code + docs + diagrams, plus progressive disclosure and explicit confidence).
