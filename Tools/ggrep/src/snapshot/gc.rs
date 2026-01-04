//! Snapshot retention and artifact garbage collection.

use std::{
   collections::HashSet,
   fs,
   path::{Path, PathBuf},
   sync::Arc,
   time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
   Result, config,
   assert,
   util::fail_point,
   lease::WriterLease,
   reader_lock::ReaderLock,
   snapshot::{SnapshotManifest, SnapshotManager},
   store::LanceStore,
   util,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcReport {
   pub dry_run:             bool,
   pub active_snapshot_id:  Option<String>,
   pub retained_snapshots:  Vec<String>,
   pub deleted_snapshots:   Vec<String>,
   pub deleted_segments:    Vec<String>,
   pub deleted_tombstones:  Vec<String>,
   pub duration_ms:         u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcOptions {
   pub dry_run:         bool,
   pub active_snapshot: Option<String>,
   pub pinned:          HashSet<String>,
   pub retain_snapshots_min: Option<usize>,
   pub retain_snapshots_min_age_secs: Option<u64>,
   pub safety_window_ms: Option<u64>,
}

impl Default for GcOptions {
   fn default() -> Self {
      Self {
         dry_run: true,
         active_snapshot: None,
         pinned: HashSet::new(),
         retain_snapshots_min: None,
         retain_snapshots_min_age_secs: None,
         safety_window_ms: None,
      }
   }
}

#[derive(Debug)]
struct SnapshotEntry {
   snapshot_id: String,
   created_at:  DateTime<Utc>,
   manifest:    SnapshotManifest,
}

pub async fn gc_snapshots(
   store: Arc<LanceStore>,
   store_id: &str,
   config_fingerprint: &str,
   ignore_fingerprint: &str,
   options: GcOptions,
) -> Result<GcReport> {
   let start = Instant::now();

   let snapshot_manager = SnapshotManager::new(
      Arc::clone(&store),
      store_id.to_string(),
      config_fingerprint.to_string(),
      ignore_fingerprint.to_string(),
   );

   let active_snapshot_id = options
      .active_snapshot
      .clone()
      .or_else(|| snapshot_manager.read_active_snapshot_id().ok().flatten());

   let lease = WriterLease::acquire(store_id).await?;
   assert::gc_requires_writer_lease(store_id, lease.owner_id(), lease.lease_epoch())?;
   let _reader_lock = ReaderLock::acquire_exclusive(store_id)?;

   snapshot_manager.cleanup_staging()?;

   let snapshots_dir = snapshot_manager.snapshots_dir();
   let mut snapshots: Vec<SnapshotEntry> = Vec::new();
   if snapshots_dir.exists() {
      for entry in fs::read_dir(&snapshots_dir)? {
         let entry = entry?;
         if !entry.file_type()?.is_dir() {
            continue;
         }
         let manifest_path = entry.path().join("manifest.json");
         let Ok(manifest) = SnapshotManifest::load(&manifest_path) else {
            continue;
         };
         let Ok(created_at) = DateTime::parse_from_rfc3339(&manifest.created_at) else {
            continue;
         };
         snapshots.push(SnapshotEntry {
            snapshot_id: manifest.snapshot_id.clone(),
            created_at: created_at.with_timezone(&Utc),
            manifest,
         });
      }
   }

   snapshots.sort_by(|a, b| b.created_at.cmp(&a.created_at));

   let cfg = config::get();
   let min_keep = options
      .retain_snapshots_min
      .unwrap_or(cfg.retain_snapshots_min)
      .max(1);
   let min_age = Duration::from_secs(
      options
         .retain_snapshots_min_age_secs
         .unwrap_or(cfg.retain_snapshots_min_age_secs),
   );
   let safety_window = Duration::from_millis(
      options
         .safety_window_ms
         .unwrap_or(cfg.query_timeout_ms + cfg.gc_safety_margin_ms),
   );

   let mut retain: HashSet<String> = HashSet::new();
   if let Some(active) = &active_snapshot_id {
      retain.insert(active.clone());
   }
   retain.extend(options.pinned.iter().cloned());

   let now = Utc::now();
   for entry in &snapshots {
      let age = now.signed_duration_since(entry.created_at);
      if age.num_seconds() <= min_age.as_secs() as i64 {
         retain.insert(entry.snapshot_id.clone());
      }
      if age.num_seconds() <= safety_window.as_secs() as i64 {
         retain.insert(entry.snapshot_id.clone());
      }
   }

   for entry in snapshots.iter().take(min_keep) {
      retain.insert(entry.snapshot_id.clone());
   }

   let mut retained_manifests: Vec<&SnapshotManifest> = Vec::new();
   let mut deleted_snapshots: Vec<String> = Vec::new();
   for entry in &snapshots {
      if retain.contains(&entry.snapshot_id) {
         retained_manifests.push(&entry.manifest);
      } else {
         deleted_snapshots.push(entry.snapshot_id.clone());
      }
   }

   fail_point("gc.after_delete_list")?;

   let mut keep_segments: HashSet<String> = HashSet::new();
   let mut keep_tombstones: HashSet<String> = HashSet::new();
   for manifest in retained_manifests {
      for segment in &manifest.segments {
         keep_segments.insert(segment.table.clone());
      }
      for tombstone in &manifest.tombstones {
         keep_tombstones.insert(tombstone.path.clone());
      }
   }

   let store_root = snapshot_manager.store_root();
   let mut deleted_tombstones = Vec::new();

   if !options.dry_run {
      fail_point("gc.before_delete")?;
      for snapshot_id in &deleted_snapshots {
         let snapshot_dir = snapshot_manager.snapshot_dir(snapshot_id);
         if snapshot_dir.exists() {
            let (deleted, _) = delete_snapshot_dir(&store_root, &snapshot_dir, &keep_tombstones)?;
            deleted_tombstones.extend(deleted);
         }
      }
   } else {
      for snapshot_id in &deleted_snapshots {
         let snapshot_dir = snapshot_manager.snapshot_dir(snapshot_id);
         if snapshot_dir.exists() {
            for entry in fs::read_dir(&snapshot_dir)? {
               let entry = entry?;
               let path = entry.path();
               let rel = path.strip_prefix(&store_root).unwrap_or(&path);
               let rel_str = rel.to_string_lossy().to_string();
               if keep_tombstones.contains(&rel_str) {
                  continue;
               }
               if rel_str.ends_with("tombstones.jsonl") {
                  deleted_tombstones.push(rel_str);
               }
            }
         }
      }
   }

   let mut deleted_segments = Vec::new();
   if !options.dry_run {
      let tables = store.list_tables(store_id).await.unwrap_or_default();
      for table in tables {
         if !table.starts_with("seg_") {
            continue;
         }
         if keep_segments.contains(&table) {
            continue;
         }
         if store.drop_table(store_id, &table).await.is_ok() {
            deleted_segments.push(table);
         }
      }
   }

   if !options.dry_run {
      if let Ok(mut meta_store) = crate::meta::MetaStore::load(store_id) {
         meta_store.record_gc(start.elapsed().as_millis() as u64);
         let _ = meta_store.save();
      }
   }

   let mut retained_snapshots: Vec<String> = retain.into_iter().collect();
   retained_snapshots.sort();
   deleted_snapshots.sort();
   deleted_segments.sort();
   deleted_tombstones.sort();

   Ok(GcReport {
      dry_run: options.dry_run,
      active_snapshot_id,
      retained_snapshots,
      deleted_snapshots,
      deleted_segments,
      deleted_tombstones,
      duration_ms: start.elapsed().as_millis() as u64,
   })
}

fn delete_snapshot_dir(
   store_root: &Path,
   snapshot_dir: &Path,
   keep_tombstones: &HashSet<String>,
) -> Result<(Vec<String>, Vec<PathBuf>)> {
   let mut deleted_tombstones = Vec::new();
   let mut deleted_paths = Vec::new();

   for entry in fs::read_dir(snapshot_dir)? {
      let entry = entry?;
      let path = entry.path();
      let rel = path.strip_prefix(store_root).unwrap_or(&path);
      let rel_str = rel.to_string_lossy().to_string();
      if keep_tombstones.contains(&rel_str) {
         continue;
      }
      if path.is_dir() {
         fs::remove_dir_all(&path)?;
         deleted_paths.push(path);
      } else {
         if rel_str.ends_with("tombstones.jsonl") {
            deleted_tombstones.push(rel_str);
         }
         fs::remove_file(&path)?;
         deleted_paths.push(path);
      }
   }

   if snapshot_dir.read_dir()?.next().is_none() {
      let _ = fs::remove_dir(snapshot_dir);
   }

   let _ = util::fsync_dir(snapshot_dir);
   Ok((deleted_tombstones, deleted_paths))
}
