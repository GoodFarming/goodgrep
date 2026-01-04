mod support;

use std::sync::Arc;

use ggrep::{
   chunker::Chunker,
   config,
   embed::Embedder,
   file::{LocalFileSystem, resolve_candidate},
   identity,
   meta::MetaStore,
   search::SearchEngine,
   snapshot::SnapshotManager,
   store::LanceStore,
   sync::{ChangeSet, SyncEngine},
};
use support::{TestEmbedder, set_temp_home};
use tempfile::TempDir;

#[cfg(unix)]
#[tokio::test]
async fn toctou_out_of_root_swap_deletes_file() {
   use std::os::unix::fs as unix_fs;

   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   let external = TempDir::new().expect("external");

   let file_path = root.join("safe.rs");
   std::fs::write(&file_path, "pub fn safe() {}\n").expect("write file");

   let external_file = external.path().join("evil.rs");
   std::fs::write(&external_file, "pub fn evil() {}\n").expect("external file");

   config::init_for_root(root);

   let store_id = "toctou";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder::new(config::get().dense_dim));
   let sync_engine = SyncEngine::new(
      LocalFileSystem::new(),
      Chunker::default(),
      Arc::clone(&embedder),
      Arc::clone(&store),
   );

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   let resolved = resolve_candidate(root, &file_path)
      .expect("resolve")
      .expect("resolved");

   std::fs::remove_file(&file_path).expect("remove file");
   unix_fs::symlink(&external_file, &file_path).expect("symlink");

   let changeset = ChangeSet {
      add:    Vec::new(),
      modify: vec![resolved],
      delete: Vec::new(),
      rename: Vec::new(),
   };

   sync_engine
      .initial_sync(store_id, root, Some(changeset), false, &mut ())
      .await
      .expect("sync after swap");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = SnapshotManager::new(
      store.clone(),
      store_id.to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   );
   let snapshot_view = snapshot_manager
      .open_snapshot_view()
      .await
      .expect("snapshot view");
   assert!(snapshot_view.is_tombstoned("safe.rs"));

   let engine = SearchEngine::new(store.clone(), embedder);
   let include_anchors = config::get().fast_mode;
   let response = engine
      .search(&snapshot_view, store_id, "pub fn safe", 5, 2, None, false, include_anchors)
      .await
      .expect("search");
   assert!(response.results.is_empty());

   let meta = MetaStore::load(store_id).expect("meta store");
   assert!(meta.all_paths().next().is_none());
}
