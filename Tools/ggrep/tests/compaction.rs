mod support;

use std::sync::Arc;

use ggrep::{
   chunker::Chunker,
   config,
   embed::{Embedder, DummyEmbedder},
   file::LocalFileSystem,
   identity,
   search::SearchEngine,
   snapshot::{SnapshotManager, compact_store, CompactionOptions},
   store::LanceStore,
   sync::SyncEngine,
   types::SearchMode,
};
use support::{set_temp_home};
use tempfile::TempDir;

#[tokio::test]
async fn tombstone_prune() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("keep.rs"), "pub fn keep() {}\n").expect("seed file");
   std::fs::write(root.join("drop.rs"), "pub fn drop_me() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "compaction-test";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder.clone(), store.clone());

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   std::fs::remove_file(root.join("drop.rs")).expect("delete file");
   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("sync delete");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = SnapshotManager::new(
      store.clone(),
      store_id.to_string(),
      fingerprints.config_fingerprint.clone(),
      fingerprints.ignore_fingerprint.clone(),
   );
   let active_id = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");
   let manifest =
      ggrep::snapshot::SnapshotManifest::load(&snapshot_manager.manifest_path(&active_id))
         .expect("manifest");
   assert!(manifest.tombstones.iter().map(|t| t.count).sum::<u64>() > 0);

   let compaction = compact_store(
      store.clone(),
      store_id,
      &fingerprints.config_fingerprint,
      &fingerprints.ignore_fingerprint,
      CompactionOptions { force: true, max_retries: 1 },
   )
   .await
   .expect("compaction");
   assert!(compaction.performed);

   let new_snapshot = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");
   let new_manifest =
      ggrep::snapshot::SnapshotManifest::load(&snapshot_manager.manifest_path(&new_snapshot))
         .expect("manifest");
   assert!(new_manifest.tombstones.is_empty());

   let search_engine = SearchEngine::new(store.clone(), embedder.clone());
   let snapshot_view = snapshot_manager.open_snapshot_view().await.expect("snapshot view");
   let include_anchors = config::get().fast_mode;
   let results_drop = search_engine
      .search_with_mode(
         &snapshot_view,
         store_id,
         "drop_me",
         5,
         5,
         None,
         false,
         include_anchors,
         SearchMode::Balanced,
      )
      .await
      .expect("search drop");
   assert!(results_drop.results.is_empty());

   let results_keep = search_engine
      .search_with_mode(
         &snapshot_view,
         store_id,
         "keep",
         5,
         5,
         None,
         false,
         include_anchors,
         SearchMode::Balanced,
      )
      .await
      .expect("search keep");
   assert!(!results_keep.results.is_empty());
}
