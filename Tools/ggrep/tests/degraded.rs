mod support;

use std::sync::{
   Arc,
   atomic::{AtomicBool, Ordering},
};

use ggrep::{
   chunker::Chunker,
   config,
   embed::{Embedder, HybridEmbedding, QueryEmbedding},
   file::LocalFileSystem,
   identity,
   search::SearchEngine,
   snapshot::SnapshotManager,
   store::LanceStore,
   sync::{SyncEngine, SyncOptions},
   types::SearchMode,
   Error, Str,
};
use ndarray::Array2;
use support::set_temp_home;
use tempfile::TempDir;

#[derive(Debug)]
struct FlakyEmbedder {
   dense_dim: usize,
   fail_token: Option<String>,
   fail_once: AtomicBool,
}

impl FlakyEmbedder {
   fn new(dense_dim: usize) -> Self {
      Self { dense_dim, fail_token: None, fail_once: AtomicBool::new(false) }
   }

   fn fail_on(mut self, token: &str) -> Self {
      self.fail_token = Some(token.to_string());
      self
   }

   fn fail_once(mut self) -> Self {
      self.fail_once.store(true, Ordering::SeqCst);
      self
   }

   fn embed_text(&self, text: &Str) -> HybridEmbedding {
      let mut dense = vec![0.0; self.dense_dim];
      if !dense.is_empty() {
         dense[0] = text.as_str().len() as f32;
      }
      HybridEmbedding { dense, colbert: Vec::new(), colbert_scale: 1.0 }
   }
}

#[async_trait::async_trait]
impl Embedder for FlakyEmbedder {
   async fn compute_hybrid(&self, texts: &[Str]) -> ggrep::Result<Vec<HybridEmbedding>> {
      if let Some(token) = &self.fail_token {
         if texts.iter().any(|t| t.as_str().contains(token)) {
            return Err(
               Error::Server { op: "embed", reason: "forced failure".to_string() }.into(),
            );
         }
      }
      if self.fail_once.swap(false, Ordering::SeqCst) {
         return Err(Error::Server { op: "embed", reason: "transient failure".to_string() }.into());
      }
      Ok(texts.iter().map(|t| self.embed_text(t)).collect())
   }

   async fn encode_query(&self, text: &str) -> ggrep::Result<QueryEmbedding> {
      let mut dense = vec![0.0; self.dense_dim];
      if !dense.is_empty() {
         dense[0] = text.len() as f32;
      }
      Ok(QueryEmbedding { dense, colbert: Array2::zeros((0, 0)) })
   }

   fn is_ready(&self) -> bool {
      true
   }
}

#[tokio::test]
async fn allow_degraded_publishes_with_errors() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("good.rs"), "fn good() {}\n// token_good").expect("good file");
   std::fs::write(root.join("bad.rs"), "FAIL token_bad").expect("bad file");

   config::init_for_root(root);

   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> =
      Arc::new(FlakyEmbedder::new(config::get().dense_dim).fail_on("FAIL"));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder.clone(), store.clone());

   sync_engine
      .initial_sync_with_options(
         "degraded-test",
         root,
         None,
         false,
         SyncOptions { allow_degraded: true, embed_max_retries: 0, embed_backoff_ms: 0 },
         &mut (),
      )
      .await
      .expect("degraded sync");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = SnapshotManager::new(
      store.clone(),
      "degraded-test".to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   );

   let active = snapshot_manager
      .read_active_snapshot_id()
      .expect("read snapshot")
      .expect("snapshot id");
   let manifest =
      ggrep::snapshot::SnapshotManifest::load(&snapshot_manager.manifest_path(&active))
         .expect("manifest");

   assert!(manifest.degraded, "manifest should be degraded");
   assert!(manifest.errors.iter().any(|e| e.path_key.ends_with("bad.rs")));

   let snapshot_view = snapshot_manager.open_snapshot_view().await.expect("snapshot view");
   let search_engine = SearchEngine::new(store.clone(), embedder);
   let include_anchors = config::get().fast_mode;

   let good_results = search_engine
      .search_with_mode(
         &snapshot_view,
         "degraded-test",
         "token_good",
         5,
         5,
         None,
         false,
         include_anchors,
         SearchMode::Balanced,
      )
      .await
      .expect("search good");
   assert!(
      good_results.results.iter().any(|r| r.path.ends_with("good.rs")),
      "good.rs should be indexed"
   );

   let bad_results = search_engine
      .search_with_mode(
         &snapshot_view,
         "degraded-test",
         "token_bad",
         5,
         5,
         None,
         false,
         include_anchors,
         SearchMode::Balanced,
      )
      .await
      .expect("search bad");
   assert!(
      bad_results.results.iter().all(|r| !r.path.ends_with("bad.rs")),
      "bad.rs should be missing from degraded snapshot"
   );
}

#[tokio::test]
async fn embed_retry_recovers() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("ok.rs"), "fn ok() {}\n// token_ok").expect("ok file");

   config::init_for_root(root);

   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> =
      Arc::new(FlakyEmbedder::new(config::get().dense_dim).fail_once());
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder.clone(), store.clone());

   sync_engine
      .initial_sync_with_options(
         "retry-test",
         root,
         None,
         false,
         SyncOptions { allow_degraded: false, embed_max_retries: 1, embed_backoff_ms: 0 },
         &mut (),
      )
      .await
      .expect("sync with retry");

   let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
   let snapshot_manager = SnapshotManager::new(
      store,
      "retry-test".to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   );
   let active = snapshot_manager
      .read_active_snapshot_id()
      .expect("read snapshot")
      .expect("snapshot id");
   let manifest =
      ggrep::snapshot::SnapshotManifest::load(&snapshot_manager.manifest_path(&active))
         .expect("manifest");
   assert!(!manifest.degraded, "manifest should not be degraded after retry");
}
