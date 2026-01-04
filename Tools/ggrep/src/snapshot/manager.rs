//! Snapshot manager (publish + active snapshot pointer).

use std::{
   fs::{self, File},
   io::Write,
   path::{Path, PathBuf},
   sync::Arc,
   time::SystemTime,
};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::{
   Result, config,
   assert,
   error::Error,
   store::LanceStore,
   util::{fail_point, fsync_dir},
};

use super::manifest::{CHUNK_ROW_SCHEMA_VERSION, MANIFEST_SCHEMA_VERSION, SnapshotManifest};
use super::view::SnapshotView;

#[derive(Clone)]
pub struct SnapshotManager {
   store:              Arc<LanceStore>,
   store_id:           String,
   config_fingerprint: String,
   ignore_fingerprint: String,
}

impl SnapshotManager {
   pub fn new(
      store: Arc<LanceStore>,
      store_id: String,
      config_fingerprint: String,
      ignore_fingerprint: String,
   ) -> Self {
      Self { store, store_id, config_fingerprint, ignore_fingerprint }
   }

   pub fn store_root(&self) -> PathBuf {
      config::data_dir().join(&self.store_id)
   }

   pub fn snapshots_dir(&self) -> PathBuf {
      self.store_root().join("snapshots")
   }

   pub fn staging_dir(&self) -> PathBuf {
      self.store_root().join("staging")
   }

   pub fn active_snapshot_path(&self) -> PathBuf {
      self.store_root().join("ACTIVE_SNAPSHOT")
   }

   pub fn read_active_snapshot_id(&self) -> Result<Option<String>> {
      let path = self.active_snapshot_path();
      match fs::read_to_string(&path) {
         Ok(raw) => {
            let id = raw.lines().next().unwrap_or("").trim();
            if id.is_empty() {
               Ok(None)
            } else {
               Ok(Some(id.to_string()))
            }
         },
         Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
         Err(e) => Err(e.into()),
      }
   }

   pub fn manifest_path(&self, snapshot_id: &str) -> PathBuf {
      self.snapshot_dir(snapshot_id).join("manifest.json")
   }

   pub fn snapshot_dir(&self, snapshot_id: &str) -> PathBuf {
      self.snapshots_dir().join(snapshot_id)
   }

   pub fn staging_path(&self, staging_txn_id: &str) -> PathBuf {
      self.staging_dir().join(staging_txn_id)
   }

   pub fn create_staging(&self, staging_txn_id: &str) -> Result<PathBuf> {
      let path = self.staging_path(staging_txn_id);
      fs::create_dir_all(&path)?;
      Ok(path)
   }

   pub fn cleanup_staging(&self) -> Result<()> {
      let ttl = std::time::Duration::from_millis(config::get().staging_ttl_ms);
      let now = SystemTime::now();
      let staging_dir = self.staging_dir();
      if !staging_dir.exists() {
         return Ok(());
      }
      for entry in fs::read_dir(&staging_dir)? {
         let entry = entry?;
         if !entry.file_type()?.is_dir() {
            continue;
         }
         let path = entry.path();
         let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(now);
         if now.duration_since(mtime).unwrap_or(ttl) > ttl {
            let _ = fs::remove_dir_all(&path);
         }
      }
      Ok(())
   }

   pub async fn open_snapshot_view(&self) -> Result<SnapshotView> {
      if let Some(active_id) = self.read_active_snapshot_id()? {
         let manifest = SnapshotManifest::load(&self.manifest_path(&active_id))?;
         if self.verify_manifest(&manifest).await.is_ok() {
            return SnapshotView::from_manifest(manifest, &self.store_root());
         }
      }

      let mut candidates: Vec<(DateTime<Utc>, SnapshotManifest)> = Vec::new();
      let dir = self.snapshots_dir();
      if dir.exists() {
         for entry in fs::read_dir(&dir)? {
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
            candidates.push((created_at.with_timezone(&Utc), manifest));
         }
      }

      candidates.sort_by(|a, b| b.0.cmp(&a.0));
      for (_, manifest) in candidates {
         if self.verify_manifest(&manifest).await.is_ok() {
            return SnapshotView::from_manifest(manifest, &self.store_root());
         }
      }

      Err(
         Error::Server {
            op:     "snapshot",
            reason: "store corrupt: no valid snapshot".to_string(),
         }
         .into(),
      )
   }

   pub fn write_active_snapshot(&self, snapshot_id: &str) -> Result<()> {
      let path = self.active_snapshot_path();
      if let Some(parent) = path.parent() {
         fs::create_dir_all(parent)?;
      }
      let tmp_path = path.with_file_name("ACTIVE_SNAPSHOT.tmp");
      {
         let mut file = File::create(&tmp_path)?;
         writeln!(file, "{snapshot_id}")?;
         file.sync_all()?;
      }
      fs::rename(&tmp_path, &path)?;
      if let Some(parent) = path.parent() {
         fsync_dir(parent)?;
      }
      Ok(())
   }

   pub async fn publish_manifest(
      &self,
      manifest: &SnapshotManifest,
      lease_owner: &str,
      lease_epoch: u64,
   ) -> Result<()> {
      assert::lease_epoch_preflight(&self.store_id, lease_owner, lease_epoch)?;
      self.verify_manifest(manifest).await?;

      let snapshot_dir = self.snapshot_dir(&manifest.snapshot_id);
      fs::create_dir_all(&snapshot_dir)?;

      let manifest_path = snapshot_dir.join("manifest.json");
      manifest.write_atomic(&manifest_path)?;
      fsync_dir(&snapshot_dir)?;
      fail_point("publish.after_manifest")?;

      fail_point("publish.before_pointer_swap")?;
      self.write_active_snapshot(&manifest.snapshot_id)?;
      fail_point("publish.after_pointer_swap")?;
      Ok(())
   }

   pub async fn verify_manifest(&self, manifest: &SnapshotManifest) -> Result<()> {
      if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
         return Err(Error::Server {
            op:     "snapshot",
            reason: format!(
               "unsupported manifest schema version {}",
               manifest.schema_version
            ),
         }
         .into());
      }
      if manifest.chunk_row_schema_version != CHUNK_ROW_SCHEMA_VERSION {
         return Err(Error::Server {
            op:     "snapshot",
            reason: format!(
               "unsupported chunk row schema version {}",
               manifest.chunk_row_schema_version
            ),
         }
         .into());
      }
      if manifest.store_id != self.store_id {
         return Err(Error::Server {
            op:     "snapshot",
            reason: "manifest store id mismatch".to_string(),
         }
         .into());
      }
      if manifest.config_fingerprint != self.config_fingerprint {
         return Err(Error::Server {
            op:     "snapshot",
            reason: "manifest config fingerprint mismatch".to_string(),
         }
         .into());
      }
      if manifest.ignore_fingerprint != self.ignore_fingerprint {
         return Err(Error::Server {
            op:     "snapshot",
            reason: "manifest ignore fingerprint mismatch".to_string(),
         }
         .into());
      }

      let segment_rows: u64 = manifest.segments.iter().map(|s| s.rows).sum();
      let tombstone_count: u64 = manifest.tombstones.iter().map(|t| t.count).sum();
      if manifest.counts.chunks_indexed != segment_rows {
         return Err(Error::Server {
            op:     "snapshot",
            reason: "manifest chunk counts mismatch".to_string(),
         }
         .into());
      }
      if manifest.counts.tombstones_added != tombstone_count {
         return Err(Error::Server {
            op:     "snapshot",
            reason: "manifest tombstone counts mismatch".to_string(),
         }
         .into());
      }

      for segment in &manifest.segments {
         let metadata = self
            .store
            .segment_metadata(&self.store_id, &segment.table)
            .await?;
         if metadata.rows != segment.rows {
            return Err(Error::Server {
               op:     "snapshot",
               reason: format!("segment rows mismatch for {}", segment.table),
            }
            .into());
         }
         if metadata.size_bytes != segment.size_bytes || metadata.sha256 != segment.sha256 {
            return Err(Error::Server {
               op:     "snapshot",
               reason: format!("segment checksum mismatch for {}", segment.table),
            }
            .into());
         }
      }

      for tombstone in &manifest.tombstones {
         let path = self.store_root().join(&tombstone.path);
         let (size_bytes, sha256, count) = compute_tombstone_artifact(&path)?;
         if size_bytes != tombstone.size_bytes
            || sha256 != tombstone.sha256
            || count != tombstone.count
         {
            return Err(Error::Server {
               op:     "snapshot",
               reason: format!("tombstone checksum mismatch for {}", tombstone.path),
            }
            .into());
         }
      }

      Ok(())
   }
}

pub fn compute_dir_hash(path: &Path) -> Result<(u64, String)> {
   let mut files = Vec::new();
   for entry in WalkDir::new(path) {
      let entry = entry.map_err(|e| Error::Server {
         op:     "snapshot",
         reason: format!("walkdir error: {e}"),
      })?;
      if entry.file_type().is_file() {
         files.push(entry.path().to_path_buf());
      }
   }
   files.sort();

   let mut hasher = Sha256::new();
   let mut size_bytes = 0u64;
   for file in files {
      let rel = file.strip_prefix(path).unwrap_or(&file);
      hasher.update(rel.as_os_str().as_encoded_bytes());
      hasher.update([0u8]);
      let data = fs::read(&file)?;
      size_bytes = size_bytes.saturating_add(data.len() as u64);
      hasher.update(&data);
   }
   let digest = hex::encode(hasher.finalize());
   Ok((size_bytes, digest))
}

pub fn compute_tombstone_artifact(path: &Path) -> Result<(u64, String, u64)> {
   let data = fs::read(path)?;
   let size_bytes = data.len() as u64;
   let mut hasher = Sha256::new();
   hasher.update(&data);
   let digest = hex::encode(hasher.finalize());
   let count = data
      .split(|b| *b == b'\n')
      .filter(|line| !line.is_empty())
      .count() as u64;
   Ok((size_bytes, digest, count))
}

pub fn segment_table_name(snapshot_id: &str, seq: usize) -> String {
   format!("seg_{snapshot_id}_{seq}")
}
