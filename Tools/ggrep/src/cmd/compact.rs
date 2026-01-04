//! Compaction command for merging segments and pruning tombstones.

use std::path::PathBuf;
use std::sync::Arc;

use console::style;
use serde::Serialize;

use crate::{Result, identity, snapshot::{CompactionOptions, compact_store}, store::LanceStore};

#[derive(Serialize)]
struct CompactionJson {
   schema_version:   u32,
   store_id:         String,
   base_snapshot_id: Option<String>,
   new_snapshot_id:  Option<String>,
   performed:        bool,
   reason:           Option<String>,
   segments_before:  usize,
   segments_after:   usize,
   tombstones_before: u64,
   tombstones_after:  u64,
   rows_before:      u64,
   rows_after:       u64,
   duration_ms:      u64,
}

pub async fn execute(
   path: Option<PathBuf>,
   force: bool,
   json: bool,
   store_id: Option<String>,
) -> Result<()> {
   let cwd = std::env::current_dir()?.canonicalize()?;
   let requested = path.unwrap_or(cwd).canonicalize()?;
   let identity = identity::resolve_index_identity(&requested)?;
   let root_store_id = store_id.unwrap_or(identity.store_id.clone());

   let store = Arc::new(LanceStore::new()?);
   let result = compact_store(
      store,
      &root_store_id,
      &identity.config_fingerprint,
      &identity.ignore_fingerprint,
      CompactionOptions { force, max_retries: 1 },
   )
   .await?;

   if json {
      let payload = CompactionJson {
         schema_version: 1,
         store_id: root_store_id,
         base_snapshot_id: result.base_snapshot_id,
         new_snapshot_id: result.new_snapshot_id,
         performed: result.performed,
         reason: result.reason,
         segments_before: result.segments_before,
         segments_after: result.segments_after,
         tombstones_before: result.tombstones_before,
         tombstones_after: result.tombstones_after,
         rows_before: result.rows_before,
         rows_after: result.rows_after,
         duration_ms: result.duration_ms,
      };
      println!("{}", serde_json::to_string_pretty(&payload)?);
      return Ok(());
   }

   if !result.performed {
      let reason = result
         .reason
         .unwrap_or_else(|| "compaction skipped".to_string());
      println!("{}", style(reason).yellow());
      return Ok(());
   }

   println!("{}", style("✓ Compaction complete").green());
   println!(
      "  segments: {} -> {}",
      style(result.segments_before).dim(),
      style(result.segments_after).dim()
   );
   println!(
      "  tombstones: {} -> {}",
      style(result.tombstones_before).dim(),
      style(result.tombstones_after).dim()
   );
   println!(
      "  rows: {} -> {}",
      style(result.rows_before).dim(),
      style(result.rows_after).dim()
   );
   if let Some(new_snapshot) = result.new_snapshot_id {
      println!("  snapshot: {}", style(new_snapshot).dim());
   }

   Ok(())
}
