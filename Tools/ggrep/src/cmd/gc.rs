//! Garbage collection command for stores and snapshots.

use std::{
   fs,
   path::PathBuf,
   time::{Duration, SystemTime},
};

use console::style;
use serde::Serialize;
use tokio::time;

use crate::{
   Result, config,
   cmd::daemon::{HandshakeOutcome, client_handshake},
   error::Error,
   identity,
   ipc::{self, Request, Response},
   snapshot::{GcOptions, gc_snapshots},
   store::LanceStore,
   usock,
   util::{format_size, get_dir_size},
};

#[derive(Serialize, Clone)]
struct GcStoreInfo {
   store_id:    String,
   size_bytes:  u64,
   modified_at: String,
   has_meta:    bool,
}

#[derive(Serialize)]
struct GcJson {
   schema_version: u32,
   dry_run:        bool,
   candidates:     Vec<GcStoreInfo>,
   deleted:        Vec<GcStoreInfo>,
}

#[derive(Serialize)]
struct SnapshotGcJson {
   schema_version:      u32,
   store_id:            String,
   active_snapshot_id:  Option<String>,
   dry_run:             bool,
   retained_snapshots:  Vec<String>,
   deleted_snapshots:   Vec<String>,
   deleted_segments:    Vec<String>,
   deleted_tombstones:  Vec<String>,
   duration_ms:         u64,
}

pub async fn execute(
   stores: bool,
   force: bool,
   json: bool,
   path: Option<PathBuf>,
   store_id: Option<String>,
) -> Result<()> {
   if stores {
      return gc_stores(force, json);
   }

   gc_snapshots_command(force, json, path, store_id).await
}

fn gc_stores(force: bool, json: bool) -> Result<()> {
   let data_dir = config::data_dir();
   let meta_dir = config::meta_dir();
   let mut candidates = Vec::new();

   if data_dir.exists() {
      for entry in fs::read_dir(data_dir)? {
         let entry = entry?;
         let path = entry.path();
         if !path.is_dir() {
            continue;
         }
         let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
         };
         let meta_path = meta_dir.join(format!("{name}.json"));
         let metadata = fs::metadata(&path)?;
         let modified = metadata.modified()?;
         let size = get_dir_size(&path)?;
         let info = GcStoreInfo {
            store_id: name.to_string(),
            size_bytes: size,
            modified_at: format_time_rfc3339(modified),
            has_meta: meta_path.exists(),
         };
         if !info.has_meta {
            candidates.push(info);
         }
      }
   }

   let mut deleted = Vec::new();
   if force {
      for candidate in &candidates {
         let path = data_dir.join(&candidate.store_id);
         if path.exists() {
            fs::remove_dir_all(&path)?;
            deleted.push(candidate.clone());
         }
      }
   }

   if json {
      let payload = GcJson {
         schema_version: 1,
         dry_run: !force,
         candidates,
         deleted,
      };
      println!("{}", serde_json::to_string_pretty(&payload)?);
      return Ok(());
   }

   if candidates.is_empty() {
      println!("{}", style("No orphan stores found.").green());
      return Ok(());
   }

   if !force {
      println!(
         "{}",
         style(format!(
            "{} orphan store(s) found. Re-run with --force to delete:",
            candidates.len()
         ))
         .yellow()
      );
      for candidate in &candidates {
         println!(
            "  {} ({}; modified {})",
            style(&candidate.store_id).bold(),
            style(format_size(candidate.size_bytes)).dim(),
            style(&candidate.modified_at).dim()
         );
      }
      return Ok(());
   }

   println!(
      "{}",
      style(format!("Deleted {} orphan store(s).", deleted.len())).green()
   );
   Ok(())
}

async fn gc_snapshots_command(
   force: bool,
   json: bool,
   path: Option<PathBuf>,
   store_id: Option<String>,
) -> Result<()> {
   let cwd = std::env::current_dir()?.canonicalize()?;
   let requested = path.unwrap_or(cwd).canonicalize()?;
   let identity = identity::resolve_index_identity(&requested)?;
   let root_store_id = store_id.unwrap_or(identity.store_id.clone());

   let report = if let Ok(Ok(mut stream)) = time::timeout(
      Duration::from_millis(500),
      usock::Stream::connect(&root_store_id),
   )
   .await
   {
      let outcome = client_handshake(
         &mut stream,
         &root_store_id,
         &identity.config_fingerprint,
         "ggrep-gc",
      )
      .await?;
      match outcome {
         HandshakeOutcome::Compatible => {
            let mut buffer = ipc::SocketBuffer::new();
            buffer
               .send(&mut stream, &Request::Gc { dry_run: !force })
               .await?;
            match buffer
               .recv_with_limit::<_, Response>(&mut stream, config::get().max_response_bytes)
               .await?
            {
               Response::Gc { report } => report,
               Response::Error { code, message } => {
                  return Err(
                     Error::Server {
                        op:     "gc",
                        reason: format!("{code}: {message}"),
                     }
                     .into(),
                  );
               },
               _ => {
                  return Err(Error::UnexpectedResponse("gc").into());
               },
            }
         },
         _ => {
            return Err(
               Error::Server {
                  op:     "gc",
                  reason: "daemon handshake failed".to_string(),
               }
               .into(),
            );
         },
      }
   } else {
      let store = std::sync::Arc::new(LanceStore::new()?);
      gc_snapshots(
         store,
         &root_store_id,
         &identity.config_fingerprint,
         &identity.ignore_fingerprint,
         GcOptions { dry_run: !force, ..GcOptions::default() },
      )
      .await?
   };

   if json {
      let payload = SnapshotGcJson {
         schema_version: 1,
         store_id: root_store_id,
         active_snapshot_id: report.active_snapshot_id,
         dry_run: report.dry_run,
         retained_snapshots: report.retained_snapshots,
         deleted_snapshots: report.deleted_snapshots,
         deleted_segments: report.deleted_segments,
         deleted_tombstones: report.deleted_tombstones,
         duration_ms: report.duration_ms,
      };
      println!("{}", serde_json::to_string_pretty(&payload)?);
      return Ok(());
   }

   if report.deleted_snapshots.is_empty() && report.deleted_segments.is_empty() {
      println!("{}", style("No GC candidates found.").green());
      return Ok(());
   }

   if report.dry_run {
      println!(
         "{}",
         style("GC dry-run: candidates found. Re-run with --force to delete.").yellow()
      );
   } else {
      println!("{}", style("GC complete.").green());
   }

   if !report.deleted_snapshots.is_empty() {
      println!("{}", style("Snapshots:").bold());
      for snap in &report.deleted_snapshots {
         println!("  {}", style(snap).dim());
      }
   }

   if !report.deleted_segments.is_empty() {
      println!("{}", style("Segments:").bold());
      for seg in &report.deleted_segments {
         println!("  {}", style(seg).dim());
      }
   }

   Ok(())
}

fn format_time_rfc3339(time: SystemTime) -> String {
   let dt: chrono::DateTime<chrono::Utc> = time.into();
   dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
