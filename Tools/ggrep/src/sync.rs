//! File synchronization and indexing engine

#[cfg(test)]
use std::sync::{Mutex, OnceLock};
use std::{
   collections::{HashMap, HashSet},
   fs,
   io::Write,
   path::{Path, PathBuf},
   sync::Arc,
   time::Duration,
};

use indicatif::ProgressBar;
use serde::Serialize;
use sha2::{Digest, Sha256};
use chrono::Utc;
use uuid::Uuid;
use tokio::{io::AsyncReadExt, time};

pub use crate::types::SyncProgress;
use crate::{
   Result, Str,
   chunker::{Chunker, anchor::create_anchor_chunk},
   config,
   embed::{Embedder, HybridEmbedding},
   error::Error,
   file::{FileSystem, ResolvedPath, canonical_root, resolve_candidate},
   git,
   identity,
   preprocess,
   lease::WriterLease,
   meta::{FileHash, MetaStore},
   snapshot::{
      SnapshotCounts, SnapshotError, SnapshotGitInfo, SnapshotManifest, SnapshotSegmentRef,
      SnapshotTombstoneRef, SnapshotManager, compute_tombstone_artifact, read_segment_file_index,
      segment_table_name, write_segment_file_index,
      manifest::{CHUNK_ROW_SCHEMA_VERSION, MANIFEST_SCHEMA_VERSION},
   },
   store::LanceStore,
   types::{PreparedChunk, VectorRecord},
   util,
};

const CHUNKER_VERSION: &str = "chunker-v2";
const HEAD_HASH_BYTES: usize = 4096;
const STABLE_READ_RETRIES: usize = 3;
const STABLE_READ_BACKOFF_MS: u64 = 25;

#[derive(Debug, Clone, Serialize)]
struct TombstoneEntry {
   path_key: String,
   reason:   String,
}

#[cfg(test)]
static STABLE_READ_HOOK: OnceLock<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>> = OnceLock::new();

/// Gets file modification time (Unix seconds) and size in bytes.
async fn get_mtime_and_size(path: &Path) -> (u64, u64) {
   let Ok(metadata) = tokio::fs::metadata(path).await else {
      return (0, 0);
   };
   let mtime = metadata
      .modified()
      .ok()
      .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
      .map_or(0, |d| d.as_secs());
   (mtime, metadata.len())
}

async fn stat_mtime_and_size(path: &Path) -> Result<(u64, u64)> {
   let metadata = tokio::fs::metadata(path).await?;
   let mtime = metadata
      .modified()
      .ok()
      .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
      .map_or(0, |d| d.as_secs());
   Ok((mtime, metadata.len()))
}

#[cfg(test)]
pub(crate) fn set_stable_read_hook(hook: Option<Arc<dyn Fn() + Send + Sync>>) {
   let slot = STABLE_READ_HOOK.get_or_init(|| Mutex::new(None));
   *slot.lock().unwrap() = hook;
}

fn stable_read_hook() {
   #[cfg(test)]
   {
      if let Some(slot) = STABLE_READ_HOOK.get() {
         if let Some(hook) = &*slot.lock().unwrap() {
            hook();
         }
      }
   }
}

fn build_chunk_hash(text: &Str) -> FileHash {
   FileHash::sum(text.as_str().as_bytes())
}

fn build_chunk_id(chunk_hash: &FileHash, chunker_version: &str, kind: &str) -> String {
   let mut hasher = Sha256::new();
   hasher.update(chunk_hash.as_ref());
   hasher.update(chunker_version.as_bytes());
   hasher.update(kind.as_bytes());
   hex::encode(hasher.finalize())
}

fn build_row_id(path_key: &Path, chunk_id: &str, ordinal: u32) -> String {
   let mut hasher = Sha256::new();
   hasher.update(path_key.to_string_lossy().as_bytes());
   hasher.update(chunk_id.as_bytes());
   hasher.update(ordinal.to_le_bytes());
   hex::encode(hasher.finalize())
}

fn prepare_chunk(
   path_key: &Path,
   path_key_ci: &str,
   file_hash: FileHash,
   ordinal: u32,
   kind: &str,
   content: Str,
   start_line: u32,
   end_line: u32,
   chunk_type: Option<crate::types::ChunkType>,
   context_prev: Option<Str>,
   context_next: Option<Str>,
) -> PreparedChunk {
   let text = preprocess::prepare_for_embedding(&content, path_key);
   let chunk_hash = build_chunk_hash(&text);
   let chunk_id = build_chunk_id(&chunk_hash, CHUNKER_VERSION, kind);
   let row_id = build_row_id(path_key, &chunk_id, ordinal);

   PreparedChunk {
      row_id,
      chunk_id,
      path_key: Arc::new(path_key.to_path_buf()),
      path_key_ci: path_key_ci.to_string(),
      ordinal,
      file_hash,
      chunk_hash,
      chunker: CHUNKER_VERSION.to_string(),
      kind: kind.to_string(),
      text,
      start_line,
      end_line,
      chunk_type,
      context_prev,
      context_next,
   }
}

fn write_tombstones(path: &Path, entries: &[TombstoneEntry]) -> Result<()> {
   if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)?;
   }
   let mut file = fs::File::create(path)?;
   for entry in entries {
      let line = serde_json::to_string(entry)?;
      writeln!(file, "{line}")?;
   }
   file.sync_all()?;
   Ok(())
}

fn head_hash_from_bytes(bytes: &[u8]) -> FileHash {
   let len = bytes.len().min(HEAD_HASH_BYTES);
   FileHash::sum(&bytes[..len])
}

async fn open_verified(root: &Path, path: &Path) -> Result<tokio::fs::File> {
   let file = tokio::fs::File::open(path).await?;

   #[cfg(target_family = "unix")]
   {
      use std::os::unix::io::AsRawFd;
      let fd_path = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
      if let Ok(real) = tokio::fs::read_link(&fd_path).await {
         if !real.starts_with(root) {
            return Err(
               Error::Server {
                  op:     "open",
                  reason: format!("out-of-root path after open: {}", real.display()),
               }
               .into(),
            );
         }
      }
   }

   Ok(file)
}

async fn read_head_hash(root: &Path, path: &Path) -> Result<FileHash> {
   let mut last_err: Option<Error> = None;
   for attempt in 0..=STABLE_READ_RETRIES {
      let (pre_mtime, pre_size) = stat_mtime_and_size(path).await?;
      let mut file = open_verified(root, path).await?;
      let mut buf = vec![0u8; HEAD_HASH_BYTES];
      let n = file.read(&mut buf).await?;
      buf.truncate(n);
      stable_read_hook();
      let (post_mtime, post_size) = stat_mtime_and_size(path).await?;
      if pre_mtime == post_mtime && pre_size == post_size {
         return Ok(head_hash_from_bytes(&buf));
      }
      last_err = Some(Error::Server {
         op:     "stable_read",
         reason: format!("file changed during head read: {}", path.display()),
      });
      if attempt < STABLE_READ_RETRIES {
         time::sleep(Duration::from_millis(STABLE_READ_BACKOFF_MS)).await;
      }
   }
   Err(last_err.unwrap_or_else(|| Error::Server {
      op:     "stable_read",
      reason: format!("unstable read: {}", path.display()),
   }))
}

async fn read_file_verified(root: &Path, path: &Path) -> Result<Vec<u8>> {
   let mut last_err: Option<Error> = None;
   for attempt in 0..=STABLE_READ_RETRIES {
      let (pre_mtime, pre_size) = stat_mtime_and_size(path).await?;
      let mut file = open_verified(root, path).await?;
      let mut buf = Vec::with_capacity(pre_size.min(2 * 1024 * 1024) as usize);
      file.read_to_end(&mut buf).await?;
      stable_read_hook();
      let (post_mtime, post_size) = stat_mtime_and_size(path).await?;
      if pre_mtime == post_mtime && pre_size == post_size {
         return Ok(buf);
      }
      last_err = Some(Error::Server {
         op:     "stable_read",
         reason: format!("file changed during read: {}", path.display()),
      });
      if attempt < STABLE_READ_RETRIES {
         time::sleep(Duration::from_millis(STABLE_READ_BACKOFF_MS)).await;
      }
   }
   Err(last_err.unwrap_or_else(|| Error::Server {
      op:     "stable_read",
      reason: format!("unstable read: {}", path.display()),
   }))
}

fn is_missing_or_out_of_root(err: &Error) -> bool {
   match err {
      Error::Io(ioe) => ioe.kind() == std::io::ErrorKind::NotFound,
      Error::Server { op, .. } if *op == "open" => true,
      _ => false,
   }
}

fn normalize_files(files: Vec<ResolvedPath>) -> Result<Vec<ResolvedPath>> {
   let mut sorted = files;
   sorted.sort_by(|a, b| a.path_key.cmp(&b.path_key));

   let mut unique: HashMap<PathBuf, ResolvedPath> = HashMap::new();
   let mut collisions: HashMap<String, Vec<PathBuf>> = HashMap::new();

   for file in sorted {
      if unique.contains_key(&file.path_key) {
         continue;
      }
      collisions
         .entry(file.path_key_ci.clone())
         .or_default()
         .push(file.path_key.clone());
      unique.insert(file.path_key.clone(), file);
   }

   let mut collision_paths: Vec<String> = collisions
      .into_values()
      .filter(|paths| paths.len() > 1)
      .flatten()
      .map(|p| p.to_string_lossy().into_owned())
      .collect();
   collision_paths.sort();
   collision_paths.dedup();
   if !collision_paths.is_empty() {
      return Err(Error::PathCollision { paths: collision_paths }.into());
   }

   let mut files: Vec<ResolvedPath> = unique.into_values().collect();
   files.sort_by(|a, b| a.path_key.cmp(&b.path_key));
   Ok(files)
}

#[async_trait::async_trait]
impl<'a, F: FileSystem + Sync> ChangeDetector for FileSystemChangeDetector<'a, F> {
   async fn detect(&self, root: &Path, meta_store: &MetaStore) -> Result<ChangeSet> {
      let files: Vec<ResolvedPath> = self.file_system.get_files(root)?.collect();
      let files = normalize_files(files)?;

      let mut add = Vec::new();
      let mut modify = Vec::new();
      let mut delete = Vec::new();

      let file_set: HashSet<PathBuf> = files.iter().map(|f| f.path_key.clone()).collect();

      for file in files {
         match meta_store.get_meta(&file.path_key) {
            None => add.push(file),
            Some(meta) => {
               let (current_mtime, current_size) = get_mtime_and_size(&file.real_path).await;
               if meta.mtime != current_mtime || meta.size != current_size {
                  modify.push(file);
               } else if meta.head_hash.is_none() {
                  modify.push(file);
               } else {
                  match read_head_hash(root, &file.real_path).await {
                     Ok(head_hash) => {
                        if meta.head_hash != Some(head_hash) {
                           modify.push(file);
                        }
                     },
                     Err(e) => {
                        if is_missing_or_out_of_root(&e) {
                           modify.push(file);
                        } else {
                           return Err(e);
                        }
                     },
                  }
               }
            },
         }
      }

      for path in meta_store.all_paths() {
         if !file_set.contains(path) {
            delete.push(path.clone());
         }
      }

      Ok(ChangeSet { add, modify, delete, rename: Vec::new() })
   }
}

/// Engine for synchronizing files to the index
pub struct SyncEngine<F: FileSystem, E: Embedder> {
   file_system: F,
   chunker:     Chunker,
   embedder:    E,
   store:       Arc<LanceStore>,
}

/// Result summary from a sync operation
#[derive(Debug, Clone)]
pub struct SyncResult {
   pub processed: usize,
   pub indexed:   usize,
   pub skipped:   usize,
   pub deleted:   usize,
}

#[derive(Debug, Clone, Copy)]
pub struct SyncOptions {
   pub allow_degraded:     bool,
   pub embed_max_retries:  usize,
   pub embed_backoff_ms:   u64,
}

impl Default for SyncOptions {
   fn default() -> Self {
      Self { allow_degraded: false, embed_max_retries: 1, embed_backoff_ms: 100 }
   }
}

/// Change set describing file mutations between syncs.
#[derive(Debug, Clone, Default)]
pub struct ChangeSet {
   pub add:    Vec<ResolvedPath>,
   pub modify: Vec<ResolvedPath>,
   pub delete: Vec<PathBuf>,
   pub rename: Vec<(PathBuf, PathBuf)>,
}

impl ChangeSet {
   pub fn is_empty(&self) -> bool {
      self.add.is_empty()
         && self.modify.is_empty()
         && self.delete.is_empty()
         && self.rename.is_empty()
   }
}

struct PendingEmbed {
   path_key:  PathBuf,
   hash:      FileHash,
   mtime:     u64,
   size:      u64,
   head_hash: FileHash,
   chunks:    Vec<PreparedChunk>,
}

#[derive(Debug, Default)]
struct EmbedBatchOutcome {
   indexed:       usize,
   indexed_paths: Vec<String>,
   errors:        Vec<SnapshotError>,
}

/// Trait for detecting file changes between syncs.
#[async_trait::async_trait]
pub trait ChangeDetector {
   async fn detect(&self, root: &Path, meta_store: &MetaStore) -> Result<ChangeSet>;
}

pub struct FileSystemChangeDetector<'a, F: FileSystem> {
   file_system: &'a F,
}

impl<'a, F: FileSystem> FileSystemChangeDetector<'a, F> {
   pub const fn new(file_system: &'a F) -> Self {
      Self { file_system }
   }
}

/// Trait for receiving sync progress updates
pub trait SyncProgressCallback: Send {
   fn progress(&mut self, progress: SyncProgress);
}

impl<F: FnMut(SyncProgress) + Send> SyncProgressCallback for F {
   fn progress(&mut self, progress: SyncProgress) {
      self(progress);
   }
}

impl SyncProgressCallback for () {
   fn progress(&mut self, _progress: SyncProgress) {}
}

impl SyncProgressCallback for ProgressBar {
   fn progress(&mut self, progress: SyncProgress) {
      self.update(|state| {
         state.set_len(progress.total as u64);
         state.set_pos(progress.processed as u64);
      });
      if let Some(file) = &progress.current_file {
         let short = file.rsplit('/').next().unwrap_or(&**file);
         self.set_message(short.to_string());
      }
   }
}

impl<F, E> SyncEngine<F, E>
where
   F: FileSystem + Sync,
   E: Embedder + Send + Sync,
{
   pub fn new(
      file_system: F,
      chunker: Chunker,
      embedder: E,
      store: Arc<LanceStore>,
   ) -> Self {
      Self { file_system, chunker, embedder, store }
   }

   /// Performs an initial sync of files to the index
   pub async fn initial_sync(
      &self,
      store_id: &str,
      root: &Path,
      changeset: Option<ChangeSet>,
      dry_run: bool,
      callback: &mut dyn SyncProgressCallback,
   ) -> Result<SyncResult> {
      self
         .initial_sync_with_options(store_id, root, changeset, dry_run, SyncOptions::default(), callback)
         .await
   }

   pub async fn initial_sync_with_options(
      &self,
      store_id: &str,
      root: &Path,
      changeset: Option<ChangeSet>,
      dry_run: bool,
      options: SyncOptions,
      callback: &mut dyn SyncProgressCallback,
   ) -> Result<SyncResult> {
      const SAVE_INTERVAL: usize = 25;

      let sync_start = std::time::Instant::now();
      let root_real = canonical_root(root);
      config::init_for_root(&root_real);
      let lease = WriterLease::acquire(store_id).await?;
      let cfg = config::get();

      if cfg.max_store_bytes > 0 {
         let store_path = config::data_dir().join(store_id);
         let store_bytes = util::get_dir_size(&store_path).unwrap_or(0);
         if store_bytes > cfg.max_store_bytes {
            return Err(
               Error::Server {
                  op:     "sync",
                  reason: format!("store over budget ({} > {})", store_bytes, cfg.max_store_bytes),
               }
               .into(),
            );
         }
      }
      if cfg.max_cache_bytes > 0 {
         let cache_bytes =
            util::get_dir_size(config::model_dir()).unwrap_or(0)
            + util::get_dir_size(config::grammar_dir()).unwrap_or(0);
         if cache_bytes > cfg.max_cache_bytes {
            return Err(
               Error::Server {
                  op:     "sync",
                  reason: format!("cache over budget ({} > {})", cache_bytes, cfg.max_cache_bytes),
               }
               .into(),
            );
         }
      }
      if cfg.max_log_bytes > 0 {
         let log_dir = config::base_dir().join("logs");
         let log_bytes = util::get_dir_size(&log_dir).unwrap_or(0);
         if log_bytes > cfg.max_log_bytes {
            return Err(
               Error::Server {
                  op:     "sync",
                  reason: format!("logs over budget ({} > {})", log_bytes, cfg.max_log_bytes),
               }
               .into(),
            );
         }
      }
      let mut meta_store = MetaStore::load(store_id)?;
      let model_changed = meta_store.model_mismatch();
      let index_changed = meta_store.index_mismatch();
      let file_batch_size = config::get().sync_file_batch_size.max(1);
      let fast_mode = config::get().fast_mode;
      let max_file_size = config::get().effective_max_file_size_bytes();
      let max_chunks_per_file = config::get().effective_max_chunks_per_file();
      let max_bytes_per_sync = config::get().effective_max_bytes_per_sync();
      let allow_degraded = options.allow_degraded;

      let mut degraded_errors: Vec<SnapshotError> = Vec::new();
      let mut degraded_paths: HashSet<String> = HashSet::new();

      if (model_changed || index_changed) && !dry_run {
         self.store.delete_store(store_id).await?;
         meta_store.reset_for_signature_change();
      }

      // If lance store is empty but meta_store has entries for this root,
      // clear the stale metadata (data was deleted externally)
      if !dry_run && self.store.is_empty(store_id).await? {
         meta_store.clear_all();
      }

      meta_store.normalize_paths(&root_real);
      let fingerprints = identity::compute_fingerprints(&root_real)?;
      meta_store.set_fingerprints(
         fingerprints.config_fingerprint.clone(),
         fingerprints.ignore_fingerprint.clone(),
      );

      let snapshot_manager = SnapshotManager::new(
         Arc::clone(&self.store),
         store_id.to_string(),
         fingerprints.config_fingerprint.clone(),
         fingerprints.ignore_fingerprint.clone(),
      );
      snapshot_manager.cleanup_staging()?;

      let mut effective_changeset = if let Some(changeset) = changeset {
         changeset
      } else {
         let detector = FileSystemChangeDetector::new(&self.file_system);
         detector.detect(&root_real, &meta_store).await?
      };

      if effective_changeset.is_empty() {
         if !dry_run {
            if snapshot_manager.read_active_snapshot_id()?.is_none() {
               let snapshot_id = Uuid::new_v4().to_string();
               let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
               let manifest = SnapshotManifest {
                  schema_version: MANIFEST_SCHEMA_VERSION,
                  chunk_row_schema_version: CHUNK_ROW_SCHEMA_VERSION,
                  snapshot_id: snapshot_id.clone(),
                  parent_snapshot_id: None,
                  created_at: created_at.clone(),
                  canonical_root: root_real.to_string_lossy().into_owned(),
                  store_id: store_id.to_string(),
                  config_fingerprint: fingerprints.config_fingerprint.clone(),
                  ignore_fingerprint: fingerprints.ignore_fingerprint.clone(),
                  lease_epoch: lease.lease_epoch(),
                  git: SnapshotGitInfo {
                     head_sha: git::get_head_sha(&root_real),
                     dirty: git::is_dirty(&root_real).unwrap_or(false),
                     untracked_included: true,
                  },
                  segments: Vec::new(),
                  tombstones: Vec::new(),
                  counts: SnapshotCounts {
                     files_indexed: 0,
                     chunks_indexed: 0,
                     tombstones_added: 0,
                  },
                  degraded: false,
                  errors: Vec::new(),
               };

               snapshot_manager
                  .publish_manifest(&manifest, lease.owner_id(), lease.lease_epoch())
                  .await?;
               meta_store.set_snapshot_status(
                  manifest.snapshot_id.clone(),
                  manifest.created_at.clone(),
                  false,
               );
            }
            let duration_ms = sync_start.elapsed().as_millis() as u64;
            meta_store.record_sync("ok", duration_ms);
            meta_store.save()?;
         }
         return Ok(SyncResult { processed: 0, indexed: 0, skipped: 0, deleted: 0 });
      }

      let snapshot_id = Uuid::new_v4().to_string();
      let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
      let parent_snapshot_id = snapshot_manager.read_active_snapshot_id()?;
      let segment_table = segment_table_name(&snapshot_id, 0);
      let staging_txn_id = Uuid::new_v4().to_string();
      if !dry_run {
         lease
            .set_staging_txn_id(Some(staging_txn_id.clone()))
            .await?;
         let _staging_dir = snapshot_manager.create_staging(&staging_txn_id)?;
      }

      let mut processed = 0;
      let mut indexed = 0;
      let mut skipped = 0;
      let mut bytes_processed: u64 = 0;
      let mut tombstones: Vec<TombstoneEntry> = Vec::new();
      let mut tombstone_keys: HashSet<String> = HashSet::new();
      let mut push_tombstone = |path: &Path, reason: &str| {
         let key = path.to_string_lossy().into_owned();
         if tombstone_keys.insert(key.clone()) {
            tombstones.push(TombstoneEntry { path_key: key, reason: reason.to_string() });
         }
      };
      let mut indexed_paths: HashSet<String> = HashSet::new();
      let mut replace_candidates: HashSet<String> = HashSet::new();
      fn record_degraded(
         degraded_paths: &mut HashSet<String>,
         degraded_errors: &mut Vec<SnapshotError>,
         path_key: &Path,
         code: &str,
         message: String,
      ) {
         let key = path_key.to_string_lossy().into_owned();
         if degraded_paths.insert(key.clone()) {
            degraded_errors.push(SnapshotError {
               code: code.to_string(),
               message,
               path_key: key,
            });
         }
      }

      let rename_pairs = std::mem::take(&mut effective_changeset.rename);
      let rename_from: HashSet<PathBuf> =
         rename_pairs.iter().map(|(from, _)| from.clone()).collect();
      let mut deleted_paths = std::mem::take(&mut effective_changeset.delete);
      for (from, _) in &rename_pairs {
         deleted_paths.push(from.clone());
      }
      deleted_paths.sort();
      deleted_paths.dedup();

      if !deleted_paths.is_empty() {
         for path in &deleted_paths {
            let reason = if rename_from.contains(path) {
               "rename_from"
            } else {
               "delete"
            };
            push_tombstone(path, reason);
            if !dry_run {
               meta_store.remove(path);
            }
         }
      }

      let mut deleted_count = deleted_paths.len();

      let mut candidates = Vec::new();
      candidates.append(&mut effective_changeset.add);
      candidates.append(&mut effective_changeset.modify);

      let mut candidate_keys: HashSet<PathBuf> =
         candidates.iter().map(|c| c.path_key.clone()).collect();
      for (_, to) in rename_pairs {
         if candidate_keys.contains(&to) {
            continue;
         }
         let candidate_path = root_real.join(&to);
         match resolve_candidate(&root_real, &candidate_path)? {
            Some(resolved) => {
               candidate_keys.insert(resolved.path_key.clone());
               candidates.push(resolved);
            },
            None => {
               tracing::warn!("rename target skipped (unresolvable): {}", candidate_path.display());
            },
         }
      }

      let files = normalize_files(candidates)?;

      let total = files.len();
      let mut embed_queue: Vec<PendingEmbed> = Vec::with_capacity(file_batch_size);
      let mut since_save = 0usize;

      for file in files {
         processed += 1;

         let (current_mtime, current_size) = get_mtime_and_size(&file.real_path).await;
         if current_size > max_file_size {
            skipped += 1;
            if !dry_run {
               push_tombstone(&file.path_key, "delete");
               meta_store.remove(&file.path_key);
               deleted_count += 1;
            }
            continue;
         }

         let mut skip = false;
         if let Some(meta) = meta_store.get_meta(&file.path_key)
            && meta.mtime == current_mtime
            && meta.size == current_size
         {
            if let Some(stored_head) = &meta.head_hash {
               match read_head_hash(&root_real, &file.real_path).await {
                  Ok(current_head) => {
                     if &current_head == stored_head {
                        skip = true;
                     }
                  },
                  Err(e) => {
                     if !is_missing_or_out_of_root(&e) {
                        tracing::warn!(
                           "head hash precheck failed for {}: {}",
                           file.real_path.display(),
                           e
                        );
                     }
                  },
               }
            }
         }

         if skip {
            skipped += 1;
            if processed % 100 == 0 {
               callback.progress(SyncProgress {
                  processed,
                  indexed,
                  total,
                  current_file: Some("Scanning files...".into()),
               });
            }
            continue;
         }

         if bytes_processed.saturating_add(current_size) > max_bytes_per_sync {
            return Err(
               Error::Server {
                  op:     "sync",
                  reason: format!(
                     "sync byte cap exceeded (cap={}, next_file_size={})",
                     max_bytes_per_sync, current_size
                  ),
               }
               .into(),
            );
         }
         bytes_processed = bytes_processed.saturating_add(current_size);

         let content = match read_file_verified(&root_real, &file.real_path).await {
            Ok(c) => c,
            Err(e) => {
               let should_delete = match &e {
                  Error::Io(ioe) => ioe.kind() == std::io::ErrorKind::NotFound,
                  Error::Server { op, .. } if *op == "open" => true,
                  _ => false,
               };

               if should_delete {
                  if !dry_run {
                     push_tombstone(&file.path_key, "delete");
                     meta_store.remove(&file.path_key);
                  }
                  deleted_count += 1;
                  continue;
               }

               if allow_degraded {
                  let code = if matches!(e, Error::Server { op, .. } if op == "stable_read") {
                     "stable_read_failed"
                  } else {
                     "read_failed"
                  };
                  record_degraded(
                     &mut degraded_paths,
                     &mut degraded_errors,
                     &file.path_key,
                     code,
                     format!("failed to read {}: {e}", file.real_path.display()),
                  );
                  skipped += 1;
                  continue;
               }

               if matches!(e, Error::Server { op, .. } if op == "stable_read") {
                  return Err(e);
               }

               return Err(
                  Error::Server {
                     op:     "read",
                     reason: format!("failed to read {}: {e}", file.real_path.display()),
                  }
                  .into(),
               );
            },
         };

         if content.is_empty() {
            skipped += 1;
            if !dry_run {
               let hash = FileHash::sum(&content);
               let head_hash = head_hash_from_bytes(&content);
               meta_store.set_meta(
                  file.path_key.clone(),
                  hash,
                  current_mtime,
                  current_size,
                  head_hash,
               );
            }
            continue;
         }

         let hash = FileHash::sum(&content);
         let size = content.len() as u64;
         let head_hash = head_hash_from_bytes(&content);
         let existing_hash = meta_store.get_hash(file.path_key.as_path());

         // Content unchanged but mtime differs; update stored mtime so future
         // syncs can skip the file without hashing it again.
         if existing_hash == Some(hash) {
            skipped += 1;
            if !dry_run {
               meta_store.set_meta(file.path_key.clone(), hash, current_mtime, size, head_hash);
               since_save += 1;
               if since_save >= SAVE_INTERVAL {
                  meta_store.save()?;
                  since_save = 0;
               }
            }
            continue;
         }

         if dry_run {
            indexed += 1;
            continue;
         }

         if existing_hash.is_some() {
            replace_candidates.insert(file.path_key.to_string_lossy().into_owned());
         }

         let content_str = Str::from_utf8_lossy(&content);
         let path_key_ci = file.path_key_ci.clone();
         let anchor_chunk = create_anchor_chunk(&content_str, &file.path_key);

         let mut prepared_chunks = Vec::new();
         prepared_chunks.push(prepare_chunk(
            &file.path_key,
            &path_key_ci,
            hash,
            0,
            "anchor",
            anchor_chunk.content,
            anchor_chunk.start_line as u32,
            anchor_chunk.end_line as u32,
            anchor_chunk.chunk_type,
            None,
            None,
         ));

         if !fast_mode {
            let chunks = match self.chunker.chunk(&content_str, &file.real_path).await {
               Ok(chunks) => chunks,
               Err(e) => {
                  if allow_degraded {
                  record_degraded(
                     &mut degraded_paths,
                     &mut degraded_errors,
                     &file.path_key,
                     "chunk_failed",
                     format!("failed to chunk {}: {e}", file.real_path.display()),
                  );
                     skipped += 1;
                     continue;
                  }
                  return Err(
                     Error::Server {
                        op:     "chunk",
                        reason: format!("failed to chunk {}: {e}", file.real_path.display()),
                     }
                     .into(),
                  );
               },
            };

            let total_chunks = chunks.len().saturating_add(1);
            if total_chunks > max_chunks_per_file {
               if allow_degraded {
                  record_degraded(
                     &mut degraded_paths,
                     &mut degraded_errors,
                     &file.path_key,
                     "chunk_cap_exceeded",
                     format!(
                        "chunk cap exceeded for {} (chunks={}, cap={})",
                        file.real_path.display(),
                        total_chunks,
                        max_chunks_per_file
                     ),
                  );
                  skipped += 1;
                  continue;
               }
               return Err(
                  Error::Server {
                     op:     "chunk",
                     reason: format!(
                        "chunk cap exceeded for {} (chunks={}, cap={})",
                        file.real_path.display(),
                        total_chunks,
                        max_chunks_per_file
                     ),
                  }
                  .into(),
               );
            }

            for (idx, chunk) in chunks.iter().enumerate() {
               let context_prev: Option<Str> = if idx > 0 {
                  Some(chunks[idx - 1].content.clone())
               } else {
                  None
               };

               let context_next: Option<Str> = if idx < chunks.len() - 1 {
                  Some(chunks[idx + 1].content.clone())
               } else {
                  None
               };

               let prepared = prepare_chunk(
                  &file.path_key,
                  &path_key_ci,
                  hash,
                  idx as u32 + 1,
                  "text",
                  chunk.content.clone(),
                  chunk.start_line as u32,
                  chunk.end_line as u32,
                  chunk.chunk_type,
                  context_prev,
                  context_next,
               );
               prepared_chunks.push(prepared);
            }
         }

         embed_queue.push(PendingEmbed {
            path_key: file.path_key,
            hash,
            mtime: current_mtime,
            size,
            head_hash,
            chunks: prepared_chunks,
         });

         if embed_queue.len() >= file_batch_size {
            callback.progress(SyncProgress {
               processed,
               indexed,
               total,
               current_file: Some(
                  format!("Embedding batch ({} files)...", embed_queue.len()).into(),
               ),
            });

            let batch = std::mem::take(&mut embed_queue);
            let batch_outcome = self
               .process_embed_batch(store_id, &segment_table, batch, &mut meta_store, options)
               .await?;
            indexed += batch_outcome.indexed;
            since_save += batch_outcome.indexed;
            for path in batch_outcome.indexed_paths {
               if replace_candidates.remove(&path) {
                  push_tombstone(Path::new(&path), "replace");
               }
               indexed_paths.insert(path);
            }
            let batch_failed = batch_outcome.errors.len();
            for err in batch_outcome.errors {
               if degraded_paths.insert(err.path_key.clone()) {
                  degraded_errors.push(err);
               }
            }
            skipped += batch_failed;

            if since_save >= SAVE_INTERVAL {
               meta_store.save()?;
               since_save = 0;
            }
         }

         callback.progress(SyncProgress { processed, indexed, total, current_file: None });
      }

      if !dry_run && !embed_queue.is_empty() {
         callback.progress(SyncProgress {
            processed,
            indexed,
            total,
            current_file: Some(
               format!("Embedding final batch ({} files)...", embed_queue.len()).into(),
            ),
         });

         let batch = std::mem::take(&mut embed_queue);
         let batch_outcome = self
            .process_embed_batch(store_id, &segment_table, batch, &mut meta_store, options)
            .await?;
         indexed += batch_outcome.indexed;
         for path in batch_outcome.indexed_paths {
            if replace_candidates.remove(&path) {
               push_tombstone(Path::new(&path), "replace");
            }
            indexed_paths.insert(path);
         }
         let batch_failed = batch_outcome.errors.len();
         for err in batch_outcome.errors {
            if degraded_paths.insert(err.path_key.clone()) {
               degraded_errors.push(err);
            }
         }
         skipped += batch_failed;
      }

      if !degraded_errors.is_empty() && !allow_degraded {
         if !dry_run {
            if indexed > 0 {
               let _ = self.store.drop_table(store_id, &segment_table).await;
            }
            let _ = lease.set_staging_txn_id(None).await;
            let _ = fs::remove_dir_all(snapshot_manager.staging_path(&staging_txn_id));
         }
         return Err(
            Error::Server {
               op:     "sync",
               reason: format!("failed to index {} file(s)", degraded_errors.len()),
            }
            .into(),
         );
      }

      if !dry_run {
         callback.progress(SyncProgress {
            processed,
            indexed,
            total,
            current_file: Some("Creating indexes...".into()),
         });

         if indexed > 0 {
            self.store.create_fts_index(store_id, &segment_table).await?;
            self.store.create_vector_index(store_id, &segment_table).await?;
         }

         let mut segments: Vec<SnapshotSegmentRef> = Vec::new();
         let mut tombstone_refs: Vec<SnapshotTombstoneRef> = Vec::new();

         if let Some(parent_id) = parent_snapshot_id.as_deref() {
            let parent_manifest = SnapshotManifest::load(&snapshot_manager.manifest_path(parent_id))?;
            snapshot_manager.verify_manifest(&parent_manifest).await?;
            segments.extend(parent_manifest.segments);
            tombstone_refs.extend(parent_manifest.tombstones);
         }

         if indexed > 0 {
            let metadata = self.store.segment_metadata(store_id, &segment_table).await?;
            segments.push(SnapshotSegmentRef {
               kind: "delta".to_string(),
               ref_type: "lancedb_table".to_string(),
               table: segment_table.clone(),
               rows: metadata.rows,
               size_bytes: metadata.size_bytes,
               sha256: metadata.sha256,
            });
         }

         if !tombstones.is_empty() {
            let staging_path = snapshot_manager
               .staging_path(&staging_txn_id)
               .join("tombstones.jsonl");
            write_tombstones(&staging_path, &tombstones)?;
            let snapshot_dir = snapshot_manager.snapshot_dir(&snapshot_id);
            fs::create_dir_all(&snapshot_dir)?;
            let final_path = snapshot_dir.join("tombstones.jsonl");
            fs::rename(&staging_path, &final_path)?;
            util::fsync_dir(&snapshot_dir)?;
            let (size_bytes, sha256, count) = compute_tombstone_artifact(&final_path)?;
            tombstone_refs.push(SnapshotTombstoneRef {
               ref_type: "jsonl".to_string(),
               path: format!("snapshots/{snapshot_id}/tombstones.jsonl"),
               count,
               size_bytes,
               sha256,
            });
         }

         let mut segment_index: HashMap<String, String> = HashMap::new();
         if let Some(parent_id) = parent_snapshot_id.as_deref() {
            let parent_index = snapshot_manager
               .snapshot_dir(parent_id)
               .join("segment_file_index.jsonl");
            if parent_index.exists() {
               segment_index = read_segment_file_index(&parent_index)?;
            } else {
               tracing::warn!("segment index missing for parent snapshot {parent_id}");
            }
         }

         if segment_index.is_empty() && parent_snapshot_id.is_none() && indexed > 0 {
            for path in meta_store.all_paths() {
               segment_index
                  .insert(path.to_string_lossy().into_owned(), segment_table.clone());
            }
         } else {
            for key in tombstone_keys.iter() {
               segment_index.remove(key);
            }
            if indexed > 0 {
               for key in indexed_paths.iter() {
                  segment_index.insert(key.clone(), segment_table.clone());
               }
            }
         }

         if !segment_index.is_empty() {
            let staging_path = snapshot_manager
               .staging_path(&staging_txn_id)
               .join("segment_file_index.jsonl");
            write_segment_file_index(&staging_path, &segment_index)?;
            let snapshot_dir = snapshot_manager.snapshot_dir(&snapshot_id);
            fs::create_dir_all(&snapshot_dir)?;
            let final_path = snapshot_dir.join("segment_file_index.jsonl");
            fs::rename(&staging_path, &final_path)?;
            util::fsync_dir(&snapshot_dir)?;
         }

         if cfg.max_segments_per_snapshot > 0
            && segments.len() > cfg.max_segments_per_snapshot
         {
            return Err(
               Error::Server {
                  op: "publish",
                  reason: format!(
                     "segment cap exceeded ({} > {})",
                     segments.len(),
                     cfg.max_segments_per_snapshot
                  ),
               }
               .into(),
            );
         }

         if cfg.max_total_segments_referenced > 0
            && segments.len() > cfg.max_total_segments_referenced
         {
            return Err(
               Error::Server {
                  op: "publish",
                  reason: format!(
                     "total segment cap exceeded ({} > {})",
                     segments.len(),
                     cfg.max_total_segments_referenced
                  ),
               }
               .into(),
            );
         }

         let total_tombstones: u64 = tombstone_refs.iter().map(|t| t.count).sum();
         if cfg.max_tombstones_per_snapshot > 0
            && total_tombstones > cfg.max_tombstones_per_snapshot as u64
         {
            return Err(
               Error::Server {
                  op: "publish",
                  reason: format!(
                     "tombstone cap exceeded ({} > {})",
                     total_tombstones,
                     cfg.max_tombstones_per_snapshot
                  ),
               }
               .into(),
            );
         }

         let chunks_indexed: u64 = segments.iter().map(|s| s.rows).sum();
         let files_indexed = meta_store.all_paths().count() as u64;

         let degraded = allow_degraded && !degraded_errors.is_empty();
         let errors = if degraded { degraded_errors } else { Vec::new() };

         let manifest = SnapshotManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            chunk_row_schema_version: CHUNK_ROW_SCHEMA_VERSION,
            snapshot_id: snapshot_id.clone(),
            parent_snapshot_id,
            created_at: created_at.clone(),
            canonical_root: root_real.to_string_lossy().into_owned(),
            store_id: store_id.to_string(),
            config_fingerprint: fingerprints.config_fingerprint.clone(),
            ignore_fingerprint: fingerprints.ignore_fingerprint.clone(),
            lease_epoch: lease.lease_epoch(),
            git: SnapshotGitInfo {
               head_sha: git::get_head_sha(&root_real),
               dirty: git::is_dirty(&root_real).unwrap_or(false),
               untracked_included: true,
            },
            segments,
            tombstones: tombstone_refs,
            counts: SnapshotCounts {
               files_indexed,
               chunks_indexed,
               tombstones_added: total_tombstones,
            },
            degraded,
            errors,
         };

         snapshot_manager
            .publish_manifest(&manifest, lease.owner_id(), lease.lease_epoch())
            .await?;

         let duration_ms = sync_start.elapsed().as_millis() as u64;
         meta_store.set_snapshot_status(
            manifest.snapshot_id.clone(),
            manifest.created_at.clone(),
            manifest.degraded,
         );
         meta_store.record_sync(if manifest.degraded { "degraded" } else { "ok" }, duration_ms);
         meta_store.save()?;
         lease.set_staging_txn_id(None).await?;
         let _ = fs::remove_dir_all(snapshot_manager.staging_path(&staging_txn_id));
      }

      callback.progress(SyncProgress { processed: total, indexed, total, current_file: None });

      Ok(SyncResult { processed, indexed, skipped, deleted: deleted_count })
   }

   async fn embed_with_retry(
      &self,
      texts: &[Str],
      options: SyncOptions,
   ) -> Result<Vec<HybridEmbedding>> {
      let mut attempt = 0usize;
      loop {
         match self.embedder.compute_hybrid(texts).await {
            Ok(result) => return Ok(result),
            Err(err) => {
               if attempt >= options.embed_max_retries {
                  return Err(err);
               }
               let shift = attempt as u32;
               let factor = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
               let backoff = options.embed_backoff_ms.saturating_mul(factor);
               if backoff > 0 {
                  time::sleep(Duration::from_millis(backoff)).await;
               }
               attempt = attempt.saturating_add(1);
            },
         }
      }
   }

   async fn process_embed_batch(
      &self,
      store_id: &str,
      table_name: &str,
      batch: Vec<PendingEmbed>,
      meta_store: &mut MetaStore,
      options: SyncOptions,
   ) -> Result<EmbedBatchOutcome> {
      let mut outcome = EmbedBatchOutcome::default();
      if batch.is_empty() {
         return Ok(outcome);
      }

      let all_chunks: Vec<PreparedChunk> = batch
         .iter()
         .flat_map(|entry| entry.chunks.iter().cloned())
         .collect();

      if all_chunks.is_empty() {
         return Ok(outcome);
      }

      let texts: Vec<Str> = all_chunks.iter().map(|c| c.text.clone()).collect();

      match self.embed_with_retry(&texts, options).await {
         Ok(embeddings) => {
            if embeddings.len() != all_chunks.len() {
               return Err(
                  Error::Server {
                     op:     "embed",
                     reason: "embedding count mismatch for batch".to_string(),
                  }
                  .into(),
               );
            }
            let records: Vec<VectorRecord> = all_chunks
               .into_iter()
               .zip(embeddings.into_iter())
               .map(|(chunk, embedding)| VectorRecord {
                  row_id:        chunk.row_id,
                  chunk_id:      chunk.chunk_id,
                  path_key:      chunk.path_key,
                  path_key_ci:   chunk.path_key_ci,
                  ordinal:       chunk.ordinal,
                  file_hash:     chunk.file_hash,
                  chunk_hash:    chunk.chunk_hash,
                  chunker:       chunk.chunker,
                  kind:          chunk.kind,
                  text:          chunk.text,
                  start_line:    chunk.start_line,
                  end_line:      chunk.end_line,
                  chunk_type:    chunk.chunk_type,
                  context_prev:  chunk.context_prev,
                  context_next:  chunk.context_next,
                  vector:        embedding.dense,
                  colbert:       embedding.colbert,
                  colbert_scale: embedding.colbert_scale,
               })
               .collect();

            self.store.insert_segment_batch(store_id, table_name, records).await?;

            for entry in batch {
               outcome.indexed_paths.push(entry.path_key.to_string_lossy().into_owned());
               meta_store.set_meta(
                  entry.path_key,
                  entry.hash,
                  entry.mtime,
                  entry.size,
                  entry.head_hash,
               );
            }
            outcome.indexed = outcome.indexed_paths.len();
            return Ok(outcome);
         },
         Err(_) => {
            // Fall back to per-file retries to isolate failures.
         },
      }

      for entry in batch {
         let PendingEmbed {
            path_key,
            hash,
            mtime,
            size,
            head_hash,
            chunks,
         } = entry;
         let texts: Vec<Str> = chunks.iter().map(|c| c.text.clone()).collect();
         if texts.is_empty() {
            continue;
         }
         match self.embed_with_retry(&texts, options).await {
            Ok(embeddings) => {
               if embeddings.len() != chunks.len() {
                  return Err(
                     Error::Server {
                        op:     "embed",
                        reason: "embedding count mismatch for file".to_string(),
                     }
                     .into(),
                  );
               }
               let records: Vec<VectorRecord> = chunks
                  .into_iter()
                  .zip(embeddings.into_iter())
                  .map(|(chunk, embedding)| VectorRecord {
                     row_id:        chunk.row_id,
                     chunk_id:      chunk.chunk_id,
                     path_key:      chunk.path_key,
                     path_key_ci:   chunk.path_key_ci,
                     ordinal:       chunk.ordinal,
                     file_hash:     chunk.file_hash,
                     chunk_hash:    chunk.chunk_hash,
                     chunker:       chunk.chunker,
                     kind:          chunk.kind,
                     text:          chunk.text,
                     start_line:    chunk.start_line,
                     end_line:      chunk.end_line,
                     chunk_type:    chunk.chunk_type,
                     context_prev:  chunk.context_prev,
                     context_next:  chunk.context_next,
                     vector:        embedding.dense,
                     colbert:       embedding.colbert,
                     colbert_scale: embedding.colbert_scale,
                  })
                  .collect();

               self.store.insert_segment_batch(store_id, table_name, records).await?;
               outcome.indexed_paths.push(path_key.to_string_lossy().into_owned());
               meta_store.set_meta(path_key, hash, mtime, size, head_hash);
               outcome.indexed = outcome.indexed.saturating_add(1);
            },
            Err(err) => {
               outcome.errors.push(SnapshotError {
                  code:     "embed_failed".to_string(),
                  message:  format!("embedding failed: {err}"),
                  path_key: path_key.to_string_lossy().into_owned(),
               });
            },
         }
      }

      Ok(outcome)
   }
}

#[cfg(test)]
mod tests {
   use std::sync::atomic::{AtomicBool, Ordering};

   use tempfile::TempDir;

   use super::*;

   #[tokio::test]
   async fn stable_read_detects_change_after_read() {
      let root = TempDir::new().expect("temp dir");
      let file_path = root.path().join("file.txt");
      std::fs::write(&file_path, "hello world").expect("write file");

      let toggle = Arc::new(AtomicBool::new(false));
      let hook_path = file_path.clone();
      let toggle_flag = Arc::clone(&toggle);
      set_stable_read_hook(Some(Arc::new(move || {
         let next = !toggle_flag.load(Ordering::SeqCst);
         toggle_flag.store(next, Ordering::SeqCst);
         if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&hook_path) {
            let _ = file.set_len(if next { 0 } else { 1 });
         }
      })));

      let result = read_file_verified(root.path(), &file_path).await;
      set_stable_read_hook(None);

      match result {
         Err(Error::Server { op, .. }) => assert_eq!(op, "stable_read"),
         _ => panic!("expected stable_read failure"),
      }
   }
}
