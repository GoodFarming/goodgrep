//! Long-running daemon server command.
//!
//! Starts a background server that maintains an index, watches for file
//! changes, and responds to search requests over Unix domain sockets.
//! Automatically shuts down after a period of inactivity.

use std::{
   collections::{HashMap, HashSet, VecDeque},
   path::{Path, PathBuf},
   sync::{
      Arc,
      atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
   },
   time::{Duration, Instant},
};

use console::style;
use parking_lot::Mutex as ParkingMutex;
use tokio::{
   signal,
   sync::{Mutex, RwLock, mpsc, watch},
   time,
};

use crate::{
   Result, config,
   embed::{Embedder, DummyEmbedder, candle::CandleEmbedder},
   file::{
      FileWatcher, IgnorePatterns, LocalFileSystem, WatchAction, normalize_relative,
      resolve_candidate,
   },
   identity,
   ipc::{self, Request, Response, ServerStatus},
   meta::MetaStore,
   snapshot::{
      CompactionOptions, SnapshotManager, SnapshotManifest, compaction_overdue, compact_store,
      gc_snapshots, pins::SnapshotPins, GcOptions,
   },
   search::SearchEngine,
   store::LanceStore,
   sync::{ChangeSet, SyncEngine, SyncOptions},
   types::{SearchMode, SearchResponse, SearchResult, SearchStatus, SearchTimings, SyncProgress},
   usock,
   util::sanitize_output,
   version,
};

const PERF_WINDOW: usize = 200;

/// The main server state managing indexing, search, and file watching.
struct Server {
   store: Arc<LanceStore>,
   embedder: Arc<dyn Embedder>,
   store_id: String,
   config_fingerprint: String,
   ignore_fingerprint: String,
   repo_config_hash: Option<String>,
   root: PathBuf,
   indexing: AtomicBool,
   progress: AtomicU8,
   files: AtomicUsize,
   query_sem: Arc<tokio::sync::Semaphore>,
   queued_queries: AtomicUsize,
   max_concurrent_queries: usize,
   max_query_queue: usize,
   max_concurrent_queries_per_client: usize,
   query_timeout: Duration,
   slow_query_ms: u64,
   open_handles_sem: Arc<tokio::sync::Semaphore>,
   max_open_segments_per_query: usize,
   max_open_segments_global: usize,
   client_limits: Mutex<HashMap<String, Arc<ClientLimiter>>>,
   snapshot_meta: RwLock<SnapshotMeta>,
   snapshot_pins: SnapshotPins,
   allow_degraded: bool,
   compaction_in_progress: AtomicBool,
   perf_metrics: ParkingMutex<PerfMetrics>,
   query_total: AtomicU64,
   busy_total: AtomicU64,
   timeouts_total: AtomicU64,
   slow_total: AtomicU64,
   launch_time: Instant,
   last_activity: AtomicU64,
   shutdown: watch::Sender<bool>,
}

struct ClientLimiter {
   sem: Arc<tokio::sync::Semaphore>,
}

#[derive(Clone, Default)]
struct SnapshotMeta {
   snapshot_id: Option<String>,
   created_at:  Option<String>,
}

impl Server {
   async fn config_watch_loop(self: Arc<Self>) {
      const CHECK_INTERVAL_MS: u64 = 500;
      const DEBOUNCE_MS: u64 = 2000;
      const DEBOUNCE_MS_LOW_IMPACT: u64 = 4000;

      let debounce_ms = if config::get().low_impact {
         DEBOUNCE_MS_LOW_IMPACT
      } else {
         DEBOUNCE_MS
      };

      let mut shutdown_rx = self.shutdown.subscribe();
      let mut tick = time::interval(Duration::from_millis(CHECK_INTERVAL_MS));
      let mut pending_since: Option<Instant> = None;

      loop {
         tokio::select! {
            _ = shutdown_rx.changed() => {
               if *shutdown_rx.borrow() {
                  break;
               }
            }
            _ = tick.tick() => {
               let repo_hash = match identity::compute_repo_config_hash(&self.root) {
                  Ok(hash) => hash,
                  Err(e) => {
                     tracing::warn!("config watch: failed to read repo config hash: {}", e);
                     continue;
                  }
               };
               let ignore_hash = match identity::compute_ignore_fingerprint(&self.root) {
                  Ok(hash) => hash,
                  Err(e) => {
                     tracing::warn!("config watch: failed to compute ignore fingerprint: {}", e);
                     continue;
                  }
               };

               let repo_changed = repo_hash.as_deref() != self.repo_config_hash.as_deref();
               let ignore_changed = ignore_hash != self.ignore_fingerprint;
               if repo_changed || ignore_changed {
                  let now = Instant::now();
                  pending_since = match pending_since {
                     None => Some(now),
                     Some(since) => {
                        if now.duration_since(since).as_millis() as u64 >= debounce_ms {
                           tracing::warn!(
                              "config/ignore fingerprint changed; shutting down daemon for restart"
                           );
                           let _ = self.shutdown.send(true);
                           break;
                        }
                        Some(since)
                     }
                  };
               } else {
                  pending_since = None;
               }
            }
         }
      }
   }

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

   fn pin_snapshot(&self, snapshot_id: &str) -> SnapshotPinGuard<'_> {
      self.snapshot_pins.pin(snapshot_id);
      SnapshotPinGuard { server: self, snapshot_id: snapshot_id.to_string() }
   }

   fn unpin_snapshot(&self, snapshot_id: &str) {
      self.snapshot_pins.unpin(snapshot_id);
   }

   fn pinned_snapshot_ids(&self) -> HashSet<String> {
      self.snapshot_pins.ids()
   }

   fn record_perf(&self, latency_ms: u64, segments: usize) {
      let mut metrics = self.perf_metrics.lock();
      metrics.record(latency_ms, segments, PERF_WINDOW);
   }

   fn perf_snapshot(&self) -> (u64, u64, u64) {
      let metrics = self.perf_metrics.lock();
      metrics.snapshot()
   }

   fn maybe_schedule_compaction(self: &Arc<Self>) {
      if self.compaction_in_progress.swap(true, Ordering::AcqRel) {
         return;
      }

      let server = Arc::clone(self);
      tokio::spawn(async move {
         let snapshot_manager = SnapshotManager::new(
            Arc::clone(&server.store),
            server.store_id.clone(),
            server.config_fingerprint.clone(),
            server.ignore_fingerprint.clone(),
         );
         let mut should_compact = false;
         if let Ok(Some(active)) = snapshot_manager.read_active_snapshot_id() {
            let manifest_path = snapshot_manager.manifest_path(&active);
            if let Ok(manifest) = SnapshotManifest::load(&manifest_path) {
               should_compact = compaction_overdue(&manifest);
            }
         }

         if should_compact {
            let _ = compact_store(
               Arc::clone(&server.store),
               &server.store_id,
               &server.config_fingerprint,
               &server.ignore_fingerprint,
               CompactionOptions { force: false, max_retries: 1 },
            )
            .await;
         }

         server.compaction_in_progress.store(false, Ordering::Release);
      });
   }
}

struct SnapshotPinGuard<'a> {
   server:     &'a Server,
   snapshot_id: String,
}

impl Drop for SnapshotPinGuard<'_> {
   fn drop(&mut self) {
      self.server.unpin_snapshot(&self.snapshot_id);
   }
}

struct PerfMetrics {
   latencies_ms:      VecDeque<u64>,
   segments_touched:  VecDeque<usize>,
}

impl PerfMetrics {
   fn new() -> Self {
      Self { latencies_ms: VecDeque::new(), segments_touched: VecDeque::new() }
   }

   fn record(&mut self, latency_ms: u64, segments: usize, max_len: usize) {
      self.latencies_ms.push_back(latency_ms);
      self.segments_touched.push_back(segments);
      while self.latencies_ms.len() > max_len {
         self.latencies_ms.pop_front();
      }
      while self.segments_touched.len() > max_len {
         self.segments_touched.pop_front();
      }
   }

   fn snapshot(&self) -> (u64, u64, u64) {
      let mut latencies: Vec<u64> = self.latencies_ms.iter().copied().collect();
      let segments: Vec<u64> = self.segments_touched.iter().map(|v| *v as u64).collect();

      let p50 = percentile(&mut latencies, 0.50);
      let p95 = percentile(&mut latencies, 0.95);
      let max_segments = segments.into_iter().max().unwrap_or(0);
      (p50, p95, max_segments)
   }
}

fn percentile(values: &mut Vec<u64>, percentile: f64) -> u64 {
   if values.is_empty() {
      return 0;
   }
   values.sort_unstable();
   let idx = ((values.len() - 1) as f64 * percentile).round() as usize;
   values[idx]
}

struct PidFileGuard {
   store_id: String,
}

impl Drop for PidFileGuard {
   fn drop(&mut self) {
      usock::remove_pid(&self.store_id);
   }
}

enum SyncSignal {
   Events(Vec<(PathBuf, WatchAction)>),
   Reconcile,
}

fn count_indexed_files(store_id: &str, _root: &Path) -> usize {
   MetaStore::load(store_id)
      .map(|meta| meta.all_paths().count())
      .unwrap_or(0)
}

fn pct_from_sync_progress(progress: &SyncProgress) -> u8 {
   if progress.total == 0 {
      return 100;
   }
   ((progress.processed.saturating_mul(100) / progress.total).min(100)) as u8
}

/// Executes the serve command, starting a long-running daemon server.
pub async fn execute(
   path: Option<PathBuf>,
   store_id: Option<String>,
   allow_degraded: bool,
) -> Result<()> {
   let cwd = std::env::current_dir()?.canonicalize()?;
   let requested = path.unwrap_or(cwd).canonicalize()?;
   let index_identity = identity::resolve_index_identity(&requested)?;
   let serve_path = index_identity.canonical_root.clone();

   let identity::IndexIdentity {
      store_id: default_store_id,
      config_fingerprint,
      ignore_fingerprint,
      repo_config_hash,
      ..
   } = index_identity;

   let resolved_store_id = store_id.unwrap_or(default_store_id);

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

   let store: Arc<LanceStore> = Arc::new(LanceStore::new()?);
   let embedder: Arc<dyn Embedder> = if std::env::var("GGREP_DUMMY_EMBEDDER").is_ok() {
      Arc::new(DummyEmbedder::new(config::get().dense_dim))
   } else {
      Arc::new(CandleEmbedder::new()?)
   };

   if !embedder.is_ready() {
      println!("{}", style("Waiting for embedder to initialize...").yellow());
      time::sleep(Duration::from_millis(500)).await;
   }

   let (shutdown_tx, shutdown_rx) = watch::channel(false);
   let initial_files = count_indexed_files(&resolved_store_id, &serve_path);

   let cfg = config::get();
   let snapshot_meta = MetaStore::load(&resolved_store_id)
      .ok()
      .map(|meta| SnapshotMeta {
         snapshot_id: meta.snapshot_id().map(|s| s.to_string()),
         created_at:  meta.snapshot_created_at().map(|s| s.to_string()),
      })
      .unwrap_or_default();
   let server = Arc::new(Server {
      store,
      embedder,
      store_id: resolved_store_id,
      config_fingerprint,
      ignore_fingerprint,
      repo_config_hash,
      root: serve_path,
      indexing: AtomicBool::new(true),
      progress: AtomicU8::new(0),
      files: AtomicUsize::new(initial_files),
      query_sem: Arc::new(tokio::sync::Semaphore::new(cfg.max_concurrent_queries)),
      queued_queries: AtomicUsize::new(0),
      max_concurrent_queries: cfg.max_concurrent_queries,
      max_query_queue: cfg.max_query_queue,
      max_concurrent_queries_per_client: cfg.effective_max_concurrent_queries_per_client(),
      query_timeout: Duration::from_millis(cfg.query_timeout_ms),
      slow_query_ms: cfg.slow_query_ms,
      open_handles_sem: Arc::new(tokio::sync::Semaphore::new(
         cfg.effective_max_open_segments_global(),
      )),
      max_open_segments_per_query: cfg.effective_max_open_segments_per_query(),
      max_open_segments_global: cfg.effective_max_open_segments_global(),
      client_limits: Mutex::new(HashMap::new()),
      snapshot_meta: RwLock::new(snapshot_meta),
      snapshot_pins: SnapshotPins::default(),
      allow_degraded,
      compaction_in_progress: AtomicBool::new(false),
      perf_metrics: ParkingMutex::new(PerfMetrics::new()),
      query_total: AtomicU64::new(0),
      busy_total: AtomicU64::new(0),
      timeouts_total: AtomicU64::new(0),
      slow_total: AtomicU64::new(0),
      last_activity: AtomicU64::new(0),
      launch_time: Instant::now(),
      shutdown: shutdown_tx.clone(),
   });

   let (sync_tx, sync_rx) = mpsc::unbounded_channel::<SyncSignal>();
   let _ = sync_tx.send(SyncSignal::Reconcile);

   let sync_server = Arc::clone(&server);
   tokio::spawn(async move { sync_server.sync_loop(sync_rx).await });

   let config_server = Arc::clone(&server);
   tokio::spawn(async move { config_server.config_watch_loop().await });

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

   #[cfg(unix)]
   let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate()).ok();
   #[cfg(unix)]
   let sigterm_fut = async {
      if let Some(sigterm) = &mut sigterm {
         let _ = sigterm.recv().await;
      } else {
         std::future::pending::<()>().await;
      }
   };
   #[cfg(not(unix))]
   let sigterm_fut = async { std::future::pending::<()>().await };

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
      _ = sigterm_fut => {
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
      let mut handshake_done = false;
      let mut client_id: Option<String> = None;

      loop {
         let request: Request = match buffer
            .recv_with_limit(&mut stream, config::get().max_request_bytes)
            .await
         {
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

         let response = if !handshake_done {
            match request {
               Request::Hello {
                  protocol_versions,
                  store_id,
                  config_fingerprint,
                  client_id: hello_client_id,
                  ..
               } => {
                  client_id = hello_client_id;
                  let response =
                     self.handle_handshake(protocol_versions, store_id, config_fingerprint);
                  if matches!(response, Response::Hello { .. }) {
                     handshake_done = true;
                  }
                  response
               },
               _ => Response::Error {
                  code:    "invalid_request".to_string(),
                  message: "handshake required before other requests".to_string(),
               },
            }
         } else {
            match request {
               Request::Hello {
                  protocol_versions,
                  store_id,
                  config_fingerprint,
                  client_id: hello_client_id,
                  ..
               } => {
                  client_id = hello_client_id;
                  self.handle_handshake(protocol_versions, store_id, config_fingerprint)
               },
               Request::Search { query, limit, per_file, mode, path, rerank } => {
                  self
                     .handle_search(
                        query,
                        limit,
                        per_file,
                        mode,
                        path,
                        rerank,
                        client_id.as_deref(),
                     )
                     .await
               },
               Request::Health => {
                  let (p50, p95, max_segments) = self.perf_snapshot();
                  Response::Health {
                     status: ServerStatus {
                     indexing:          self.indexing.load(Ordering::Relaxed),
                     progress:          self.progress.load(Ordering::Relaxed),
                     files:             self.files.load(Ordering::Relaxed),
                     queries_in_flight: self
                        .max_concurrent_queries
                        .saturating_sub(self.query_sem.available_permits()),
                     queries_queued:    self.queued_queries.load(Ordering::Relaxed),
                     busy_total:        self.busy_total.load(Ordering::Relaxed),
                     timeouts_total:    self.timeouts_total.load(Ordering::Relaxed),
                     slow_total:        self.slow_total.load(Ordering::Relaxed),
                     query_latency_p50_ms: p50,
                     query_latency_p95_ms: p95,
                     segments_touched_max: max_segments,
                     segments_open:     self
                        .max_open_segments_global
                        .saturating_sub(self.open_handles_sem.available_permits())
                        as u64,
                     segments_budget:   self.max_open_segments_global as u64,
                  },
               }
               },
               Request::Gc { dry_run } => self.handle_gc(dry_run).await,
               Request::Shutdown => {
                  shutting_down = true;
                  Response::Shutdown { success: true }
               },
            }
         };

         if let Err(e) = buffer.send(&mut stream, &response).await {
            tracing::debug!("Client write error: {}", e);
            break;
         }

         if shutting_down {
            let _ = self.shutdown.send(true);
            break;
         }

         if !handshake_done {
            break;
         }
      }
   }

   fn handle_handshake(
      &self,
      client_versions: Vec<u32>,
      store_id: String,
      config_fingerprint: String,
   ) -> Response {
      handshake_response(
         &self.store_id,
         &self.config_fingerprint,
         &client_versions,
         &store_id,
         &config_fingerprint,
      )
   }

   async fn handle_search(
      &self,
      query: String,
      limit: usize,
      per_file: usize,
      mode: SearchMode,
      path: Option<PathBuf>,
      rerank: bool,
      client_id: Option<&str>,
   ) -> Response {
      if query.is_empty() {
         return Response::Error {
            code:    "invalid_request".to_string(),
            message: "query is required".to_string(),
         };
      }

      self.query_total.fetch_add(1, Ordering::Relaxed);

      let cfg = config::get();
      let limit = limit.min(cfg.max_query_results).max(1);
      let per_file = per_file.min(cfg.max_query_per_file).max(1);

      let deadline = Instant::now() + self.query_timeout;

      let client_permit = match self.admit_client(client_id).await {
         Ok(permit) => permit,
         Err(response) => return response,
      };

      let (permit, admission_ms) = match self.admit_query(deadline).await {
         Ok(p) => p,
         Err(response) => {
            drop(client_permit);
            return response;
         },
      };

      let open_handle_permit = match self.admit_open_handles() {
         Ok(permit) => permit,
         Err(response) => {
            drop(permit);
            drop(client_permit);
            return response;
         },
      };

      if let Some(delay_ms) = std::env::var("GGREP_TEST_QUERY_DELAY_MS")
         .ok()
         .and_then(|v| v.parse::<u64>().ok())
      {
         time::sleep(Duration::from_millis(delay_ms)).await;
      }

      let search_path = path.as_ref().map(|p| {
         if p.is_absolute() {
            p.clone()
         } else {
            self.root.join(p)
         }
      });

      let engine = SearchEngine::new(Arc::clone(&self.store), Arc::clone(&self.embedder));
      let snapshot_start = Instant::now();
      let snapshot_manager = SnapshotManager::new(
         Arc::clone(&self.store),
         self.store_id.clone(),
         self.config_fingerprint.clone(),
         self.ignore_fingerprint.clone(),
      );
      let snapshot_view = match snapshot_manager.open_snapshot_view().await {
         Ok(view) => view,
         Err(e) => {
            drop(open_handle_permit);
            drop(permit);
            drop(client_permit);
            return Response::Error {
               code: "invalid_request".to_string(),
               message: format!("snapshot error: {e}"),
            };
         }
      };
      let _pin = self.pin_snapshot(&snapshot_view.snapshot_id);
      let snapshot_read_ms = snapshot_start.elapsed().as_millis() as u64;
      let store_id = self.store_id.as_str();
      let include_anchors = config::get().fast_mode;
      let segments_touched = snapshot_view.segment_tables().len();
      let remaining = deadline.saturating_duration_since(Instant::now());
      if remaining.is_zero() {
         self.timeouts_total.fetch_add(1, Ordering::Relaxed);
         return Response::Error {
            code:    "timeout".to_string(),
            message: "query timeout exceeded".to_string(),
         };
      }

      let mut shutdown_rx = self.shutdown.subscribe();
      let search_fut = engine.search_with_mode(
         &snapshot_view,
         store_id,
         &query,
         limit,
         per_file,
         search_path.as_deref(),
         rerank,
         include_anchors,
         mode,
      );

      let query_start = Instant::now();
      let search_result = tokio::select! {
         _ = shutdown_rx.changed() => {
            return Response::Error {
               code: "cancelled".to_string(),
               message: "query cancelled due to shutdown".to_string(),
            };
         }
         result = time::timeout(remaining, search_fut) => {
            match result {
               Ok(r) => r,
               Err(_) => {
                  self.timeouts_total.fetch_add(1, Ordering::Relaxed);
                  return Response::Error {
                     code: "timeout".to_string(),
                     message: "query timeout exceeded".to_string(),
                  };
               }
            }
         }
      };

      drop(open_handle_permit);
      drop(permit);
      drop(client_permit);
      let elapsed = query_start.elapsed();
      let elapsed_ms = elapsed.as_millis() as u64;
      if elapsed_ms > self.slow_query_ms {
         self.slow_total.fetch_add(1, Ordering::Relaxed);
      }

      match search_result {
         Ok(mut response) => {
            self.record_perf(elapsed_ms, segments_touched);
            let timings = response.timings_ms.take();
            let timings = timings.map(|mut t| {
               t.snapshot_read_ms = snapshot_read_ms;
               t
            });
            let limits_hit = response
               .limits_hit
               .into_iter()
               .map(|mut hit| {
                  hit.code = sanitize_output(&hit.code);
                  if let Some(path_key) = hit.path_key.take() {
                     let path = PathBuf::from(path_key);
                     let rel_path = path
                        .strip_prefix(&self.root)
                        .map(PathBuf::from)
                        .unwrap_or(path);
                     hit.path_key = Some(sanitize_output(&rel_path.to_string_lossy()));
                  }
                  hit
               })
               .collect::<Vec<_>>();
            let warnings = response
               .warnings
               .into_iter()
               .map(|mut warning| {
                  warning.code = sanitize_output(&warning.code);
                  warning.message = sanitize_output(&warning.message);
                  if let Some(path_key) = warning.path_key.take() {
                     let path = PathBuf::from(path_key);
                     let rel_path = path
                        .strip_prefix(&self.root)
                        .map(PathBuf::from)
                        .unwrap_or(path);
                     warning.path_key = Some(sanitize_output(&rel_path.to_string_lossy()));
                  }
                  warning
               })
               .collect::<Vec<_>>();
            let results = response
               .results
               .into_iter()
               .map(|r| {
                  let rel_path = r
                     .path
                     .strip_prefix(&self.root)
                     .map(PathBuf::from)
                     .unwrap_or(r.path);
                  let sanitized_path = PathBuf::from(sanitize_output(&rel_path.to_string_lossy()));
                  let sanitized_content =
                     crate::Str::from_string(sanitize_output(r.content.as_str()));

                  SearchResult {
                     path:            sanitized_path,
                     content:         sanitized_content,
                     score:           r.score,
                     secondary_score: r.secondary_score,
                     row_id:          r.row_id.clone(),
                     segment_table:   r.segment_table.clone(),
                     start_line:      r.start_line,
                     num_lines:       r.num_lines,
                     chunk_type:      r.chunk_type,
                     is_anchor:       r.is_anchor,
                  }
               })
               .collect();

            let is_indexing = self.indexing.load(Ordering::Relaxed);
            let progress_val = self.progress.load(Ordering::Relaxed);

            let timings_ms = timings
               .map(|mut t| {
                  t.admission_ms = admission_ms;
                  t
               })
               .or_else(|| {
                  Some(SearchTimings {
                     admission_ms,
                     snapshot_read_ms,
                     ..SearchTimings::default()
                  })
               });

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
               timings_ms,
               limits_hit,
               warnings,
            })
         },
         Err(e) => Response::Error {
            code:    "internal".to_string(),
            message: format!("search failed: {e}"),
         },
      }
   }

   async fn handle_gc(&self, dry_run: bool) -> Response {
      if self.indexing.load(Ordering::Relaxed) {
         return Response::Error {
            code:    "busy".to_string(),
            message: "indexing in progress".to_string(),
         };
      }

      let in_flight = self
         .max_concurrent_queries
         .saturating_sub(self.query_sem.available_permits());
      if in_flight > 0 {
         return Response::Error {
            code:    "busy".to_string(),
            message: "queries in flight".to_string(),
         };
      }

      let pinned = self.pinned_snapshot_ids();
      let report = gc_snapshots(
         Arc::clone(&self.store),
         &self.store_id,
         &self.config_fingerprint,
         &self.ignore_fingerprint,
         GcOptions { dry_run, pinned, active_snapshot: None, ..GcOptions::default() },
      )
      .await;

      match report {
         Ok(report) => Response::Gc { report },
         Err(e) => Response::Error {
            code:    "internal".to_string(),
            message: format!("gc failed: {e}"),
         },
      }
   }

   async fn admit_query(
      &self,
      deadline: Instant,
   ) -> Result<(tokio::sync::OwnedSemaphorePermit, u64), Response> {
      let start = Instant::now();
      if let Ok(permit) = self.query_sem.clone().try_acquire_owned() {
         let elapsed = start.elapsed().as_millis() as u64;
         return Ok((permit, elapsed));
      }

      if self.max_query_queue == 0 {
         self.busy_total.fetch_add(1, Ordering::Relaxed);
         return Err(Response::Error {
            code:    "busy".to_string(),
            message: "daemon busy".to_string(),
         });
      }

      let queued = self.queued_queries.fetch_add(1, Ordering::AcqRel) + 1;
      if queued > self.max_query_queue {
         self.queued_queries.fetch_sub(1, Ordering::AcqRel);
         self.busy_total.fetch_add(1, Ordering::Relaxed);
         return Err(Response::Error {
            code:    "busy".to_string(),
            message: "daemon busy".to_string(),
         });
      }

      let mut shutdown_rx = self.shutdown.subscribe();
      let permit = tokio::select! {
         _ = shutdown_rx.changed() => {
            self.queued_queries.fetch_sub(1, Ordering::AcqRel);
            return Err(Response::Error {
               code: "cancelled".to_string(),
               message: "query cancelled due to shutdown".to_string(),
            });
         }
         result = time::timeout_at(time::Instant::from_std(deadline), self.query_sem.clone().acquire_owned()) => {
            match result {
               Ok(Ok(permit)) => permit,
               Ok(Err(_)) => {
                  self.queued_queries.fetch_sub(1, Ordering::AcqRel);
                  return Err(Response::Error {
                     code: "internal".to_string(),
                     message: "failed to admit query".to_string(),
                  });
               }
               Err(_) => {
                  self.queued_queries.fetch_sub(1, Ordering::AcqRel);
                  self.timeouts_total.fetch_add(1, Ordering::Relaxed);
                  return Err(Response::Error {
                     code: "timeout".to_string(),
                     message: "query timeout exceeded".to_string(),
                  });
               }
            }
         }
      };

      self.queued_queries.fetch_sub(1, Ordering::AcqRel);
      let elapsed = start.elapsed().as_millis() as u64;
      Ok((permit, elapsed))
   }

   async fn admit_client(
      &self,
      client_id: Option<&str>,
   ) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, Response> {
      let Some(client_id) = client_id else {
         return Ok(None);
      };

      if self.max_concurrent_queries_per_client == 0 {
         return Ok(None);
      }

      let limiter = {
         let mut limits = self.client_limits.lock().await;
         limits
            .entry(client_id.to_string())
            .or_insert_with(|| {
               Arc::new(ClientLimiter {
                  sem: Arc::new(tokio::sync::Semaphore::new(
                     self.max_concurrent_queries_per_client,
                  )),
               })
            })
            .clone()
      };

      match limiter.sem.clone().try_acquire_owned() {
         Ok(permit) => Ok(Some(permit)),
         Err(_) => {
            self.busy_total.fetch_add(1, Ordering::Relaxed);
            Err(Response::Error {
               code:    "busy".to_string(),
               message: "client concurrency limit reached".to_string(),
            })
         },
      }
   }

   fn admit_open_handles(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Response> {
      if self.max_open_segments_per_query == 0 || self.max_open_segments_global == 0 {
         return Err(Response::Error {
            code:    "internal".to_string(),
            message: "open handle budget disabled".to_string(),
         });
      }

      let needed = self
         .max_open_segments_per_query
         .min(self.max_open_segments_global);

      match self
         .open_handles_sem
         .clone()
         .try_acquire_many_owned(needed as u32)
      {
         Ok(permit) => Ok(permit),
         Err(_) => {
            self.busy_total.fetch_add(1, Ordering::Relaxed);
            Err(Response::Error {
               code:    "busy".to_string(),
               message: "open handle budget exceeded".to_string(),
            })
         },
      }
   }

   async fn sync_loop(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<SyncSignal>) {
      const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);
      const RECONCILE_INTERVAL: Duration = Duration::from_secs(300);
      const IDLE_RECONCILE_DELAY: Duration = Duration::from_secs(120);

      let mut shutdown_rx = self.shutdown.subscribe();
      let mut pending: HashMap<PathBuf, WatchAction> = HashMap::new();
      let mut reconcile_tick = time::interval(RECONCILE_INTERVAL);
      let idle_timer = time::sleep(IDLE_RECONCILE_DELAY);
      tokio::pin!(idle_timer);
      let mut last_full_reconcile = Instant::now() - RECONCILE_INTERVAL;

      loop {
         tokio::select! {
            _ = shutdown_rx.changed() => {
               if *shutdown_rx.borrow() {
                  break;
               }
            }
            _ = reconcile_tick.tick() => {
               let now = Instant::now();
               if pending.is_empty()
                  && !self.indexing.load(Ordering::Relaxed)
                  && now.duration_since(last_full_reconcile) >= RECONCILE_INTERVAL
               {
                  if let Err(e) = self.sync_once(None).await {
                     tracing::error!("Reconciliation sync failed: {}", e);
                  } else {
                     last_full_reconcile = now;
                  }
               }
            }
            _ = &mut idle_timer => {
               let now = Instant::now();
               if pending.is_empty()
                  && !self.indexing.load(Ordering::Relaxed)
                  && now.duration_since(last_full_reconcile) >= IDLE_RECONCILE_DELAY
               {
                  if let Err(e) = self.sync_once(None).await {
                     tracing::error!("Idle reconciliation sync failed: {}", e);
                  } else {
                     last_full_reconcile = now;
                  }
               }
               idle_timer.as_mut().reset(time::Instant::now() + IDLE_RECONCILE_DELAY);
            }
            msg = rx.recv() => {
               let Some(signal) = msg else {
                  break;
               };

               let mut force_reconcile = false;
               match signal {
                  SyncSignal::Reconcile => {
                     force_reconcile = true;
                  }
                  SyncSignal::Events(changes) => {
                     for (path, action) in changes {
                        pending.insert(path, action);
                     }
                     idle_timer.as_mut().reset(time::Instant::now() + IDLE_RECONCILE_DELAY);
                  }
               }

               // Debounce: drain bursts of change notifications into a single sync.
               loop {
                  match time::timeout(DEBOUNCE_WINDOW, rx.recv()).await {
                     Ok(Some(SyncSignal::Events(changes))) => {
                        for (path, action) in changes {
                           pending.insert(path, action);
                        }
                        idle_timer.as_mut().reset(time::Instant::now() + IDLE_RECONCILE_DELAY);
                     }
                     Ok(Some(SyncSignal::Reconcile)) => {
                        force_reconcile = true;
                        break;
                     }
                     Ok(None) => break,
                     Err(_) => break,
                  }
               }

               if !pending.is_empty() {
                  let changeset = self.build_changeset(&pending);
                  if changeset.is_empty() {
                     pending.clear();
                  } else {
                     match self.sync_once(Some(changeset)).await {
                        Ok(_) => pending.clear(),
                        Err(e) => tracing::error!("Sync failed: {}", e),
                     }
                  }
               }

               if force_reconcile {
                  match self.sync_once(None).await {
                     Ok(_) => last_full_reconcile = Instant::now(),
                     Err(e) => tracing::error!("Reconciliation sync failed: {}", e),
                  }
               }
            }
         }
      }
   }

   async fn sync_once(self: &Arc<Self>, changeset: Option<ChangeSet>) -> Result<()> {
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

      let sync_start = Instant::now();
      let result = sync_engine
         .initial_sync_with_options(
            &store_id,
            &root,
            changeset,
            false,
            SyncOptions { allow_degraded: self.allow_degraded, ..SyncOptions::default() },
            &mut callback,
         )
         .await;

      match result {
         Ok(_) => {
            self.progress.store(100, Ordering::Relaxed);
            self
               .files
               .store(count_indexed_files(&self.store_id, &self.root), Ordering::Relaxed);
            self.indexing.store(false, Ordering::Relaxed);
            if let Ok(meta) = MetaStore::load(&self.store_id) {
               let mut snapshot_meta = self.snapshot_meta.write().await;
               snapshot_meta.snapshot_id = meta.snapshot_id().map(|s| s.to_string());
               snapshot_meta.created_at = meta.snapshot_created_at().map(|s| s.to_string());
            }
            self.maybe_schedule_compaction();
            Ok(())
         },
         Err(e) => {
            self.indexing.store(false, Ordering::Relaxed);
            let duration_ms = sync_start.elapsed().as_millis() as u64;
            if let Ok(mut meta_store) = MetaStore::load(&self.store_id) {
               meta_store.record_sync("error", duration_ms);
               let _ = meta_store.save();
            }
            Err(e)
         },
      }
   }

   fn build_changeset(&self, pending: &HashMap<PathBuf, WatchAction>) -> ChangeSet {
      let root = &self.root;
      let mut changeset = ChangeSet::default();

      for (path, action) in pending {
         match action {
            WatchAction::Upsert => {
               if path.is_dir() {
                  continue;
               }
               match resolve_candidate(root, path) {
                  Ok(Some(resolved)) => changeset.modify.push(resolved),
                  Ok(None) => {},
                  Err(e) => {
                     tracing::warn!("failed to resolve watcher path {}: {e}", path.display())
                  },
               }
            },
            WatchAction::Delete => {
               let full_path = if path.is_absolute() {
                  path.clone()
               } else {
                  root.join(path)
               };
               if let Ok(relative) = full_path.strip_prefix(root)
                  && let Some(path_key) = normalize_relative(relative)
               {
                  changeset.delete.push(path_key);
               }
            },
         }
      }

      changeset.delete.sort();
      changeset.delete.dedup();
      changeset.modify.sort_by(|a, b| a.path_key.cmp(&b.path_key));
      changeset.modify.dedup_by(|a, b| a.path_key == b.path_key);

      changeset
   }

   fn start_watcher(
      self: &Arc<Self>,
      sync_tx: mpsc::UnboundedSender<SyncSignal>,
   ) -> Result<FileWatcher> {
      let ignore_patterns = IgnorePatterns::new(&self.root);
      let server = Arc::clone(self);
      let watcher = FileWatcher::new(self.root.clone(), ignore_patterns, move |changes| {
         server.touch();
         let _ = sync_tx.send(SyncSignal::Events(changes));
      })?;

      Ok(watcher)
   }
}

fn handshake_response(
   server_store_id: &str,
   server_fingerprint: &str,
   client_versions: &[u32],
   client_store_id: &str,
   client_fingerprint: &str,
) -> Response {
   if client_store_id != server_store_id {
      return Response::Error {
         code:    "invalid_request".to_string(),
         message: "store_id mismatch".to_string(),
      };
   }
   if client_fingerprint != server_fingerprint {
      return Response::Error {
         code:    "invalid_request".to_string(),
         message: "config_fingerprint mismatch".to_string(),
      };
   }

   let Some(protocol_version) = ipc::negotiate_protocol(client_versions) else {
      return Response::Error {
         code:    "incompatible".to_string(),
         message: "no compatible protocol version".to_string(),
      };
   };

   Response::Hello {
      protocol_version,
      protocol_versions: ipc::PROTOCOL_VERSIONS.to_vec(),
      binary_version: version::VERSION.to_string(),
      supported_schema_versions: ipc::SupportedSchemaVersions::current(),
      store_id: server_store_id.to_string(),
      config_fingerprint: server_fingerprint.to_string(),
   }
}

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn handshake_mismatch_store_id_returns_invalid_request() {
      let response = handshake_response("store-a", "cfg", &[2], "store-b", "cfg");
      match response {
         Response::Error { code, .. } => assert_eq!(code, "invalid_request"),
         _ => panic!("expected invalid_request error"),
      }
   }

   #[test]
   fn handshake_mismatch_config_returns_invalid_request() {
      let response = handshake_response("store-a", "cfg-a", &[2], "store-a", "cfg-b");
      match response {
         Response::Error { code, .. } => assert_eq!(code, "invalid_request"),
         _ => panic!("expected invalid_request error"),
      }
   }
}
