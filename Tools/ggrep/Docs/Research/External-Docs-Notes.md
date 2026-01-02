# External / Official Docs Notes (Phase II pre-start)

Purpose: capture **official** upstream documentation that influences Phase II hardening assumptions (Linux-only, CPU-only).

This is not a design doc; it’s a reference bundle to avoid “unknown unknowns” mid-implementation.

## 1) Linux durability: rename + fsync (crash consistency)

### Sources
- `rename(2)` — Linux man-pages: https://man7.org/linux/man-pages/man2/rename.2.html
- `fsync(2)` — Linux man-pages: https://man7.org/linux/man-pages/man2/fsync.2.html

### Key points (from man-pages)
- `rename()` is atomic with respect to path resolution (all-or-nothing), but atomic rename **does not automatically imply crash durability**.
- `fsync(fd)` flushes file data/metadata for that file descriptor, **but it may not flush the directory entry** created/updated by `rename()` unless the directory itself is also `fsync`’d (the man page explicitly calls out the need to fsync the directory for durable file name changes).

### Implications for GGREP
- Any “pointer swap” (`ACTIVE_SNAPSHOT` rename) and any staged artifact finalization must treat **directory fsync** as part of the durability barrier (when the underlying filesystem semantics require it).
- “Durable publish” must be defined as a sequence, not a single operation.

## 1.1) Linux “open under root” semantics: `openat2(2)` resolution restrictions

### Sources
- `openat2(2)` — Linux man-pages: https://man7.org/linux/man-pages/man2/openat2.2.html
- `openat2(2)` — Arch man-pages: https://man.archlinux.org/man/openat2.2.en

### Key points (from man-pages)
- `openat2` supports a `resolve` bitmask for constraining path traversal. In particular:
  - `RESOLVE_BENEATH`: path resolution must remain beneath `dirfd` (rejects absolute symlinks and absolute paths escaping the root).
  - `RESOLVE_NO_SYMLINKS`: disallow symlink traversal for all components (stronger than `O_NOFOLLOW`, which only applies to the final component).
- The API can fail with `EAGAIN` in some cases when the kernel cannot ensure constraints during resolution; callers may need bounded retries.
- `openat2` is Linux-specific (appeared in Linux 5.6) and may require syscall usage depending on libc support.

### Implications for GGREP
- For Linux-only Phase II, `openat2` is the strongest primitive for TOCTOU-resistant “stay under canonical root” file access.
- If we don’t use `openat2`, “open-then-verify” must be treated as best-effort and tested against symlink mutation races.

## 2) Linux file locking: `flock` vs `fcntl` and network FS caveats

### Sources
- `flock(2)` — Linux man-pages: https://man7.org/linux/man-pages/man2/flock.2.html
- `fcntl(2)` record locking — Linux man-pages: https://man7.org/linux/man-pages/man2/fcntl.2.html
- `fcntl_locking(2)` — Linux man-pages: https://man7.org/linux/man-pages/man2/fcntl_locking.2.html

### Key points (relevant excerpts)
- Both `flock` and `fcntl` locks are **advisory** by default on Linux.
- **NFS details** (man-pages): over NFS, lock behavior can be emulated and locks can be lost under certain conditions; NFSv4 clients historically could lose/regain locks under disconnect scenarios. (Linux 3.12+ makes I/O fail for processes that “think” they hold a lock after disconnect, until reopen; still a non-trivial operational behavior.)
- **SMB/CIFS details** (man-pages): `flock` propagation semantics have changed across kernel versions; SMB may have mandatory-lock semantics depending on protocol/mount/server.

### Implications for GGREP
- Shared-store on **networked filesystems** (NFS/SMB) is high-risk unless explicitly constrained and tested. Advisory locks can become unreliable or “surprising”.
- If Phase II wants “no footguns”, default stance should be: shared-store requires a **strongly consistent local filesystem**, or shared-store over NFS/SMB is explicitly unsupported (or guarded behind capability checks + operator acknowledgement).
- If we do support network mounts later, we likely need an explicit compatibility matrix and stronger fencing beyond advisory locks.

## 3) Lance (storage format) commit semantics and concurrent writers

### Source
- Lance format docs (official): https://lancedb.github.io/lance/format.html
- LanceDB storage guide (official): https://www.lancedb.com/documentation/guides/storage/storage/

### Key points (from docs)
- Lance datasets use a commit-oriented model (“dataset versions”) and discuss how “commit” is accomplished depending on storage.
- The docs describe a transaction/commit approach using **atomic operations** such as “rename-if-not-exists” / “put-if-not-exists” on stores that support it, and note that on stores that don’t, **external locking is required** for concurrent writes.
- The docs warn about recovery and concurrent writer scenarios (e.g., lingering transaction files if a process crashes, and the need to guard concurrent commits).
- The LanceDB storage guide highlights that some backends (notably object stores) require an external “commit store” (e.g., DynamoDB) to enable safe concurrent writes, reinforcing that backend semantics matter.
### Implications for GGREP
- Our Phase II “durability barrier” and “single writer” assumptions align with Lance’s direction: if the underlying store can’t guarantee atomic commit primitives, we must provide external locking/fencing.
- For “segment writes”, we should treat the Lance dataset/table commit boundary as the underlying durability unit, then add GGREP’s own manifest + pointer swap barrier.

## 6) Unix domain sockets: path length limits (Linux)

### Source
- `unix(7)` — Linux man-pages: https://man7.org/linux/man-pages/man7/unix.7.html

### Key points (from man-pages)
- The `sockaddr_un.sun_path` field for AF_UNIX sockets has a small fixed maximum length (platform-dependent; commonly ~108 bytes on Linux).
- Long socket paths can fail with errors like `ENAMETOOLONG`.

### Implications for GGREP
- IPC endpoint paths must be short and deterministic (hashed subpaths under `~/.ggrep/daemon/…`), and the plan should keep a mitigation strategy for very long `store_id` values.
## 4) LanceDB (Rust API) — vector column type expectation

### Source
- LanceDB Rust docs: https://docs.rs/lancedb/latest/lancedb/

### Key points (from docs)
- The Rust API expects vector columns to be Arrow **FixedSizeList**.
### Implications for GGREP
- Our “ChunkRow schema v1” must align with the Arrow types expected by LanceDB (especially for the embedding/vector column).
## 5) Candle (CPU-only) build expectations

### Source
- Candle docs (installation): https://huggingface.github.io/candle/guide/installation.html

### Key points (from docs)
- Candle supports CUDA via feature flags (e.g., `candle-core` `cuda`); CPU-only builds are possible without enabling CUDA.
- CPU performance can be improved via optional MKL support (x86).
### Implications for GGREP
- Phase II should standardize “CPU-only” as the baseline build profile and ensure no CUDA-only dependencies are required.
- Any “optional GPU acceleration” should remain a separate feature path (out of scope for the Linux+CPU-only Phase II track).
## Notes / Follow-ups (if needed later)
- If we decide to support shared-store over NFS/SMB, we should add explicit “filesystem capability probes” and a documented support matrix.
- If we decide to rely on LanceDB for certain durability properties, we should verify (in code/tests) what “commit” guarantees on local FS and whether additional fsync is needed around tables/directories.
