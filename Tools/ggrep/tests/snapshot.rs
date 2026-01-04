mod support;

use std::sync::Arc;

use ggrep::{
   chunker::Chunker,
   config,
   embed::Embedder,
   file::LocalFileSystem,
   identity,
   snapshot::{SnapshotManager, SnapshotManifest},
   store::LanceStore,
   sync::SyncEngine,
};
use support::{TestEmbedder, set_temp_home};
use tempfile::TempDir;
use uuid::Uuid;

#[tokio::test]
async fn manifest_verification_detects_tamper() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("main.rs"), "fn main() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "snapshot-verify";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder::new(config::get().dense_dim));
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

   let snapshot_id = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");
   let mut manifest =
      SnapshotManifest::load(&snapshot_manager.manifest_path(&snapshot_id)).expect("manifest");
   snapshot_manager
      .verify_manifest(&manifest)
      .await
      .expect("manifest verify");

   manifest.counts.chunks_indexed = manifest.counts.chunks_indexed.saturating_add(1);
   let err = snapshot_manager
      .verify_manifest(&manifest)
      .await
      .expect_err("tampered manifest rejected");
   let message = format!("{err}");
   assert!(message.contains("chunk counts mismatch"));
}

#[tokio::test]
async fn staging_does_not_change_active_snapshot() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("main.rs"), "fn main() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "snapshot-staging";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder, store.clone());

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = SnapshotManager::new(
      store,
      store_id.to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   );

   let before = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");
   let staging_txn_id = format!("staging-{}", Uuid::new_v4());
   snapshot_manager
      .create_staging(&staging_txn_id)
      .expect("create staging");
   let after = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");
   assert_eq!(before, after);
}

#[tokio::test]
async fn checksum_verification() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("main.rs"), "fn main() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "snapshot-checksum";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(TestEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder, store.clone());

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = SnapshotManager::new(
      store,
      store_id.to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   );

   let snapshot_id = snapshot_manager
      .read_active_snapshot_id()
      .expect("active snapshot id")
      .expect("active snapshot");
   let mut manifest =
      SnapshotManifest::load(&snapshot_manager.manifest_path(&snapshot_id)).expect("manifest");

   if let Some(segment) = manifest.segments.first_mut() {
      segment.sha256 = "deadbeef".to_string();
   }

   let err = snapshot_manager
      .verify_manifest(&manifest)
      .await
      .expect_err("checksum mismatch rejected");
   let message = format!("{err}");
   assert!(message.contains("checksum mismatch"));
}
