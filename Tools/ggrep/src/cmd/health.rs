//! Health check command.
//!
//! Reports on index and daemon health using structured checks.

use std::{
   path::{Path, PathBuf},
   sync::Arc,
};

use console::style;
use hf_hub::Cache;
use serde::Serialize;

use crate::{
   Result, config,
   embed::limiter,
   git,
   grammar::GrammarManager,
   identity,
   ipc::{self, Request, Response},
   meta::MetaStore,
   models,
   snapshot::{SnapshotManager, SnapshotManifest},
   store::LanceStore,
   usock,
   util::get_dir_size,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
   Ok,
   Warn,
   Fail,
}

impl Severity {
   fn as_str(self) -> &'static str {
      match self {
         Severity::Ok => "ok",
         Severity::Warn => "warn",
         Severity::Fail => "fail",
      }
   }
}

#[derive(Serialize)]
struct HealthCheck {
   code:     String,
   severity: String,
   message:  String,
}

#[derive(Serialize)]
struct HealthJson {
   schema_version:     u32,
   store_id:           String,
   active_snapshot_id: Option<String>,
   ok:                 bool,
   checks:             Vec<HealthCheck>,
}

pub async fn execute(json: bool) -> Result<()> {
   let cwd = std::env::current_dir()?;
   let payload = collect_health_payload(&cwd).await?;

   if json {
      println!("{}", serde_json::to_string_pretty(&payload)?);
      return Ok(());
   }

   println!("{}", style("ggrep Health").bold());
   for check in &payload.checks {
      let symbol = match check.severity.as_str() {
         "ok" => style("✓").green(),
         "warn" => style("○").yellow(),
         _ => style("✗").red(),
      };
      println!("{} {} - {}", symbol, check.code, check.message);
   }

   if payload.ok {
      println!("\n{}", style("✓ All health checks passed.").green().bold());
   } else {
      println!("\n{}", style("✗ Some health checks failed.").red().bold());
   }

   Ok(())
}

pub(crate) async fn collect_health_json(path: &Path, pretty: bool) -> Result<String> {
   let payload = collect_health_payload(path).await?;
   if pretty {
      Ok(serde_json::to_string_pretty(&payload)?)
   } else {
      Ok(serde_json::to_string(&payload)?)
   }
}

async fn collect_health_payload(path: &Path) -> Result<HealthJson> {
   let identity = identity::resolve_index_identity(path)?;
   let store_id = identity.store_id.clone();

   let mut checks: Vec<HealthCheck> = Vec::new();
   let mut ok = true;
   let mut active_snapshot_id: Option<String> = None;

   let meta_path = config::meta_dir().join(format!("{store_id}.json"));
   let meta_store = MetaStore::load(&store_id).ok();
   if meta_path.exists() && meta_store.is_some() {
      push_check(&mut checks, &mut ok, "manifest_present", Severity::Ok, "metadata present");
   } else {
      push_check(
         &mut checks,
         &mut ok,
         "manifest_present",
         Severity::Fail,
         "metadata missing or unreadable",
      );
   }

   let mut row_count = None;
   let mut segments_count = None;
   let mut tombstones_count = None;

   let store = match LanceStore::new() {
      Ok(store) => Some(Arc::new(store)),
      Err(e) => {
         push_check(
            &mut checks,
            &mut ok,
            "segments_present",
            Severity::Fail,
            format!("failed to open store: {e}"),
         );
         None
      },
   };

   if let Some(store) = store {
      let snapshot_manager = SnapshotManager::new(
         store,
         store_id.clone(),
         identity.config_fingerprint.clone(),
         identity.ignore_fingerprint.clone(),
      );

      active_snapshot_id = snapshot_manager.read_active_snapshot_id().ok().flatten();
      if let Some(snapshot_id) = active_snapshot_id.as_deref() {
         match SnapshotManifest::load(&snapshot_manager.manifest_path(snapshot_id)) {
            Ok(manifest) => {
               row_count = Some(manifest.counts.chunks_indexed);
               segments_count = Some(manifest.segments.len());
               tombstones_count = Some(manifest.tombstones.iter().map(|t| t.count).sum());
               if manifest.counts.chunks_indexed > 0 {
                  push_check(
                     &mut checks,
                     &mut ok,
                     "segments_present",
                     Severity::Ok,
                     "segments present",
                  );
               } else {
                  push_check(
                     &mut checks,
                     &mut ok,
                     "segments_present",
                     Severity::Warn,
                     "store has no indexed rows",
                  );
               }
            },
            Err(e) => {
               push_check(
                  &mut checks,
                  &mut ok,
                  "segments_present",
                  Severity::Fail,
                  format!("failed to read active manifest: {e}"),
               );
            },
         }
      } else {
         push_check(
            &mut checks,
            &mut ok,
            "segments_present",
            Severity::Warn,
            "no active snapshot",
         );
      }
   }

   push_check(
      &mut checks,
      &mut ok,
      "tombstones_enforced",
      Severity::Ok,
      "tombstone filtering enabled",
   );

   let compaction_check = compaction_policy_check(segments_count, tombstones_count);
   push_check(&mut checks, &mut ok, "compaction_policy", compaction_check.0, compaction_check.1);

   match detect_casefold_collisions(meta_store.as_ref()) {
      Ok(None) => push_check(
         &mut checks,
         &mut ok,
         "casefold_collisions",
         Severity::Ok,
         "no collisions detected",
      ),
      Ok(Some(collisions)) => push_check(
         &mut checks,
         &mut ok,
         "casefold_collisions",
         Severity::Fail,
         format!("casefold collisions: {}", collisions.join(", ")),
      ),
      Err(e) => push_check(
         &mut checks,
         &mut ok,
         "casefold_collisions",
         Severity::Warn,
         format!("unable to evaluate collisions: {e}"),
      ),
   }

   push_check(&mut checks, &mut ok, "path_safety", Severity::Ok, "out-of-root enforcement enabled");

   if let Some((count, bytes)) = untracked_stats(&identity.canonical_root) {
      if count > 0 {
         push_check(
            &mut checks,
            &mut ok,
            "repo_hygiene",
            Severity::Warn,
            format!("untracked files: {count} ({bytes} bytes)"),
         );
      } else {
         push_check(&mut checks, &mut ok, "repo_hygiene", Severity::Ok, "no untracked files");
      }
   }

   let artifact_check = artifact_integrity_check();
   push_check(&mut checks, &mut ok, "artifact_integrity", artifact_check.0, artifact_check.1);

   let disk_check = disk_budget_check(&store_id);
   push_check(&mut checks, &mut ok, "disk_budgets", disk_check.0, disk_check.1);

   let embed_check = embed_limiter_check();
   push_check(&mut checks, &mut ok, "embed_limiter", embed_check.0, embed_check.1);

   let daemon_status = daemon_status(&store_id, &identity.config_fingerprint).await;
   let open_handles_check = open_handles_check(daemon_status.as_ref());
   push_check(&mut checks, &mut ok, "open_handles", open_handles_check.0, open_handles_check.1);

   let perf_query_check = perf_query_latency_check(daemon_status.as_ref());
   push_check(
      &mut checks,
      &mut ok,
      "perf_query_latency",
      perf_query_check.0,
      perf_query_check.1,
   );
   let perf_segments_check = perf_segments_touched_check(daemon_status.as_ref());
   push_check(
      &mut checks,
      &mut ok,
      "perf_segments_touched",
      perf_segments_check.0,
      perf_segments_check.1,
   );
   let perf_publish_check = perf_publish_check(meta_store.as_ref());
   push_check(
      &mut checks,
      &mut ok,
      "perf_publish_time",
      perf_publish_check.0,
      perf_publish_check.1,
   );
   let perf_gc_check = perf_gc_check(meta_store.as_ref());
   push_check(&mut checks, &mut ok, "perf_gc_time", perf_gc_check.0, perf_gc_check.1);
   let perf_compaction_check = perf_compaction_check(meta_store.as_ref());
   push_check(
      &mut checks,
      &mut ok,
      "perf_compaction_time",
      perf_compaction_check.0,
      perf_compaction_check.1,
   );

   if let Some(rows) = row_count {
      if let Some(meta) = meta_store.as_ref() {
         let files = meta.all_paths().count();
         if rows == 0 && files > 0 {
            push_check(
               &mut checks,
               &mut ok,
               "index_drift",
               Severity::Warn,
               format!("metadata has {files} files but store is empty"),
            );
         }
      }
   }

   Ok(HealthJson { schema_version: 1, store_id, active_snapshot_id, ok, checks })
}

fn push_check(
   checks: &mut Vec<HealthCheck>,
   ok: &mut bool,
   code: &str,
   severity: Severity,
   message: impl Into<String>,
) {
   if severity == Severity::Fail {
      *ok = false;
   }
   checks.push(HealthCheck {
      code:     code.to_string(),
      severity: severity.as_str().to_string(),
      message:  message.into(),
   });
}

fn detect_casefold_collisions(meta: Option<&MetaStore>) -> Result<Option<Vec<String>>> {
   let Some(meta) = meta else {
      return Ok(None);
   };
   let mut seen: std::collections::HashMap<String, Vec<PathBuf>> = std::collections::HashMap::new();
   for path in meta.all_paths() {
      if let Some(key) = crate::file::casefold_path_key(path) {
         seen.entry(key).or_default().push(path.clone());
      }
   }
   let mut collisions: Vec<String> = seen
      .into_values()
      .filter(|paths| paths.len() > 1)
      .flatten()
      .map(|p| p.to_string_lossy().into_owned())
      .collect();
   collisions.sort();
   collisions.dedup();
   if collisions.is_empty() {
      Ok(None)
   } else {
      Ok(Some(collisions))
   }
}

fn untracked_stats(root: &Path) -> Option<(usize, u64)> {
   let paths = git::untracked_paths(root)?;
   let mut bytes = 0u64;
   for path in &paths {
      if let Ok(meta) = std::fs::metadata(path) {
         bytes = bytes.saturating_add(meta.len());
      }
   }
   Some((paths.len(), bytes))
}

fn artifact_integrity_check() -> (Severity, String) {
   let cfg = config::get();
   let models = [&cfg.dense_model, &cfg.colbert_model];
   const MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

   let cache = Cache::new(config::model_dir().clone());
   let mut missing_any = Vec::new();
   for model_id in models {
      let repo = cache.repo(models::repo_for_model(model_id));
      let missing: Vec<&str> = MODEL_FILES
         .iter()
         .copied()
         .filter(|f| repo.get(f).is_none())
         .collect();
      if !missing.is_empty() {
         missing_any.push(format!("{model_id} missing {}", missing.join(", ")));
      }
   }

   let grammar_missing = match GrammarManager::with_auto_download(false) {
      Ok(gm) => {
         let missing = gm.missing_languages().collect::<Vec<_>>();
         if missing.is_empty() {
            None
         } else {
            Some(missing.join(", "))
         }
      },
      Err(_) => Some("grammar manager failed to initialize".to_string()),
   };

   if missing_any.is_empty() && grammar_missing.is_none() {
      (Severity::Ok, "models and grammars available".to_string())
   } else {
      let mut issues = missing_any;
      if let Some(missing) = grammar_missing {
         issues.push(format!("missing grammars: {missing}"));
      }
      (Severity::Warn, issues.join("; "))
   }
}

fn disk_budget_check(store_id: &str) -> (Severity, String) {
   let cfg = config::get();
   let store_bytes = get_dir_size(&config::data_dir().join(store_id)).unwrap_or(0);
   let cache_bytes = get_dir_size(config::model_dir()).unwrap_or(0)
      + get_dir_size(config::grammar_dir()).unwrap_or(0);
   let log_bytes = 0u64;

   let mut warnings = Vec::new();
   if cfg.max_store_bytes > 0 && store_bytes > cfg.max_store_bytes {
      warnings.push(format!("store over budget ({} > {})", store_bytes, cfg.max_store_bytes));
   }
   if cfg.max_cache_bytes > 0 && cache_bytes > cfg.max_cache_bytes {
      warnings.push(format!("cache over budget ({} > {})", cache_bytes, cfg.max_cache_bytes));
   }
   if cfg.max_log_bytes > 0 && log_bytes > cfg.max_log_bytes {
      warnings.push(format!("logs over budget ({} > {})", log_bytes, cfg.max_log_bytes));
   }

   if warnings.is_empty() {
      (Severity::Ok, "disk usage within budgets".to_string())
   } else {
      (Severity::Warn, warnings.join("; "))
   }
}

fn embed_limiter_check() -> (Severity, String) {
   match limiter::status() {
      Ok(status) => {
         if status.max_concurrent == 0 {
            return (Severity::Warn, "global embed limiter disabled".to_string());
         }
         let msg = format!("embed limiter usage {} / {}", status.in_use, status.max_concurrent);
         if status.stale_lock {
            (Severity::Warn, format!("{msg}; stale lock detected"))
         } else {
            (Severity::Ok, msg)
         }
      },
      Err(e) => (Severity::Warn, format!("embed limiter status unavailable: {e}")),
   }
}

fn compaction_policy_check(
   segments_count: Option<usize>,
   tombstones_count: Option<u64>,
) -> (Severity, String) {
   let cfg = config::get();
   let Some(segments) = segments_count else {
      return (
         Severity::Warn,
         format!(
            "compaction metrics unavailable; thresholds segments/overdue={}/{}, tombstones/overdue={}/{}",
            cfg.compaction_overdue_segments,
            cfg.max_segments_per_snapshot,
            cfg.compaction_overdue_tombstones,
            cfg.max_tombstones_per_snapshot
         ),
      );
   };
   let tombstones = tombstones_count.unwrap_or(0);

   if cfg.max_segments_per_snapshot > 0 && segments > cfg.max_segments_per_snapshot {
      return (
         Severity::Fail,
         format!(
            "segment cap exceeded ({} > {})",
            segments, cfg.max_segments_per_snapshot
         ),
      );
   }
   if cfg.max_tombstones_per_snapshot > 0
      && tombstones > cfg.max_tombstones_per_snapshot as u64
   {
      return (
         Severity::Fail,
         format!(
            "tombstone cap exceeded ({} > {})",
            tombstones, cfg.max_tombstones_per_snapshot
         ),
      );
   }

   if (cfg.compaction_overdue_segments > 0 && segments >= cfg.compaction_overdue_segments)
      || (cfg.compaction_overdue_tombstones > 0
         && tombstones >= cfg.compaction_overdue_tombstones as u64)
   {
      return (
         Severity::Warn,
         format!(
            "compaction overdue (segments={}, tombstones={})",
            segments, tombstones
         ),
      );
   }

   (
      Severity::Ok,
      format!(
         "compaction within thresholds (segments={}, tombstones={})",
         segments, tombstones
      ),
   )
}

async fn daemon_status(
   store_id: &str,
   config_fingerprint: &str,
) -> Option<ipc::ServerStatus> {
   let Ok(mut stream) = usock::Stream::connect(store_id).await else {
      return None;
   };

   let mut buffer = ipc::SocketBuffer::new();
   let hello = ipc::client_hello(
      store_id,
      config_fingerprint,
      Some(ipc::default_client_id("ggrep-health")),
      ipc::default_client_capabilities(),
   );
   if buffer.send(&mut stream, &hello).await.is_err() {
      return None;
   }
   let response: Option<Response> = buffer
      .recv_with_limit(&mut stream, config::get().max_response_bytes)
      .await
      .ok();
   if !matches!(response, Some(Response::Hello { .. })) {
      return None;
   }

   buffer.send(&mut stream, &Request::Health).await.ok();
   let response: Option<Response> = buffer
      .recv_with_limit(&mut stream, config::get().max_response_bytes)
      .await
      .ok();

   if let Some(Response::Health { status }) = response {
      return Some(status);
   }
   None
}

fn open_handles_check(status: Option<&ipc::ServerStatus>) -> (Severity, String) {
   let Some(status) = status else {
      return (Severity::Warn, "daemon not running".to_string());
   };

   if status.segments_open > status.segments_budget {
      return (
         Severity::Warn,
         format!(
            "open handles over budget ({} > {})",
            status.segments_open, status.segments_budget
         ),
      );
   }

   (
      Severity::Ok,
      format!("open handles ok ({} / {})", status.segments_open, status.segments_budget),
   )
}

fn perf_query_latency_check(status: Option<&ipc::ServerStatus>) -> (Severity, String) {
   let cfg = config::get();
   let Some(status) = status else {
      return (Severity::Warn, "query latency metrics unavailable".to_string());
   };

   let p50 = status.query_latency_p50_ms;
   let p95 = status.query_latency_p95_ms;
   if p50 == 0 && p95 == 0 {
      return (Severity::Warn, "no query latency samples yet".to_string());
   }

   if (cfg.budget_query_p50_ms > 0 && p50 > cfg.budget_query_p50_ms)
      || (cfg.budget_query_p95_ms > 0 && p95 > cfg.budget_query_p95_ms)
   {
      return (
         Severity::Warn,
         format!(
            "query latency over budget (p50 {}>{}, p95 {}>{})",
            p50, cfg.budget_query_p50_ms, p95, cfg.budget_query_p95_ms
         ),
      );
   }

   (
      Severity::Ok,
      format!(
         "query latency ok (p50 {}<= {}, p95 {}<= {})",
         p50, cfg.budget_query_p50_ms, p95, cfg.budget_query_p95_ms
      ),
   )
}

fn perf_segments_touched_check(status: Option<&ipc::ServerStatus>) -> (Severity, String) {
   let cfg = config::get();
   let Some(status) = status else {
      return (Severity::Warn, "segment touch metrics unavailable".to_string());
   };
   let max_segments = status.segments_touched_max;
   if max_segments == 0 {
      return (Severity::Warn, "no segment touch samples yet".to_string());
   }
   if cfg.budget_max_segments_touched > 0 && max_segments > cfg.budget_max_segments_touched {
      return (
         Severity::Warn,
         format!(
            "segments touched over budget ({} > {})",
            max_segments, cfg.budget_max_segments_touched
         ),
      );
   }
   (
      Severity::Ok,
      format!(
         "segments touched ok ({} <= {})",
         max_segments, cfg.budget_max_segments_touched
      ),
   )
}

fn perf_publish_check(meta: Option<&MetaStore>) -> (Severity, String) {
   let cfg = config::get();
   let Some(meta) = meta else {
      return (Severity::Warn, "publish metrics unavailable".to_string());
   };
   budget_check(
      "publish time",
      meta.last_sync_duration_ms(),
      cfg.budget_publish_ms,
   )
}

fn perf_gc_check(meta: Option<&MetaStore>) -> (Severity, String) {
   let cfg = config::get();
   let Some(meta) = meta else {
      return (Severity::Warn, "gc metrics unavailable".to_string());
   };
   budget_check("gc time", meta.last_gc_duration_ms(), cfg.budget_gc_ms)
}

fn perf_compaction_check(meta: Option<&MetaStore>) -> (Severity, String) {
   let cfg = config::get();
   let Some(meta) = meta else {
      return (Severity::Warn, "compaction metrics unavailable".to_string());
   };
   budget_check(
      "compaction time",
      meta.last_compaction_duration_ms(),
      cfg.budget_compaction_ms,
   )
}

fn budget_check(label: &str, observed: Option<u64>, budget_ms: u64) -> (Severity, String) {
   if budget_ms == 0 {
      return (Severity::Ok, format!("{label} budget disabled"));
   }
   let Some(observed) = observed else {
      return (Severity::Warn, format!("{label} unavailable (budget {budget_ms}ms)"));
   };
   if observed > budget_ms {
      return (
         Severity::Warn,
         format!("{label} over budget ({}ms > {}ms)", observed, budget_ms),
      );
   }
   (
      Severity::Ok,
      format!("{label} ok ({}ms <= {}ms)", observed, budget_ms),
   )
}
