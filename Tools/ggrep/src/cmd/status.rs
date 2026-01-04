//! Server status command.
//!
//! Displays the status of all running ggrep daemon servers.

use std::{fs, time::Duration};

use chrono::{DateTime, Utc};
use console::style;
use serde::Serialize;
use tokio::time;

use crate::{
   Result,
   cmd::daemon::{HandshakeOutcome, client_handshake},
   config,
   embed::limiter,
   git, identity,
   ipc::{self, Request, Response},
   meta::MetaStore,
   usock, util,
};

/// Executes the status command to show running servers.
pub async fn execute(json: bool) -> Result<()> {
   if json {
      return execute_json().await;
   }

   const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
   const RPC_TIMEOUT: Duration = Duration::from_millis(2000);

   let servers = usock::list_running_servers();

   if servers.is_empty() {
      println!("{}", style("No servers running").dim());
      return Ok(());
   }

   println!("{}", style("Running servers:").bold());
   println!();

   let mut buffer = ipc::SocketBuffer::new();
   for store_id in servers {
      let stream = match time::timeout(CONNECT_TIMEOUT, usock::Stream::connect(&store_id)).await {
         Ok(Ok(s)) => s,
         Ok(Err(_)) | Err(_) => {
            println!("  {} {} {}", style("●").red(), store_id, style("(stale)").dim());
            continue;
         },
      };

      let mut stream = stream;

      let config_fingerprint = MetaStore::load(&store_id)
         .ok()
         .and_then(|meta| meta.config_fingerprint().map(|s| s.to_string()))
         .unwrap_or_default();

      let handshake = time::timeout(
         RPC_TIMEOUT,
         client_handshake(&mut stream, &store_id, &config_fingerprint, "ggrep-status"),
      )
      .await;

      if !matches!(handshake, Ok(Ok(HandshakeOutcome::Compatible))) {
         println!("  {} {} {}", style("●").yellow(), store_id, style("(incompatible)").dim());
         continue;
      }

      let sent = time::timeout(RPC_TIMEOUT, buffer.send(&mut stream, &Request::Health)).await;
      if !matches!(sent, Ok(Ok(()))) {
         println!("  {} {} {}", style("●").yellow(), store_id, style("(unresponsive)").dim());
         continue;
      }

      let recv = time::timeout(
         RPC_TIMEOUT,
         buffer.recv_with_limit(&mut stream, config::get().max_response_bytes),
      )
      .await;
      match recv {
         Ok(Ok(Response::Health { status })) => {
            let state = if status.indexing {
               format!("indexing {}%", status.progress)
            } else {
               "ready".to_string()
            };
            println!(
               "  {} {} {}",
               style("●").green(),
               store_id,
               style(format!("({state}, files: {})", status.files)).dim()
            );
         },
         Ok(Ok(_)) => {
            println!("  {} {} {}", style("●").yellow(), store_id, style("(unknown)").dim());
         },
         Ok(Err(_)) | Err(_) => {
            println!("  {} {} {}", style("●").yellow(), store_id, style("(unresponsive)").dim());
         },
      }
   }

   Ok(())
}

#[derive(Serialize)]
struct StatusJson {
   schema_version:     u32,
   store_id:           String,
   canonical_root:     String,
   config_fingerprint: String,
   ignore_fingerprint: String,
   daemon:             DaemonJson,
   snapshot:           SnapshotJson,
   sync:               SyncJson,
   queries:            QueriesJson,
   resources:          ResourcesJson,
   performance:        PerformanceJson,
}

#[derive(Serialize)]
struct DaemonJson {
   running: bool,
   pid: Option<u32>,
   started_at: Option<String>,
   binary_version: Option<String>,
   protocol_version: Option<u32>,
   stale: bool,
   supported_schema_versions: Option<ipc::SupportedSchemaVersions>,
}

#[derive(Serialize)]
struct SnapshotJson {
   active_snapshot_id: Option<String>,
   head_sha:           Option<String>,
   dirty:              Option<bool>,
   untracked_included: Option<bool>,
   degraded:           bool,
   created_at:         Option<String>,
}

#[derive(Serialize)]
struct SyncJson {
   state:            String,
   last_sync_at:     Option<String>,
   last_result:      Option<String>,
   last_duration_ms: Option<u64>,
   staging_txn_id:   Option<String>,
}

#[derive(Serialize)]
struct QueriesJson {
   max_concurrent:  usize,
   max_queue_depth: usize,
   timeout_ms:      u64,
   in_flight:       usize,
   queue_depth:     usize,
   busy_total:      u64,
   timeouts_total:  u64,
   slow_total:      u64,
}

#[derive(Serialize)]
struct ResourcesJson {
   embed_global: EmbedGlobalJson,
   disk:         DiskJson,
   open_handles: OpenHandlesJson,
}

#[derive(Serialize)]
struct PerformanceJson {
   query_latency_p50_ms:      Option<u64>,
   query_latency_p95_ms:      Option<u64>,
   query_latency_budget_p50_ms: u64,
   query_latency_budget_p95_ms: u64,
   segments_touched_max:      Option<u64>,
   segments_touched_budget:   u64,
   publish_time_last_ms:      Option<u64>,
   publish_time_budget_ms:    u64,
   gc_time_last_ms:           Option<u64>,
   gc_time_budget_ms:         u64,
   compaction_time_last_ms:   Option<u64>,
   compaction_time_budget_ms: u64,
}

#[derive(Serialize)]
struct EmbedGlobalJson {
   max_concurrent: u32,
   in_use:         u32,
   stale_lock:     bool,
}

#[derive(Serialize)]
struct DiskJson {
   store_bytes:        u64,
   store_budget_bytes: u64,
   cache_bytes:        u64,
   cache_budget_bytes: u64,
   log_bytes:          u64,
   log_budget_bytes:   u64,
}

#[derive(Serialize)]
struct OpenHandlesJson {
   segments_open:   u64,
   segments_budget: u64,
}

async fn execute_json() -> Result<()> {
   let cwd = std::env::current_dir()?;
   println!("{}", collect_status_json(&cwd, true).await?);
   Ok(())
}

pub(crate) async fn collect_status_json(path: &std::path::Path, pretty: bool) -> Result<String> {
   const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
   const RPC_TIMEOUT: Duration = Duration::from_millis(2000);

   let identity = identity::resolve_index_identity(path)?;

   let pid = usock::read_pid(&identity.store_id);
   let started_at = pid.and_then(|_| pid_started_at(&identity.store_id));
   let meta_store = MetaStore::load(&identity.store_id).ok();
   let mut daemon = DaemonJson {
      running: false,
      pid,
      started_at,
      binary_version: None,
      protocol_version: None,
      stale: false,
      supported_schema_versions: None,
   };

   let mut status = None;

   if let Ok(Ok(stream)) =
      time::timeout(CONNECT_TIMEOUT, usock::Stream::connect(&identity.store_id)).await
   {
      let mut stream = stream;
      let mut buffer = ipc::SocketBuffer::new();
      let request = ipc::client_hello(
         &identity.store_id,
         &identity.config_fingerprint,
         Some(ipc::default_client_id("ggrep-status")),
         ipc::default_client_capabilities(),
      );

      let hello = time::timeout(RPC_TIMEOUT, async {
         buffer.send(&mut stream, &request).await?;
         buffer
            .recv_with_limit::<_, Response>(&mut stream, config::get().max_response_bytes)
            .await
      })
      .await;

      match hello {
         Ok(Ok(Response::Hello {
            protocol_version,
            binary_version,
            supported_schema_versions,
            ..
         })) => {
            daemon.running = true;
            daemon.protocol_version = Some(protocol_version);
            daemon.binary_version = Some(binary_version);
            daemon.supported_schema_versions = Some(supported_schema_versions);

            let health = time::timeout(RPC_TIMEOUT, async {
               buffer.send(&mut stream, &Request::Health).await?;
               buffer
                  .recv_with_limit::<_, Response>(&mut stream, config::get().max_response_bytes)
                  .await
            })
            .await;

            if let Ok(Ok(Response::Health { status: s })) = health {
               status = Some(s);
            }
         },
         Ok(Ok(Response::Error { code, .. })) if code == "invalid_request" => {
            daemon.running = true;
            daemon.stale = true;
         },
         _ => {},
      }
   }

   let cfg = config::get();
   let in_flight = status.as_ref().map(|s| s.queries_in_flight).unwrap_or(0);
   let queued = status.as_ref().map(|s| s.queries_queued).unwrap_or(0);
   let busy_total = status.as_ref().map(|s| s.busy_total).unwrap_or(0);
   let timeouts_total = status.as_ref().map(|s| s.timeouts_total).unwrap_or(0);
   let slow_total = status.as_ref().map(|s| s.slow_total).unwrap_or(0);
   let indexing = status.as_ref().map(|s| s.indexing).unwrap_or(false);

   let store_path = config::data_dir().join(&identity.store_id);
   let store_bytes = util::get_dir_size(&store_path).unwrap_or(0);
   let cache_bytes = util::get_dir_size(config::model_dir()).unwrap_or(0)
      + util::get_dir_size(config::grammar_dir()).unwrap_or(0);
   let log_bytes = 0u64;

   let head_sha = git::get_head_sha(&identity.canonical_root);
   let dirty = git::is_dirty(&identity.canonical_root);
   let untracked_included = if head_sha.is_some() || dirty.is_some() {
      Some(true)
   } else {
      None
   };
   let (snapshot_id, snapshot_created_at, last_sync_at, last_result, last_duration_ms, degraded) =
      if let Some(meta) = meta_store.as_ref() {
         (
            meta.snapshot_id().map(|s| s.to_string()),
            meta.snapshot_created_at().map(|s| s.to_string()),
            meta.last_sync_at().map(|s| s.to_string()),
            meta.last_sync_result().map(|s| s.to_string()),
            meta.last_sync_duration_ms(),
            meta.snapshot_degraded(),
         )
      } else {
         (None, None, None, None, None, false)
      };

   let segments_open = status.as_ref().map(|s| s.segments_open).unwrap_or(0);
   let segments_budget = status
      .as_ref()
      .map(|s| s.segments_budget)
      .unwrap_or(cfg.effective_max_open_segments_global() as u64);
   let query_latency_p50_ms = status
      .as_ref()
      .map(|s| s.query_latency_p50_ms);
   let query_latency_p95_ms = status
      .as_ref()
      .map(|s| s.query_latency_p95_ms);
   let segments_touched_max = status
      .as_ref()
      .map(|s| s.segments_touched_max);
   let embed_status = limiter::status().unwrap_or(limiter::EmbedLimiterStatus {
      max_concurrent: 0,
      in_use:         0,
      stale_lock:     false,
   });
   let publish_time_last_ms = meta_store.as_ref().and_then(|m| m.last_sync_duration_ms());
   let gc_time_last_ms = meta_store.as_ref().and_then(|m| m.last_gc_duration_ms());
   let compaction_time_last_ms =
      meta_store.as_ref().and_then(|m| m.last_compaction_duration_ms());

   let json = StatusJson {
      schema_version: 1,
      store_id: identity.store_id,
      canonical_root: identity.canonical_root.to_string_lossy().to_string(),
      config_fingerprint: identity.config_fingerprint,
      ignore_fingerprint: identity.ignore_fingerprint,
      daemon,
      snapshot: SnapshotJson {
         active_snapshot_id: snapshot_id,
         head_sha,
         dirty,
         untracked_included,
         degraded,
         created_at: snapshot_created_at,
      },
      sync: SyncJson {
         state: if indexing {
            "indexing".to_string()
         } else {
            "idle".to_string()
         },
         last_sync_at,
         last_result,
         last_duration_ms,
         staging_txn_id: None,
      },
      queries: QueriesJson {
         max_concurrent: cfg.max_concurrent_queries,
         max_queue_depth: cfg.max_query_queue,
         timeout_ms: cfg.query_timeout_ms,
         in_flight,
         queue_depth: queued,
         busy_total,
         timeouts_total,
         slow_total,
      },
      resources: ResourcesJson {
         embed_global: EmbedGlobalJson {
            max_concurrent: embed_status.max_concurrent as u32,
            in_use:         embed_status.in_use as u32,
            stale_lock:     embed_status.stale_lock,
         },
         disk:         DiskJson {
            store_bytes,
            store_budget_bytes: cfg.max_store_bytes,
            cache_bytes,
            cache_budget_bytes: cfg.max_cache_bytes,
            log_bytes,
            log_budget_bytes: cfg.max_log_bytes,
         },
         open_handles: OpenHandlesJson { segments_open, segments_budget },
      },
      performance: PerformanceJson {
         query_latency_p50_ms,
         query_latency_p95_ms,
         query_latency_budget_p50_ms: cfg.budget_query_p50_ms,
         query_latency_budget_p95_ms: cfg.budget_query_p95_ms,
         segments_touched_max,
         segments_touched_budget: cfg.budget_max_segments_touched,
         publish_time_last_ms,
         publish_time_budget_ms: cfg.budget_publish_ms,
         gc_time_last_ms,
         gc_time_budget_ms: cfg.budget_gc_ms,
         compaction_time_last_ms,
         compaction_time_budget_ms: cfg.budget_compaction_ms,
      },
   };

   if pretty {
      Ok(serde_json::to_string_pretty(&json)?)
   } else {
      Ok(serde_json::to_string(&json)?)
   }
}

fn pid_started_at(store_id: &str) -> Option<String> {
   let path = usock::pid_path(store_id);
   let meta = fs::metadata(path).ok()?;
   let modified = meta.modified().ok()?;
   let dt: DateTime<Utc> = modified.into();
   Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}
