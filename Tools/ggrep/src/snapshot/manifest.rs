//! Snapshot manifest schema (v1).

use std::{
   fs,
   io::Write,
   path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{Result, error::Error, util::fsync_dir};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const CHUNK_ROW_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotGitInfo {
   pub head_sha:           Option<String>,
   pub dirty:              bool,
   pub untracked_included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotSegmentRef {
   pub kind:       String,
   pub ref_type:   String,
   pub table:      String,
   pub rows:       u64,
   pub size_bytes: u64,
   pub sha256:     String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotTombstoneRef {
   pub ref_type:   String,
   pub path:       String,
   pub count:      u64,
   pub size_bytes: u64,
   pub sha256:     String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotError {
   pub code:     String,
   pub message:  String,
   pub path_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotCounts {
   pub files_indexed:    u64,
   pub chunks_indexed:   u64,
   pub tombstones_added: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotManifest {
   pub schema_version: u32,
   pub chunk_row_schema_version: u32,
   pub snapshot_id: String,
   pub parent_snapshot_id: Option<String>,
   pub created_at: String,
   pub canonical_root: String,
   pub store_id: String,
   pub config_fingerprint: String,
   pub ignore_fingerprint: String,
   pub lease_epoch: u64,
   pub git: SnapshotGitInfo,
   pub segments: Vec<SnapshotSegmentRef>,
   pub tombstones: Vec<SnapshotTombstoneRef>,
   pub counts: SnapshotCounts,
   pub degraded: bool,
   pub errors: Vec<SnapshotError>,
}

impl SnapshotManifest {
   pub fn load(path: &Path) -> Result<Self> {
      let raw = fs::read_to_string(path)?;
      let manifest: SnapshotManifest = serde_json::from_str(&raw)?;
      Ok(manifest)
   }

   pub fn write_atomic(&self, path: &Path) -> Result<()> {
      if let Some(parent) = path.parent() {
         fs::create_dir_all(parent)?;
      }
      let tmp_path = temp_path(path)?;
      let data = serde_json::to_string_pretty(self)?;
      {
         let mut file = fs::File::create(&tmp_path)?;
         file.write_all(data.as_bytes())?;
         file.sync_all()?;
      }
      fs::rename(&tmp_path, path)?;
      if let Some(parent) = path.parent() {
         fsync_dir(parent)?;
      }
      Ok(())
   }
}

fn temp_path(path: &Path) -> Result<PathBuf> {
   let name = path
      .file_name()
      .and_then(|n| n.to_str())
      .ok_or_else(|| Error::Server {
         op:     "manifest",
         reason: "invalid manifest path".to_string(),
      })?;
   Ok(path.with_file_name(format!("{name}.tmp")))
}
