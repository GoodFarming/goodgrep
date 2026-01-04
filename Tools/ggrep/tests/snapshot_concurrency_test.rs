mod support;

use std::{
   path::Path,
   sync::{
      Arc,
      atomic::{AtomicBool, Ordering},
   },
   time::Duration,
};

use ggrep::{
   chunker::Chunker,
   config,
   embed::Embedder,
   file::LocalFileSystem,
   identity,
   search::SearchEngine,
   snapshot::SnapshotManager,
   store::LanceStore,
   sync::SyncEngine,
};
use support::{TestEmbedder, set_temp_home};
use tempfile::TempDir;
use tokio::time;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn queries_during_publish_are_snapshot_consistent() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("old.rs"), "pub fn old() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "snapshot-consistency";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder.clone(), store.clone());

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = Arc::new(SnapshotManager::new(
      store.clone(),
      store_id.to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   ));
   let snapshot1_id = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");

   let engine = Arc::new(SearchEngine::new(store.clone(), embedder));
   let running = Arc::new(AtomicBool::new(true));
   let include_anchors = true;

   let mut handles = Vec::new();
   for _ in 0..4 {
      let engine = Arc::clone(&engine);
      let running = Arc::clone(&running);
      let snapshot_manager = Arc::clone(&snapshot_manager);
      let snapshot1_id = snapshot1_id.clone();
      let store_id = store_id.to_string();
      handles.push(tokio::spawn(async move {
         while running.load(Ordering::Relaxed) {
            let snapshot_view = match snapshot_manager.open_snapshot_view().await {
               Ok(view) => view,
               Err(e) => return Err(format!("snapshot error: {e}")),
            };
            let response = engine
               .search(
                  &snapshot_view,
                  &store_id,
                  "new",
                  5,
                  2,
                  Some(Path::new("new.rs")),
                  false,
                  include_anchors,
               )
               .await
               .map_err(|e| format!("search error: {e}"))?;

            if snapshot_view.snapshot_id == snapshot1_id {
               if !response.results.is_empty() {
                  return Err("saw new file results in old snapshot".to_string());
               }
            } else if response.results.is_empty() {
               return Err("expected new file results in new snapshot".to_string());
            }

            time::sleep(Duration::from_millis(10)).await;
         }
         Ok::<(), String>(())
      }));
   }

   std::fs::write(root.join("new.rs"), "pub fn new_feature() {}\n").expect("new file");
   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("sync publish");

   time::sleep(Duration::from_millis(200)).await;
   running.store(false, Ordering::Relaxed);

   for handle in handles {
      handle.await.expect("task join").expect("search loop");
   }
}
