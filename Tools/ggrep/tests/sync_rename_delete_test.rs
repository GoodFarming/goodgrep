mod support;

use std::{path::PathBuf, sync::Arc};

use ggrep::{
   chunker::Chunker,
   config,
   embed::Embedder,
   file::LocalFileSystem,
   identity,
   meta::MetaStore,
   snapshot::SnapshotManager,
   store::LanceStore,
   sync::{ChangeSet, SyncEngine},
};
use support::{TestEmbedder, set_temp_home};
use tempfile::TempDir;

#[tokio::test]
async fn rename_and_delete_changes_apply_cleanly() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();

   std::fs::write(root.join("a.rs"), "pub fn a() {}\n").expect("seed file");
   std::fs::write(root.join("b.rs"), "pub fn b() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "rename-delete";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder::new(config::get().dense_dim));
   let chunker = Chunker::default();
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), chunker, Arc::clone(&embedder), Arc::clone(&store));

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   std::fs::rename(root.join("a.rs"), root.join("c.rs")).expect("rename");
   std::fs::remove_file(root.join("b.rs")).expect("delete");

   let changeset = ChangeSet {
      add:    Vec::new(),
      modify: Vec::new(),
      delete: vec![PathBuf::from("b.rs")],
      rename: vec![(PathBuf::from("a.rs"), PathBuf::from("c.rs"))],
   };

   sync_engine
      .initial_sync(store_id, root, Some(changeset), false, &mut ())
      .await
      .expect("rename/delete sync");

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
   assert!(snapshot_view.is_tombstoned("a.rs"));
   assert!(snapshot_view.is_tombstoned("b.rs"));
   assert!(!snapshot_view.is_tombstoned("c.rs"));

   let meta = MetaStore::load(store_id).expect("meta store");
   let mut paths: Vec<PathBuf> = meta.all_paths().cloned().collect();
   paths.sort();
   assert_eq!(paths, vec![PathBuf::from("c.rs")]);
}
