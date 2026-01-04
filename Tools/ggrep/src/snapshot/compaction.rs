//! Snapshot compaction (merge segments, prune tombstones, publish new snapshot).

use std::{
   collections::{HashMap, HashSet},
   fs,
   path::Path,
   sync::Arc,
   time::Instant,
};

use arrow_array::{Array, RecordBatch, StringArray};
use arrow_array::builder::BooleanBuilder;
use arrow_select::filter::filter_record_batch;
use chrono::Utc;
use futures::TryStreamExt;
use lancedb::query::ExecutableQuery;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
   Result, config,
   error::Error,
   lease::WriterLease,
   meta::MetaStore,
   snapshot::{
      SnapshotCounts, SnapshotGitInfo, SnapshotManifest, SnapshotSegmentRef,
      SnapshotTombstoneRef,
      SnapshotManager, segment_table_name, write_segment_file_index,
   },
   store::LanceStore,
   util::{self, fail_point},
};

#[derive(Debug, Clone)]
pub struct CompactionResult {
   pub performed:         bool,
   pub reason:            Option<String>,
   pub base_snapshot_id:  Option<String>,
   pub new_snapshot_id:   Option<String>,
   pub segments_before:   usize,
   pub segments_after:    usize,
   pub tombstones_before: u64,
   pub tombstones_after:  u64,
   pub rows_before:       u64,
   pub rows_after:        u64,
   pub duration_ms:       u64,
}

#[derive(Debug, Clone)]
pub struct CompactionOptions {
   pub force:       bool,
   pub max_retries: usize,
}

impl Default for CompactionOptions {
   fn default() -> Self {
      Self { force: false, max_retries: 1 }
   }
}

#[derive(Debug)]
struct CompactionBuild {
   snapshot_id: String,
   table_name:  String,
   rows_after:  u64,
   path_keys:   HashSet<String>,
}

#[derive(Debug, Deserialize)]
struct TombstoneEntry {
   path_key: String,
}

pub fn compaction_overdue(manifest: &SnapshotManifest) -> bool {
   let cfg = config::get();
   let segments = manifest.segments.len();
   let tombstones: u64 = manifest.tombstones.iter().map(|t| t.count).sum();

   if cfg.compaction_overdue_segments > 0 && segments >= cfg.compaction_overdue_segments {
      return true;
   }
   if cfg.compaction_overdue_tombstones > 0
      && tombstones >= cfg.compaction_overdue_tombstones as u64
   {
      return true;
   }
   false
}

pub async fn compact_store(
   store: Arc<LanceStore>,
   store_id: &str,
   config_fingerprint: &str,
   ignore_fingerprint: &str,
   options: CompactionOptions,
) -> Result<CompactionResult> {
   let start = Instant::now();
   let snapshot_manager = SnapshotManager::new(
      Arc::clone(&store),
      store_id.to_string(),
      config_fingerprint.to_string(),
      ignore_fingerprint.to_string(),
   );

   for attempt in 0..=options.max_retries {
      let base_snapshot_id = snapshot_manager.read_active_snapshot_id()?;
      let Some(base_snapshot_id) = base_snapshot_id else {
         return Ok(CompactionResult {
            performed: false,
            reason: Some("no active snapshot".to_string()),
            base_snapshot_id: None,
            new_snapshot_id: None,
            segments_before: 0,
            segments_after: 0,
            tombstones_before: 0,
            tombstones_after: 0,
            rows_before: 0,
            rows_after: 0,
            duration_ms: start.elapsed().as_millis() as u64,
         });
      };

      let base_manifest =
         SnapshotManifest::load(&snapshot_manager.manifest_path(&base_snapshot_id))?;
      snapshot_manager.verify_manifest(&base_manifest).await?;

      let segments_before = base_manifest.segments.len();
      let rows_before: u64 = base_manifest.segments.iter().map(|s| s.rows).sum();
      let tombstones_before: u64 = base_manifest.tombstones.iter().map(|t| t.count).sum();

      if !options.force && !compaction_overdue(&base_manifest) {
         return Ok(CompactionResult {
            performed: false,
            reason: Some("compaction not required".to_string()),
            base_snapshot_id: Some(base_snapshot_id),
            new_snapshot_id: None,
            segments_before,
            segments_after: segments_before,
            tombstones_before,
            tombstones_after: tombstones_before,
            rows_before,
            rows_after: rows_before,
            duration_ms: start.elapsed().as_millis() as u64,
         });
      }

      let tombstones = load_tombstones(&base_manifest, &snapshot_manager.store_root())?;
      let build = build_compaction_segment(
         Arc::clone(&store),
         store_id,
         &base_manifest,
         &tombstones,
      )
      .await?;
      fail_point("compaction.after_build")?;

      let lease = WriterLease::acquire(store_id).await?;
      let active_after = snapshot_manager.read_active_snapshot_id()?;
      if active_after.as_deref() != Some(&base_snapshot_id) {
         let _ = store.drop_table(store_id, &build.table_name).await;
         drop(lease);
         if attempt < options.max_retries {
            continue;
         }
         return Err(
            Error::Server {
               op:     "compaction",
               reason: "active snapshot changed; retry required".to_string(),
            }
            .into(),
         );
      }

      let mut segments: Vec<SnapshotSegmentRef> = Vec::new();
      let mut segments_after = 0usize;
      let mut rows_after = build.rows_after;

      if build.rows_after > 0 {
         store.create_fts_index(store_id, &build.table_name).await?;
         store.create_vector_index(store_id, &build.table_name).await?;
         let metadata = store.segment_metadata(store_id, &build.table_name).await?;
         rows_after = metadata.rows;
         segments.push(SnapshotSegmentRef {
            kind: "compacted".to_string(),
            ref_type: "lancedb_table".to_string(),
            table: build.table_name.clone(),
            rows: metadata.rows,
            size_bytes: metadata.size_bytes,
            sha256: metadata.sha256,
         });
         segments_after = 1;
      }

      let snapshot_id = build.snapshot_id.clone();
      let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
      let snapshot_dir = snapshot_manager.snapshot_dir(&snapshot_id);
      fs::create_dir_all(&snapshot_dir)?;

      if !build.path_keys.is_empty() && build.rows_after > 0 {
         let mut segment_index: HashMap<String, String> = HashMap::new();
         for path_key in &build.path_keys {
            segment_index.insert(path_key.clone(), build.table_name.clone());
         }
         let final_path = snapshot_dir.join("segment_file_index.jsonl");
         write_segment_file_index(&final_path, &segment_index)?;
      }

      util::fsync_dir(&snapshot_dir)?;

      let manifest = SnapshotManifest {
         schema_version: base_manifest.schema_version,
         chunk_row_schema_version: base_manifest.chunk_row_schema_version,
         snapshot_id: snapshot_id.clone(),
         parent_snapshot_id: Some(base_snapshot_id.clone()),
         created_at: created_at.clone(),
         canonical_root: base_manifest.canonical_root.clone(),
         store_id: base_manifest.store_id.clone(),
         config_fingerprint: base_manifest.config_fingerprint.clone(),
         ignore_fingerprint: base_manifest.ignore_fingerprint.clone(),
         lease_epoch: lease.lease_epoch(),
         git: SnapshotGitInfo {
            head_sha: base_manifest.git.head_sha.clone(),
            dirty: base_manifest.git.dirty,
            untracked_included: base_manifest.git.untracked_included,
         },
         segments,
         tombstones: Vec::<SnapshotTombstoneRef>::new(),
         counts: SnapshotCounts {
            files_indexed: build.path_keys.len() as u64,
            chunks_indexed: rows_after,
            tombstones_added: 0,
         },
         degraded: base_manifest.degraded,
         errors: base_manifest.errors.clone(),
      };

      fail_point("compaction.before_publish")?;
      snapshot_manager
         .publish_manifest(&manifest, lease.owner_id(), lease.lease_epoch())
         .await?;
      fail_point("compaction.after_publish")?;

      if let Ok(mut meta_store) = MetaStore::load(store_id) {
         let duration_ms = start.elapsed().as_millis() as u64;
         meta_store.set_snapshot_status(
            manifest.snapshot_id.clone(),
            manifest.created_at.clone(),
            manifest.degraded,
         );
         meta_store.record_compaction(duration_ms);
         let _ = meta_store.save();
      }

      return Ok(CompactionResult {
         performed: true,
         reason: None,
         base_snapshot_id: Some(base_snapshot_id),
         new_snapshot_id: Some(snapshot_id),
         segments_before,
         segments_after,
         tombstones_before,
         tombstones_after: 0,
         rows_before,
         rows_after,
         duration_ms: start.elapsed().as_millis() as u64,
      });
   }

   Err(
      Error::Server {
         op:     "compaction",
         reason: "compaction retries exhausted".to_string(),
      }
      .into(),
   )
}

fn load_tombstones(
   manifest: &SnapshotManifest,
   store_root: &Path,
) -> Result<HashSet<String>> {
   let mut tombstones = HashSet::new();
   for tombstone in &manifest.tombstones {
      let path = store_root.join(&tombstone.path);
      let data = fs::read_to_string(&path).map_err(|e| Error::Server {
         op:     "tombstones",
         reason: format!("failed to read {}: {e}", path.display()),
      })?;
      for line in data.lines() {
         if line.trim().is_empty() {
            continue;
         }
         let entry: TombstoneEntry = serde_json::from_str(line).map_err(|e| Error::Server {
            op:     "tombstones",
            reason: format!("invalid tombstone line in {}: {e}", path.display()),
         })?;
         tombstones.insert(entry.path_key);
      }
   }
   Ok(tombstones)
}

async fn build_compaction_segment(
   store: Arc<LanceStore>,
   store_id: &str,
   base_manifest: &SnapshotManifest,
   tombstones: &HashSet<String>,
) -> Result<CompactionBuild> {
   let snapshot_id = Uuid::new_v4().to_string();
   let table_name = segment_table_name(&snapshot_id, 0);

   let mut rows_after: u64 = 0;
   let mut path_keys: HashSet<String> = HashSet::new();

   for segment in &base_manifest.segments {
      let table = store.get_table(store_id, &segment.table).await?;
      let mut stream = table
         .query()
         .execute()
         .await
         .map_err(|e| Error::Server {
            op:     "compaction",
            reason: format!("failed to scan segment {}: {e}", segment.table),
         })?;
      while let Some(batch) = stream.try_next().await.map_err(|e| Error::Server {
         op:     "compaction",
         reason: format!("failed to read segment {}: {e}", segment.table),
      })? {
         let (filtered, kept) = filter_batch(&batch, tombstones, &mut path_keys)?;
         if kept == 0 {
            continue;
         }
         store
            .append_record_batch(store_id, &table_name, filtered)
            .await?;
         rows_after = rows_after.saturating_add(kept as u64);
      }
   }

   Ok(CompactionBuild { snapshot_id, table_name, rows_after, path_keys })
}

fn filter_batch(
   batch: &RecordBatch,
   tombstones: &HashSet<String>,
   path_keys: &mut HashSet<String>,
) -> Result<(RecordBatch, usize)> {
   let path_col = batch
      .column_by_name("path_key")
      .ok_or_else(|| Error::Server {
         op:     "compaction",
         reason: "missing path_key column".to_string(),
      })?
      .as_any()
      .downcast_ref::<StringArray>()
      .ok_or_else(|| Error::Server {
         op:     "compaction",
         reason: "path_key column type mismatch".to_string(),
      })?;

   let mut builder = BooleanBuilder::new();
   let mut kept = 0usize;
   for i in 0..batch.num_rows() {
      if path_col.is_null(i) {
         builder.append_value(false);
         continue;
      }
      let path = path_col.value(i);
      if tombstones.contains(path) {
         builder.append_value(false);
         continue;
      }
      builder.append_value(true);
      kept += 1;
      path_keys.insert(path.to_string());
   }

   let filter = builder.finish();
   let filtered = filter_record_batch(batch, &filter)?;
   Ok((filtered, kept))
}
