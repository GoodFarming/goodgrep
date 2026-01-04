mod support;

use std::sync::Arc;

use ggrep::{
   chunker::Chunker,
   config,
   embed::{Embedder, DummyEmbedder},
   file::LocalFileSystem,
   identity,
   meta::MetaStore,
   snapshot::{GcOptions, gc_snapshots},
   store::LanceStore,
   sync::SyncEngine,
};
use support::set_temp_home;
use tempfile::TempDir;

#[tokio::test]
async fn gc_retains_pinned_snapshots() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("file.rs"), "pub fn v0() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "gc-pinned-test";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder.clone(), store.clone());

   let mut snapshots = Vec::new();
   for i in 0..6 {
      std::fs::write(root.join("file.rs"), format!("pub fn v{i}() {{}}\n")).expect("write");
      sync_engine
         .initial_sync(store_id, root, None, false, &mut ())
         .await
         .expect("sync");
      let meta = MetaStore::load(store_id).expect("meta");
      let snapshot_id = meta.snapshot_id().expect("snapshot id").to_string();
      snapshots.push(snapshot_id);
   }

   let pinned = snapshots.first().cloned().expect("pinned snapshot");
   let identity = identity::compute_fingerprints(root).expect("fingerprints");
   let mut pinned_set = std::collections::HashSet::new();
   pinned_set.insert(pinned.clone());

   let report = gc_snapshots(
      store,
      store_id,
      &identity.config_fingerprint,
      &identity.ignore_fingerprint,
      GcOptions {
         dry_run: true,
         pinned: pinned_set,
         retain_snapshots_min: Some(1),
         retain_snapshots_min_age_secs: Some(0),
         safety_window_ms: Some(0),
         ..GcOptions::default()
      },
   )
   .await
   .expect("gc report");

   assert!(report.retained_snapshots.contains(&pinned));
   assert!(!report.deleted_snapshots.contains(&pinned));
}
