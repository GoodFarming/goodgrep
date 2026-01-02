//! Long-running daemon server command.
//!
//! Starts a background server that maintains an index, watches for file
//! changes, and responds to search requests over Unix domain sockets.
//! Automatically shuts down after a period of inactivity.

use std::{
   path::{Path, PathBuf},
   sync::{
      Arc,
      atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
   },
   time::{Duration, Instant},
};

use console::style;
use tokio::{
   signal,
   sync::{mpsc, watch},
   time,
};

use crate::{
   Result, config,
   embed::{Embedder, candle::CandleEmbedder},
   file::{FileWatcher, IgnorePatterns, LocalFileSystem},
   git,
   ipc::{self, Request, Response, ServerStatus},
   meta::MetaStore,
   search::SearchEngine,
   store::{LanceStore, Store},
   sync::SyncEngine,
   types::{SearchMode, SearchResponse, SearchResult, SearchStatus, SyncProgress},
   usock, version,
};

/// The main server state managing indexing, search, and file watching.
struct Server {
   store:         Arc<dyn Store>,
   embedder:      Arc<dyn Embedder>,
   store_id:      String,
   root:          PathBuf,
   indexing:      AtomicBool,
   progress:      AtomicU8,
   files:         AtomicUsize,
   launch_time:   Instant,
   last_activity: AtomicU64,
   shutdown:      watch::Sender<bool>,
}

impl Server {
   fn clock(&self) -> u64 {
      self.launch_time.elapsed().as_millis() as u64
   }

   fn touch(&self) {
      self
         .last_activity
         .fetch_max(self.clock(), Ordering::Relaxed);
   }

   fn idle_duration(&self) -> Duration {
      let timestamp = self
         .clock()
         .saturating_sub(self.last_activity.load(Ordering::Relaxed));
      Duration::from_millis(timestamp)
   }
}

struct PidFileGuard {
   store_id: String,
}

impl Drop for PidFileGuard {
   fn drop(&mut self) {
      usock::remove_pid(&self.store_id);
   }
}

fn count_indexed_files(store_id: &str, root: &Path) -> usize {
   MetaStore::load(store_id)
      .map(|meta| meta.all_paths().filter(|p| p.starts_with(root)).count())
      .unwrap_or(0)
}

fn pct_from_sync_progress(progress: &SyncProgress) -> u8 {
   if progress.total == 0 {
      return 100;
   }
   ((progress.processed.saturating_mul(100) / progress.total).min(100)) as u8
}

/// Executes the serve command, starting a long-running daemon server.
pub async fn execute(path: Option<PathBuf>, store_id: Option<String>) -> Result<()> {
   let cwd = std::env::current_dir()?.canonicalize()?;
   let default_root = git::get_repo_root(&cwd).unwrap_or_else(|| cwd.clone());
   let requested = path.unwrap_or(default_root).canonicalize()?;
   let serve_path = git::get_repo_root(&requested).unwrap_or(requested);
   let serve_path = serve_path.canonicalize().unwrap_or(serve_path);

   let resolved_store_id = store_id.map_or_else(|| git::resolve_store_id(&serve_path), Ok)?;

   let listener = match usock::Listener::bind(&resolved_store_id).await {
      Ok(l) => l,
      Err(e) if e.to_string().contains("already running") => {
         println!("{}", style("Server already running").yellow());
         return Ok(());
      },
      Err(e) => return Err(e),
   };

   usock::write_pid(&resolved_store_id);
   let _pid_guard = PidFileGuard { store_id: resolved_store_id.clone() };

   println!("{}", style("Starting ggrep server...").green().bold());
   println!("Listening: {}", style(listener.local_addr()).cyan());
   println!("Path: {}", style(serve_path.display()).dim());
   println!("Store ID: {}", style(&resolved_store_id).cyan());

   let store: Arc<dyn Store> = Arc::new(LanceStore::new()?);
   let embedder: Arc<dyn Embedder> = Arc::new(CandleEmbedder::new()?);

   if !embedder.is_ready() {
      println!("{}", style("Waiting for embedder to initialize...").yellow());
      time::sleep(Duration::from_millis(500)).await;
   }

   let (shutdown_tx, shutdown_rx) = watch::channel(false);
   let initial_files = count_indexed_files(&resolved_store_id, &serve_path);

   let server = Arc::new(Server {
      store,
      embedder,
      store_id: resolved_store_id,
      root: serve_path,
      indexing: AtomicBool::new(true),
      progress: AtomicU8::new(0),
      files: AtomicUsize::new(initial_files),
      last_activity: AtomicU64::new(0),
      launch_time: Instant::now(),
      shutdown: shutdown_tx.clone(),
   });

   let (sync_tx, sync_rx) = mpsc::unbounded_channel::<()>();
   let _ = sync_tx.send(());

   let sync_server = Arc::clone(&server);
   tokio::spawn(async move { sync_server.sync_loop(sync_rx).await });

   let _watcher = server.start_watcher(sync_tx)?;

   let idle_server = Arc::clone(&server);
   let idle_shutdown = shutdown_tx.clone();
   let cfg = config::get();
   let idle_timeout = Duration::from_secs(cfg.idle_timeout_secs);
   let idle_check_interval = Duration::from_secs(cfg.idle_check_interval_secs);
   tokio::spawn(async move {
      loop {
         time::sleep(idle_check_interval).await;
         if idle_server.indexing.load(Ordering::Relaxed) {
            continue;
         }
         if idle_server.idle_duration() > idle_timeout {
            println!("{}", style("Idle timeout reached, shutting down...").yellow());
            let _ = idle_shutdown.send(true);
            break;
         }
      }
   });

   println!("\n{}", style("Server listening").green());
   println!("{}", style("Press Ctrl+C to stop").dim());

   let accept_server = Arc::clone(&server);
   let mut accept_shutdown = shutdown_rx.clone();
   let accept_handle = tokio::spawn(async move {
      loop {
         tokio::select! {
            result = listener.accept() => {
               match result {
                  Ok(stream) => {
                     let client_server = Arc::clone(&accept_server);
                     tokio::spawn(async move { client_server.handle_client(stream).await });
                  }
                  Err(e) => {
                     tracing::error!("Accept error: {}", e);
                  }
               }
            }
            _ = accept_shutdown.changed() => {
               if *accept_shutdown.borrow() {
                  break;
               }
            }
         }
      }
   });

   tokio::select! {
      _ = signal::ctrl_c() => {
         println!("\n{}", style("Shutting down...").yellow());
         let _ = shutdown_tx.send(true);
      }
      () = async {
         let mut rx = shutdown_rx.clone();
         loop {
            rx.changed().await.ok();
            if *rx.borrow() {
               break;
            }
         }
      } => {}
   }

   accept_handle.abort();

   println!("{}", style("Server stopped").green());
   Ok(())
}

impl Server {
   async fn handle_client(self: &Arc<Self>, mut stream: usock::Stream) {
      self.touch();

      let mut buffer = ipc::SocketBuffer::new();
      let mut shutting_down = false;

      loop {
         let request: Request = match buffer.recv(&mut stream).await {
            Ok(req) => req,
            Err(e) => {
               if e.to_string().contains("failed to read length") {
                  break;
               }
               tracing::debug!("Client read error: {}", e);
               break;
            },
         };

         self.touch();

         let response = match request {
            Request::Hello { .. } => Response::Hello { git_hash: version::GIT_HASH.to_string() },
            Request::Search { query, limit, per_file, mode, path, rerank } => {
               self
                  .handle_search(query, limit, per_file, mode, path, rerank)
                  .await
            },
            Request::Health => Response::Health {
               status: ServerStatus {
                  indexing: self.indexing.load(Ordering::Relaxed),
                  progress: self.progress.load(Ordering::Relaxed),
                  files:    self.files.load(Ordering::Relaxed),
               },
            },
            Request::Shutdown => {
               shutting_down = true;
               Response::Shutdown { success: true }
            },
         };

         if let Err(e) = buffer.send(&mut stream, &response).await {
            tracing::debug!("Client write error: {}", e);
            break;
         }

         if shutting_down {
            let _ = self.shutdown.send(true);
            break;
         }
      }
   }

   async fn handle_search(
      &self,
      query: String,
      limit: usize,
      per_file: usize,
      mode: SearchMode,
      path: Option<PathBuf>,
      rerank: bool,
   ) -> Response {
      if query.is_empty() {
         return Response::Error { message: "query is required".to_string() };
      }

      let search_path = path.as_ref().map(|p| {
         if p.is_absolute() {
            p.clone()
         } else {
            self.root.join(p)
         }
      });

      let engine = SearchEngine::new(Arc::clone(&self.store), Arc::clone(&self.embedder));
      let store_id = self.store_id.as_str();
      let include_anchors = config::get().fast_mode;
      let search_result = engine
         .search_with_mode(
            store_id,
            &query,
            limit,
            per_file,
            search_path.as_deref(),
            rerank,
            include_anchors,
            mode,
         )
         .await;

      match search_result {
         Ok(response) => {
            let results = response
               .results
               .into_iter()
               .map(|r| {
                  let rel_path = r
                     .path
                     .strip_prefix(&self.root)
                     .map(PathBuf::from)
                     .unwrap_or(r.path);

                  SearchResult {
                     path:       rel_path,
                     content:    r.content,
                     score:      r.score,
                     start_line: r.start_line,
                     num_lines:  r.num_lines,
                     chunk_type: r.chunk_type,
                     is_anchor:  r.is_anchor,
                  }
               })
               .collect();

            let is_indexing = self.indexing.load(Ordering::Relaxed);
            let progress_val = self.progress.load(Ordering::Relaxed);

            Response::Search(SearchResponse {
               results,
               status: if is_indexing {
                  SearchStatus::Indexing
               } else {
                  SearchStatus::Ready
               },
               progress: if is_indexing {
                  Some(progress_val)
               } else {
                  None
               },
            })
         },
         Err(e) => Response::Error { message: format!("search failed: {e}") },
      }
   }

   async fn sync_loop(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<()>) {
      let mut shutdown_rx = self.shutdown.subscribe();

      loop {
         tokio::select! {
            _ = shutdown_rx.changed() => {
               if *shutdown_rx.borrow() {
                  break;
               }
            }
            msg = rx.recv() => {
               if msg.is_none() {
                  break;
               }

               // Debounce: drain bursts of change notifications into a single sync.
               loop {
                  match time::timeout(Duration::from_millis(250), rx.recv()).await {
                     Ok(Some(_)) => continue,
                     Ok(None) => break,
                     Err(_) => break,
                  }
               }

               if let Err(e) = self.sync_once().await {
                  tracing::error!("Sync failed: {}", e);
               }
            }
         }
      }
   }

   async fn sync_once(self: &Arc<Self>) -> Result<()> {
      self.indexing.store(true, Ordering::Relaxed);
      self.progress.store(0, Ordering::Relaxed);
      self.touch();

      let sync_engine = SyncEngine::new(
         LocalFileSystem::new(),
         crate::chunker::Chunker::default(),
         Arc::clone(&self.embedder),
         Arc::clone(&self.store),
      );

      let store_id = self.store_id.clone();
      let root = self.root.clone();
      let server = Arc::clone(self);
      let mut callback = move |p: SyncProgress| {
         server.touch();
         server
            .progress
            .store(pct_from_sync_progress(&p), Ordering::Relaxed);
      };

      let result = sync_engine
         .initial_sync(&store_id, &root, false, &mut callback)
         .await;

      match result {
         Ok(_) => {
            self.progress.store(100, Ordering::Relaxed);
            self
               .files
               .store(count_indexed_files(&self.store_id, &self.root), Ordering::Relaxed);
            self.indexing.store(false, Ordering::Relaxed);
            Ok(())
         },
         Err(e) => {
            self.indexing.store(false, Ordering::Relaxed);
            Err(e)
         },
      }
   }

   fn start_watcher(self: &Arc<Self>, sync_tx: mpsc::UnboundedSender<()>) -> Result<FileWatcher> {
      let ignore_patterns = IgnorePatterns::new(&self.root);
      let server = Arc::clone(self);
      let watcher = FileWatcher::new(self.root.clone(), ignore_patterns, move |_changes| {
         server.touch();
         let _ = sync_tx.send(());
      })?;

      Ok(watcher)
   }
}
