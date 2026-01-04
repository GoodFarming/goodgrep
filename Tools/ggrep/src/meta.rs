//! File metadata tracking for incremental indexing

use std::{
   collections::HashMap,
   fmt, fs,
   path::{Path, PathBuf},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{Result, config, file::path_key_from_real};

/// Metadata for a single file
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct FileMeta {
   pub hash:      FileHash,
   pub mtime:     u64,
   #[serde(default)]
   pub size:      u64,
   #[serde(default)]
   pub head_hash: Option<FileHash>,
}

/// SHA-256 hash of file contents
#[derive(Serialize, Deserialize, Copy, Clone, Default, Eq, PartialEq, Hash)]
#[repr(transparent)]
pub struct FileHash([u8; 32]);

impl FileHash {
   /// Creates a hash from a byte slice, verifying length
   pub fn from_slice(slice: &[u8]) -> Option<Self> {
      let (this, rem) = slice.split_first_chunk()?;
      rem.is_empty().then_some(Self(*this))
   }

   pub const fn new(hash: [u8; 32]) -> Self {
      Self(hash)
   }

   /// Computes SHA-256 hash of data
   pub fn sum(dat: impl AsRef<[u8]>) -> Self {
      Self(Sha256::digest(dat.as_ref()).into())
   }
}

impl AsRef<[u8]> for FileHash {
   fn as_ref(&self) -> &[u8] {
      &self.0
   }
}

impl AsMut<[u8]> for FileHash {
   fn as_mut(&mut self) -> &mut [u8] {
      &mut self.0
   }
}

impl fmt::Display for FileHash {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      write!(f, "{}", hex::encode(self.0))
   }
}

impl fmt::Debug for FileHash {
   fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
      write!(f, "Hash({})", hex::encode(self.0))
   }
}

impl std::ops::Deref for FileHash {
   type Target = [u8];

   fn deref(&self) -> &Self::Target {
      &self.0
   }
}

impl std::ops::DerefMut for FileHash {
   fn deref_mut(&mut self) -> &mut Self::Target {
      &mut self.0
   }
}

pub const INDEX_VERSION: &str = "ggrep-3-snapshots-v1";

fn current_index_version() -> String {
   let cfg = config::get();

   let doc_prefix_sig = if cfg.doc_prefix.is_empty() {
      "none".to_string()
   } else {
      let digest: [u8; 32] = Sha256::digest(cfg.doc_prefix.as_bytes()).into();
      hex::encode(digest)
   };

   format!(
      "{INDEX_VERSION}|dense_max_length={}|colbert_max_length={}|doc_prefix={doc_prefix_sig}",
      cfg.dense_max_length, cfg.colbert_max_length,
   )
}

/// Signature of the embedding models and dimensions used to build an index
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ModelSignature {
   pub dense_model:   String,
   pub colbert_model: String,
   pub dense_dim:     usize,
   pub colbert_dim:   usize,
}

impl ModelSignature {
   pub fn current() -> Self {
      let cfg = config::get();

      Self {
         dense_model:   cfg.dense_model.clone(),
         colbert_model: cfg.colbert_model.clone(),
         dense_dim:     cfg.dense_dim,
         colbert_dim:   cfg.colbert_dim,
      }
   }
}

/// Persistent store for file metadata and hashes
#[derive(Serialize, Deserialize, Default)]
pub struct MetaStore {
   #[serde(default)]
   files:                 HashMap<PathBuf, FileMeta>,
   #[serde(default, skip_serializing)]
   hashes:                HashMap<PathBuf, FileHash>,
   #[serde(default)]
   model:                 Option<ModelSignature>,
   #[serde(default)]
   index_version:         Option<String>,
   #[serde(default)]
   config_fingerprint:    Option<String>,
   #[serde(default)]
   ignore_fingerprint:    Option<String>,
   #[serde(default)]
   snapshot_id:           Option<String>,
   #[serde(default)]
   snapshot_created_at:   Option<String>,
   #[serde(default)]
   snapshot_degraded:     Option<bool>,
   #[serde(default)]
   last_sync_at:          Option<String>,
   #[serde(default)]
   last_sync_result:      Option<String>,
   #[serde(default)]
   last_sync_duration_ms: Option<u64>,
   #[serde(default)]
   last_gc_at:            Option<String>,
   #[serde(default)]
   last_gc_duration_ms:   Option<u64>,
   #[serde(default)]
   last_compaction_at:    Option<String>,
   #[serde(default)]
   last_compaction_duration_ms: Option<u64>,
   #[serde(skip)]
   path:                  PathBuf,
   #[serde(skip)]
   dirty:                 bool,
   #[serde(skip)]
   model_mismatch:        bool,
   #[serde(skip)]
   index_mismatch:        bool,
}

impl MetaStore {
   /// Loads metadata store from disk, creating if it doesn't exist
   pub fn load(store_id: &str) -> Result<Self> {
      let meta_dir = config::meta_dir();
      let path = meta_dir.join(format!("{store_id}.json"));
      let existed = path.exists();

      let mut store = if existed {
         let content = fs::read_to_string(&path)?;
         let mut store: Self = serde_json::from_str(&content)?;
         store.path = path;
         store.migrate_legacy_hashes();
         store
      } else {
         Self {
            files: HashMap::new(),
            hashes: HashMap::new(),
            model: None,
            index_version: None,
            config_fingerprint: None,
            ignore_fingerprint: None,
            snapshot_id: None,
            snapshot_created_at: None,
            snapshot_degraded: None,
            last_sync_at: None,
            last_sync_result: None,
            last_sync_duration_ms: None,
            last_gc_at: None,
            last_gc_duration_ms: None,
            last_compaction_at: None,
            last_compaction_duration_ms: None,
            path,
            dirty: false,
            model_mismatch: false,
            index_mismatch: false,
         }
      };

      let current_model = ModelSignature::current();
      let model_mismatch = match (&store.model, existed) {
         (Some(model), true) => model != &current_model,
         (None, true) => true,
         _ => false,
      };

      let current_index = current_index_version();
      let index_mismatch = match (&store.index_version, existed) {
         (Some(version), true) => version != &current_index,
         (None, true) => true,
         _ => false,
      };

      store.model = Some(current_model);
      store.index_version = Some(current_index);
      store.model_mismatch = model_mismatch;
      store.index_mismatch = index_mismatch;
      store.dirty = store.dirty || model_mismatch || index_mismatch || !existed;

      if store.snapshot_id.is_none() && existed && !store.files.is_empty() {
         store.bump_snapshot();
      }

      Ok(store)
   }

   fn migrate_legacy_hashes(&mut self) {
      for (path, hash) in self.hashes.drain() {
         self.files.entry(path).or_insert_with(|| FileMeta {
            hash,
            mtime: 0,
            size: 0,
            head_hash: None,
         });
      }
   }

   /// Gets the stored hash for a file
   pub fn get_hash(&self, path: &Path) -> Option<FileHash> {
      self.files.get(path).map(|m| m.hash)
   }

   /// Gets the stored modification time for a file
   pub fn get_mtime(&self, path: &Path) -> Option<u64> {
      self.files.get(path).map(|m| m.mtime)
   }

   /// Gets the complete metadata for a file
   pub fn get_meta(&self, path: &Path) -> Option<&FileMeta> {
      self.files.get(path)
   }

   /// Updates the hash for a file
   pub fn set_hash(&mut self, path: &Path, hash: FileHash) {
      if let Some(meta) = self.files.get_mut(path) {
         meta.hash = hash;
      } else {
         self.files.insert(path.to_path_buf(), FileMeta {
            hash,
            mtime: 0,
            size: 0,
            head_hash: None,
         });
      }
      self.dirty = true;
   }

   /// Sets complete metadata for a file
   pub fn set_meta(
      &mut self,
      path: PathBuf,
      hash: FileHash,
      mtime: u64,
      size: u64,
      head_hash: FileHash,
   ) {
      self
         .files
         .insert(path, FileMeta { hash, mtime, size, head_hash: Some(head_hash) });
      self.dirty = true;
   }

   /// Removes metadata for a file
   pub fn remove(&mut self, path: &Path) {
      self.files.remove(path);
      self.dirty = true;
   }

   /// Saves the metadata store to disk if dirty
   pub fn save(&mut self) -> Result<()> {
      if !self.dirty {
         return Ok(());
      }

      if let Some(parent) = self.path.parent() {
         fs::create_dir_all(parent)?;
      }

      let content = serde_json::to_string(&self)?;
      fs::write(&self.path, content)?;

      self.dirty = false;
      Ok(())
   }

   /// Returns an iterator over all tracked file paths
   pub fn all_paths(&self) -> impl Iterator<Item = &PathBuf> {
      self.files.keys()
   }

   /// Normalizes stored paths to repo-relative keys when absolute paths are
   /// present.
   pub fn normalize_paths(&mut self, root: &Path) {
      if self.files.is_empty() {
         return;
      }

      let mut updated: HashMap<PathBuf, FileMeta> = HashMap::with_capacity(self.files.len());
      let mut changed = false;

      for (path, meta) in self.files.iter() {
         let key = if path.is_absolute() {
            match path_key_from_real(root, path) {
               Some(rel) => rel,
               None => continue,
            }
         } else {
            path.clone()
         };
         if &key != path {
            changed = true;
         }
         updated.entry(key).or_insert_with(|| meta.clone());
      }

      if changed || updated.len() != self.files.len() {
         self.files = updated;
         self.dirty = true;
      }
   }

   /// Deletes all metadata for files with a given path prefix
   pub fn delete_by_prefix(&mut self, prefix: &Path) {
      self.files.retain(|path, _| !path.starts_with(prefix));
      self.dirty = true;
   }

   /// Clears all tracked metadata.
   pub fn clear_all(&mut self) {
      if !self.files.is_empty() {
         self.files.clear();
         self.dirty = true;
      }
      self.snapshot_id = None;
      self.snapshot_created_at = None;
      self.snapshot_degraded = None;
      self.last_sync_at = None;
      self.last_sync_result = None;
      self.last_sync_duration_ms = None;
      self.last_gc_at = None;
      self.last_gc_duration_ms = None;
      self.last_compaction_at = None;
      self.last_compaction_duration_ms = None;
   }

   /// Whether the stored model signature differs from the current configuration
   pub const fn model_mismatch(&self) -> bool {
      self.model_mismatch
   }

   /// Whether the stored index version differs from the current signature
   pub const fn index_mismatch(&self) -> bool {
      self.index_mismatch
   }

   /// Clears all tracked metadata and records the current index signature
   pub fn reset_for_signature_change(&mut self) {
      self.files.clear();
      self.model = Some(ModelSignature::current());
      self.index_version = Some(current_index_version());
      self.config_fingerprint = None;
      self.ignore_fingerprint = None;
      self.snapshot_id = None;
      self.snapshot_created_at = None;
      self.snapshot_degraded = None;
      self.last_sync_at = None;
      self.last_sync_result = None;
      self.last_sync_duration_ms = None;
      self.last_gc_at = None;
      self.last_gc_duration_ms = None;
      self.last_compaction_at = None;
      self.last_compaction_duration_ms = None;
      self.dirty = true;
      self.model_mismatch = false;
      self.index_mismatch = false;
   }

   pub fn set_fingerprints(&mut self, config_fingerprint: String, ignore_fingerprint: String) {
      if self.config_fingerprint.as_deref() != Some(config_fingerprint.as_str()) {
         self.config_fingerprint = Some(config_fingerprint);
         self.dirty = true;
      }

      if self.ignore_fingerprint.as_deref() != Some(ignore_fingerprint.as_str()) {
         self.ignore_fingerprint = Some(ignore_fingerprint);
         self.dirty = true;
      }
   }

   pub fn config_fingerprint(&self) -> Option<&str> {
      self.config_fingerprint.as_deref()
   }

   pub fn ignore_fingerprint(&self) -> Option<&str> {
      self.ignore_fingerprint.as_deref()
   }

   pub fn snapshot_id(&self) -> Option<&str> {
      self.snapshot_id.as_deref()
   }

   pub fn snapshot_created_at(&self) -> Option<&str> {
      self.snapshot_created_at.as_deref()
   }

   pub fn snapshot_degraded(&self) -> bool {
      self.snapshot_degraded.unwrap_or(false)
   }

   pub fn last_sync_at(&self) -> Option<&str> {
      self.last_sync_at.as_deref()
   }

   pub fn last_sync_result(&self) -> Option<&str> {
      self.last_sync_result.as_deref()
   }

   pub fn last_sync_duration_ms(&self) -> Option<u64> {
      self.last_sync_duration_ms
   }

   pub fn last_gc_duration_ms(&self) -> Option<u64> {
      self.last_gc_duration_ms
   }

   pub fn last_compaction_duration_ms(&self) -> Option<u64> {
      self.last_compaction_duration_ms
   }

   pub fn record_sync(&mut self, result: &str, duration_ms: u64) {
      let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
      self.last_sync_at = Some(now);
      self.last_sync_result = Some(result.to_string());
      self.last_sync_duration_ms = Some(duration_ms);
      self.dirty = true;
   }

   pub fn record_gc(&mut self, duration_ms: u64) {
      let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
      self.last_gc_at = Some(now);
      self.last_gc_duration_ms = Some(duration_ms);
      self.dirty = true;
   }

   pub fn record_compaction(&mut self, duration_ms: u64) {
      let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
      self.last_compaction_at = Some(now);
      self.last_compaction_duration_ms = Some(duration_ms);
      self.dirty = true;
   }

   pub fn bump_snapshot(&mut self) {
      self.snapshot_id = Some(Uuid::new_v4().to_string());
      self.snapshot_created_at =
         Some(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
      self.dirty = true;
   }

   pub fn set_snapshot(&mut self, snapshot_id: String, created_at: String) {
      self.set_snapshot_status(snapshot_id, created_at, false);
   }

   pub fn set_snapshot_status(&mut self, snapshot_id: String, created_at: String, degraded: bool) {
      self.snapshot_id = Some(snapshot_id);
      self.snapshot_created_at = Some(created_at);
      self.snapshot_degraded = Some(degraded);
      self.dirty = true;
   }

   pub fn reset_for_model_change(&mut self) {
      self.reset_for_signature_change();
   }
}

#[cfg(test)]
mod tests {
   use std::fs;

   use tempfile::TempDir;

   use super::*;

   fn with_temp_home(f: impl FnOnce(&TempDir)) {
      let temp_dir = TempDir::new().unwrap();
      // SAFETY: we are setting the HOME environment variable to a temporary directory
      unsafe {
         std::env::set_var("HOME", temp_dir.path());
      }
      f(&temp_dir);
   }

   #[test]
   fn load_nonexistent_creates_empty() {
      with_temp_home(|_| {
         // Use unique store_id to avoid collision with other tests due to OnceLock
         // caching
         let store = MetaStore::load("load_nonexistent_test").unwrap();
         assert_eq!(store.files.len(), 0);
      });
   }

   #[test]
   fn set_and_get_hash() {
      with_temp_home(|_| {
         let hash = FileHash::sum(b"abc123");
         // Use unique store_id to avoid collision with other tests due to OnceLock
         // caching
         let mut store = MetaStore::load("set_and_get_hash_test").unwrap();
         store.set_hash(Path::new("/path/to/file"), hash);

         assert_eq!(store.get_hash("/path/to/file".as_ref()), Some(hash));
         assert!(store.dirty);
      });
   }

   #[test]
   fn save_and_load_roundtrip() {
      with_temp_home(|_| {
         let mut store = MetaStore::load("roundtrip_test").unwrap();
         let hash1 = FileHash::sum(b"hash1");
         let hash2 = FileHash::sum(b"hash2");
         store.set_hash(Path::new("/file1"), hash1);
         store.set_hash(Path::new("/file2"), hash2);
         store.save().unwrap();

         let loaded = MetaStore::load("roundtrip_test").unwrap();
         assert_eq!(loaded.get_hash("/file1".as_ref()), Some(hash1));
         assert_eq!(loaded.get_hash("/file2".as_ref()), Some(hash2));
      });
   }

   #[test]
   fn remove_hash() {
      with_temp_home(|_| {
         let mut store = MetaStore::load("remove_hash_test").unwrap();
         let hash = FileHash::sum(b"hash1");
         store.set_hash(Path::new("/file1"), hash);
         store.remove("/file1".as_ref());

         assert_eq!(store.get_hash("/file1".as_ref()), None);
      });
   }

   #[test]
   fn all_paths_returns_keys() {
      with_temp_home(|_| {
         let mut store = MetaStore::load("all_paths_test").unwrap();
         let hash1 = FileHash::sum(b"hash1");
         let hash2 = FileHash::sum(b"hash2");
         store.set_hash(Path::new("/file1"), hash1);
         store.set_hash(Path::new("/file2"), hash2);

         let paths: Vec<_> = store.all_paths().collect();
         assert_eq!(paths.len(), 2);
         assert!(paths.contains(&&PathBuf::from("/file1")));
         assert!(paths.contains(&&PathBuf::from("/file2")));
      });
   }

   #[test]
   fn detects_model_change_and_resets() {
      with_temp_home(|_temp| {
         let store_id = "model_change_test";
         let meta_path = config::meta_dir().join(format!("{store_id}.json"));

         fs::create_dir_all(meta_path.parent().unwrap()).unwrap();

         let legacy = serde_json::json!({
            "files": {},
            "model": {
               "dense_model": "legacy-dense",
               "colbert_model": "legacy-colbert",
               "dense_dim": 128,
               "colbert_dim": 64,
            },
         });

         fs::write(&meta_path, serde_json::to_string(&legacy).unwrap()).unwrap();

         let mut store = MetaStore::load(store_id).unwrap();
         assert!(store.model_mismatch());
         assert!(store.index_mismatch());

         store.reset_for_signature_change();
         assert!(!store.model_mismatch());
         assert!(!store.index_mismatch());
         assert_eq!(store.all_paths().count(), 0);

         store.save().unwrap();

         let reloaded = MetaStore::load(store_id).unwrap();
         assert!(!reloaded.model_mismatch());
         assert!(!reloaded.index_mismatch());
      });
   }
}
