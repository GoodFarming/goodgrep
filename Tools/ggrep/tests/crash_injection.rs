#![cfg(feature = "failpoints")]

mod support;

use std::sync::Arc;

use fail::FailScenario;
use ggrep::{
   chunker::Chunker,
   config,
   embed::{Embedder, DummyEmbedder},
   file::LocalFileSystem,
   identity,
   lease::WriterLease,
   snapshot::{CompactionOptions, SnapshotManager, compact_store, gc_snapshots, GcOptions},
   store::LanceStore,
   sync::SyncEngine,
};
use support::set_temp_home;
use tempfile::TempDir;

#[tokio::test]
async fn publish_failpoint_preserves_active_snapshot() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("main.rs"), "fn main() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "failpoint-publish";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder, store.clone());

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = SnapshotManager::new(
      store.clone(),
      store_id.to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   );
   let active_before = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");
   let mut manifest =
      ggrep::snapshot::SnapshotManifest::load(&snapshot_manager.manifest_path(&active_before))
         .expect("manifest");
   manifest.snapshot_id = uuid::Uuid::new_v4().to_string();
   manifest.parent_snapshot_id = Some(active_before.clone());
   manifest.created_at =
      chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

   let _guard = FailScenario::setup();
   fail::cfg("publish.before_pointer_swap", "return").unwrap();

   let lease = WriterLease::acquire(store_id).await.expect("lease");
   let err = snapshot_manager
      .publish_manifest(&manifest, lease.owner_id(), lease.lease_epoch())
      .await
      .expect_err("expected failpoint error");
   let msg = err.to_string();
   assert!(msg.contains("failpoint"));

   let active_after = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");
   assert_eq!(active_before, active_after);
}

#[tokio::test]
async fn compaction_failpoint_preserves_active_snapshot() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("main.rs"), "fn main() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "failpoint-compaction";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder, store.clone());

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = SnapshotManager::new(
      store.clone(),
      store_id.to_string(),
      fingerprints.config_fingerprint.clone(),
      fingerprints.ignore_fingerprint.clone(),
   );
   let active_before = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");

   let _guard = FailScenario::setup();
   fail::cfg("compaction.before_publish", "return").unwrap();

   let result = compact_store(
      store,
      store_id,
      &fingerprints.config_fingerprint,
      &fingerprints.ignore_fingerprint,
      CompactionOptions { force: true, max_retries: 1 },
   )
   .await;
   assert!(result.is_err());

   let active_after = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");
   assert_eq!(active_before, active_after);
}

#[tokio::test]
async fn gc_failpoint_returns_error() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("main.rs"), "fn main() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "failpoint-gc";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder, store.clone());

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let _guard = FailScenario::setup();
   fail::cfg("gc.before_delete", "return").unwrap();

   let result = gc_snapshots(
      store,
      store_id,
      &fingerprints.config_fingerprint,
      &fingerprints.ignore_fingerprint,
      GcOptions { dry_run: false, ..GcOptions::default() },
   )
   .await;
   assert!(result.is_err());
}
