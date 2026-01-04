mod support;

use std::{sync::Arc, time::Duration};

use ggrep::{
   chunker::Chunker,
   config,
   embed::{Embedder, DummyEmbedder},
   file::LocalFileSystem,
   identity,
   reader_lock::ReaderLock,
   search::SearchEngine,
   snapshot::SnapshotManager,
   store::LanceStore,
   sync::SyncEngine,
   types::SearchMode,
};
use support::set_temp_home;
use tempfile::TempDir;

#[tokio::test]
async fn tombstone_structural_no_bypass() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("gone.rs"), "pub fn gone() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "tombstone-structural";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder.clone(), store.clone());

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   std::fs::remove_file(root.join("gone.rs")).expect("delete file");
   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("sync delete");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = SnapshotManager::new(
      store.clone(),
      store_id.to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   );
   let snapshot_view = snapshot_manager.open_snapshot_view().await.expect("snapshot view");

   let search_engine = SearchEngine::new(store.clone(), embedder.clone());
   let include_anchors = config::get().fast_mode;
   let results = search_engine
      .search_with_mode(
         &snapshot_view,
         store_id,
         "gone",
         5,
         5,
         None,
         false,
         include_anchors,
         SearchMode::Balanced,
      )
      .await
      .expect("search");

   assert!(results.results.is_empty());
}

#[test]
fn offline_reader_lock_gc_exclusive() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let store_id = "lock-test";
   let shared = ReaderLock::acquire_shared(store_id).expect("shared lock");

   let (tx, rx) = std::sync::mpsc::channel();
   let handle = std::thread::spawn(move || {
      let _exclusive = ReaderLock::acquire_exclusive(store_id).expect("exclusive lock");
      let _ = tx.send(());
   });

   assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
   drop(shared);
   assert!(rx.recv_timeout(Duration::from_secs(1)).is_ok());
   handle.join().expect("thread join");
}
