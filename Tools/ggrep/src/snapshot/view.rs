//! Snapshot view (segments + tombstone filter).

use std::{
   collections::{HashMap, HashSet},
   fs,
   path::Path,
};

use serde::Deserialize;

use crate::{Result, error::Error};

use super::{
   manifest::{SnapshotManifest, SnapshotTombstoneRef},
   read_segment_file_index,
};

#[derive(Debug, Clone)]
pub struct SnapshotView {
   pub snapshot_id: String,
   pub manifest:    SnapshotManifest,
   tombstones:      HashSet<String>,
   segment_tables: Vec<String>,
   segment_index: HashMap<String, String>,
}

impl SnapshotView {
   pub fn from_manifest(manifest: SnapshotManifest, store_root: &Path) -> Result<Self> {
      let tombstones = load_tombstones(&manifest.tombstones, store_root)?;
      let segment_index_path = store_root
         .join("snapshots")
         .join(&manifest.snapshot_id)
         .join("segment_file_index.jsonl");
      let segment_index = if segment_index_path.exists() {
         read_segment_file_index(&segment_index_path)?
      } else {
         HashMap::new()
      };
      let segment_tables = manifest.segments.iter().map(|s| s.table.clone()).collect();
      Ok(Self {
         snapshot_id: manifest.snapshot_id.clone(),
         manifest,
         tombstones,
         segment_tables,
         segment_index,
      })
   }

   pub fn is_tombstoned(&self, path_key: &str) -> bool {
      self.tombstones.contains(path_key)
   }

   pub fn is_visible(&self, path_key: &str, segment_table: Option<&str>) -> bool {
      if !self.tombstones.contains(path_key) {
         return true;
      }

      let Some(current_segment) = self.segment_index.get(path_key) else {
         return false;
      };
      segment_table
         .is_some_and(|table| table == current_segment.as_str())
   }

   pub fn segment_tables(&self) -> &[String] {
      &self.segment_tables
   }
}

#[derive(Debug, Deserialize)]
struct TombstoneEntry {
   path_key: String,
}

fn load_tombstones(
   refs: &[SnapshotTombstoneRef],
   store_root: &Path,
) -> Result<HashSet<String>> {
   let mut tombstones = HashSet::new();
   for tombstone in refs {
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
