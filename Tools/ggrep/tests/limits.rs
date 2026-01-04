mod support;

use std::sync::Arc;

use ggrep::{
   chunker::Chunker,
   cmd::serve,
   config,
   embed::{Embedder, DummyEmbedder},
   file::LocalFileSystem,
   identity,
   ipc::{Request, Response},
   store::LanceStore,
   sync::SyncEngine,
   types::SearchMode,
   usock,
};
use support::set_temp_home;
use tempfile::TempDir;
use tokio::time;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_handle_budget() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   // Safe in test harness: isolate config paths and avoid global side effects.
   unsafe {
      std::env::set_var("GGREP_DUMMY_EMBEDDER", "1");
      std::env::set_var("GGREP_TEST_QUERY_DELAY_MS", "200");
      std::env::set_var("GGREP_MAX_OPEN_SEGMENTS_GLOBAL", "1");
      std::env::set_var("GGREP_MAX_OPEN_SEGMENTS_PER_QUERY", "1");
      std::env::set_var("GGREP_MAX_CONCURRENT_QUERIES", "2");
   }

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();
   std::fs::write(root.join("a.rs"), "pub fn alpha() {}\n").expect("seed file");

   config::init_for_root(root);

   let store_id = "open-handle-test";
   let store = Arc::new(LanceStore::new().expect("store"));
   let embedder: Arc<dyn Embedder> = Arc::new(DummyEmbedder::new(config::get().dense_dim));
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder.clone(), store.clone());

   sync_engine
      .initial_sync(store_id, root, None, false, &mut ())
      .await
      .expect("initial sync");

   let identity = identity::resolve_index_identity(root).expect("identity");
   let fingerprint = identity.config_fingerprint.clone();
   let server_root = root.to_path_buf();
   let server_store = store_id.to_string();
   let _server = tokio::spawn(async move {
      let _ = serve::execute(Some(server_root), Some(server_store), false).await;
   });

   wait_for_daemon(store_id).await;

   let q1 = tokio::spawn(run_query(store_id.to_string(), fingerprint.clone(), "alpha".to_string()));
   let q2 = tokio::spawn(run_query(store_id.to_string(), fingerprint.clone(), "alpha".to_string()));
   let (r1, r2) = tokio::join!(q1, q2);
   let r1 = r1.expect("join");
   let r2 = r2.expect("join");

   let mut ok = 0;
   let mut busy = 0;
   for response in [r1, r2] {
      match response {
         Response::Search(_) => ok += 1,
         Response::Error { code, .. } if code == "busy" => busy += 1,
         other => panic!("unexpected response: {other:?}"),
      }
   }

   assert_eq!(ok, 1);
   assert_eq!(busy, 1);

   let _ = shutdown_daemon(store_id, &fingerprint).await;
}

async fn wait_for_daemon(store_id: &str) {
   for _ in 0..50 {
      if usock::Stream::connect(store_id).await.is_ok() {
         return;
      }
      time::sleep(std::time::Duration::from_millis(50)).await;
   }
   panic!("daemon did not start");
}

async fn run_query(store_id: String, fingerprint: String, query: String) -> Response {
   let mut stream = usock::Stream::connect(&store_id).await.expect("connect");
   let mut buffer = ggrep::ipc::SocketBuffer::new();
   let hello = ggrep::ipc::client_hello(
      &store_id,
      &fingerprint,
      Some(ggrep::ipc::default_client_id("ggrep-test")),
      ggrep::ipc::default_client_capabilities(),
   );
   buffer.send(&mut stream, &hello).await.expect("hello send");
   let response: Response = buffer
      .recv_with_limit(&mut stream, config::get().max_response_bytes)
      .await
      .expect("hello recv");
   assert!(matches!(response, Response::Hello { .. }));

   buffer
      .send(
         &mut stream,
         &Request::Search {
            query,
            limit: 5,
            per_file: 5,
            mode: SearchMode::Balanced,
            path: None,
            rerank: false,
         },
      )
      .await
      .expect("search send");
   buffer
      .recv_with_limit(&mut stream, config::get().max_response_bytes)
      .await
      .expect("search recv")
}

async fn shutdown_daemon(store_id: &str, fingerprint: &str) -> ggrep::Result<()> {
   let mut stream = usock::Stream::connect(store_id).await?;
   let mut buffer = ggrep::ipc::SocketBuffer::new();
   let hello = ggrep::ipc::client_hello(
      store_id,
      fingerprint,
      Some(ggrep::ipc::default_client_id("ggrep-test")),
      ggrep::ipc::default_client_capabilities(),
   );
   buffer.send(&mut stream, &hello).await?;
   let _response: Response =
      buffer.recv_with_limit(&mut stream, config::get().max_response_bytes).await?;
   buffer.send(&mut stream, &Request::Shutdown).await?;
   let _response: Response =
      buffer.recv_with_limit(&mut stream, config::get().max_response_bytes).await?;
   Ok(())
}
