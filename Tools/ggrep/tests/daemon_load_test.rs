use std::{
   sync::{
      Arc,
      atomic::{AtomicBool, Ordering},
   },
   time::Duration,
};

mod support;

use ggrep::{
   chunker::Chunker, config, embed::Embedder, file::LocalFileSystem, identity,
   search::SearchEngine, snapshot::SnapshotManager, store::LanceStore, sync::SyncEngine,
   types::SearchMode,
};
use support::{TestEmbedder, set_temp_home};
use tempfile::TempDir;
use tokio::time;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn load_test_queries_during_sync_publish() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();

   std::fs::write(root.join("main.rs"), "fn main() { println!(\"hi\"); }\n").expect("seed file");
   std::fs::write(root.join("lib.rs"), "pub fn helper() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "load-test";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder::new(config::get().dense_dim));
   let chunker = Chunker::default();

   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), chunker, Arc::clone(&embedder), Arc::clone(&store));

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   let search_engine = Arc::new(SearchEngine::new(Arc::clone(&store), Arc::clone(&embedder)));
   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = Arc::new(SnapshotManager::new(
      Arc::clone(&store),
      store_id.to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   ));
   let include_anchors = config::get().fast_mode;
   let running = Arc::new(AtomicBool::new(true));

   let mut handles = Vec::new();
   for _ in 0..4 {
      let engine = Arc::clone(&search_engine);
      let running = Arc::clone(&running);
      let store_id = store_id.to_string();
      let snapshot_manager = Arc::clone(&snapshot_manager);
      let include_anchors = include_anchors;
      handles.push(tokio::spawn(async move {
         while running.load(Ordering::Relaxed) {
            let snapshot_view = match snapshot_manager.open_snapshot_view().await {
               Ok(view) => view,
               Err(e) => return Err(format!("snapshot error: {e}")),
            };
            let result = time::timeout(
               Duration::from_secs(2),
               engine.search_with_mode(
                  &snapshot_view,
                  &store_id,
                  "fn main",
                  5,
                  2,
                  None,
                  false,
                  include_anchors,
                  SearchMode::Balanced,
               ),
            )
            .await;

            match result {
               Ok(Ok(_)) => {},
               Ok(Err(e)) => return Err(format!("search failed: {e}")),
               Err(_) => return Err("search timed out".to_string()),
            }

            time::sleep(Duration::from_millis(10)).await;
         }
         Ok::<(), String>(())
      }));
   }

   for iter in 0..3 {
      std::fs::write(root.join("main.rs"), format!("fn main() {{ println!(\"iter {iter}\"); }}\n"))
         .expect("update file");
      if iter % 2 == 0 {
         std::fs::write(root.join(format!("extra_{iter}.rs")), "pub fn extra() {}\n")
            .expect("extra file");
      }
      sync_engine
         .initial_sync(store_id, root, None, false, &mut ())
         .await
         .expect("sync publish");
   }

   running.store(false, Ordering::Relaxed);
   for handle in handles {
      handle.await.expect("task join").expect("search loop");
   }
}
