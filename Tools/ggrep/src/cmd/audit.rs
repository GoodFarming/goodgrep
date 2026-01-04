//! Audit command for snapshot integrity checks.

use std::path::PathBuf;

use console::style;
use serde::Serialize;

use crate::{
   Result,
   error::Error,
   identity,
   snapshot::SnapshotManager,
   store::LanceStore,
};

#[derive(Serialize)]
struct AuditError {
   code:    String,
   message: String,
}

#[derive(Serialize)]
struct AuditJson {
   schema_version: u32,
   store_id:       String,
   snapshot_id:    Option<String>,
   ok:             bool,
   errors:         Vec<AuditError>,
}

/// Executes the audit command.
pub async fn execute(path: Option<PathBuf>, json: bool, store_id: Option<String>) -> Result<()> {
   let cwd = std::env::current_dir()?.canonicalize()?;
   let requested = path.unwrap_or(cwd).canonicalize()?;
   let identity = identity::resolve_index_identity(&requested)?;
   let root_store_id = store_id.unwrap_or(identity.store_id.clone());

   let store = std::sync::Arc::new(LanceStore::new()?);
   let snapshot_manager = SnapshotManager::new(
      store,
      root_store_id.clone(),
      identity.config_fingerprint.clone(),
      identity.ignore_fingerprint.clone(),
   );

   let snapshot_view = snapshot_manager.open_snapshot_view().await?;
   let snapshot_id = snapshot_view.snapshot_id.clone();
   let manifest = snapshot_view.manifest;

   let mut errors = Vec::new();
   let segment_rows: u64 = manifest.segments.iter().map(|s| s.rows).sum();
   if segment_rows != manifest.counts.chunks_indexed {
      errors.push(AuditError {
         code:    "counts_mismatch".to_string(),
         message: format!(
            "manifest counts mismatch (segments={}, manifest={})",
            segment_rows, manifest.counts.chunks_indexed
         ),
      });
   }

   let ok = errors.is_empty();

   if json {
      let payload = AuditJson {
         schema_version: 1,
         store_id: root_store_id,
         snapshot_id: Some(snapshot_id),
         ok,
         errors,
      };
      println!("{}", serde_json::to_string_pretty(&payload)?);
      return Ok(());
   }

   if ok {
      println!("{}", style("✓ Audit OK: manifest counts consistent").green());
      return Ok(());
   }

   println!("{}", style("✗ Audit failed").red().bold());
   for err in errors {
      println!("  - {}", err.message);
   }
   println!(
      "{}",
      style("Recommendation: run `ggrep repair` or reindex if repair fails").yellow()
   );

   Err(
      Error::Server {
         op:     "audit",
         reason: "audit failed".to_string(),
      }
      .into(),
   )
}
