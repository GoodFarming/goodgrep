mod support;

use std::sync::Arc;

use chrono::Utc;
use ggrep::{
   chunker::Chunker,
   config,
   embed::Embedder,
   file::LocalFileSystem,
   identity,
   lease::{WriterLease, WriterLeaseRecord},
   snapshot::{SnapshotManager, SnapshotManifest},
   store::LanceStore,
   sync::SyncEngine,
};
use support::{TestEmbedder, set_temp_home};
use tempfile::TempDir;

#[tokio::test]
async fn publish_fails_after_lease_steal() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("main.rs"), "fn main() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "lease-preflight";
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
   let manifest =
      SnapshotManifest::load(&snapshot_manager.manifest_path(&snapshot_id)).expect("manifest");

   let lease = WriterLease::acquire(store_id)
      .await
      .expect("lease acquire");
   let lease_owner = lease.owner_id().to_string();
   let lease_epoch = lease.lease_epoch();

   let lease_path = config::data_dir()
      .join(store_id)
      .join("locks")
      .join("writer_lease.json");
   let mut record: WriterLeaseRecord =
      serde_json::from_str(&std::fs::read_to_string(&lease_path).expect("lease read"))
         .expect("lease parse");
   record.owner_id = "stolen-writer".to_string();
   record.lease_epoch = record.lease_epoch.saturating_add(1);
   let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
   record.started_at = now.clone();
   record.last_heartbeat_at = now;
   std::fs::write(&lease_path, serde_json::to_string_pretty(&record).expect("lease json"))
      .expect("lease write");

   let result = snapshot_manager
      .publish_manifest(&manifest, &lease_owner, lease_epoch)
      .await;
   let message = format!("{}", result.expect_err("publish should fail"));
   assert!(message.contains("lease ownership lost"));
}
