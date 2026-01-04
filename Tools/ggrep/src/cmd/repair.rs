//! Repair command for rebuilding missing segments.

use std::{
   path::PathBuf,
   sync::Arc,
};

use console::style;

use crate::{
   Result,
   chunker::Chunker,
   embed::{Embedder, candle::CandleEmbedder},
   error::Error,
   file::{LocalFileSystem, resolve_candidate},
   identity,
   snapshot::{SnapshotManager, read_segment_file_index},
   store::LanceStore,
   sync::{ChangeSet, SyncEngine},
};

/// Executes the repair command.
pub async fn execute(path: Option<PathBuf>, store_id: Option<String>) -> Result<()> {
   let cwd = std::env::current_dir()?.canonicalize()?;
   let requested = path.unwrap_or(cwd).canonicalize()?;
   let identity = identity::resolve_index_identity(&requested)?;
   let root = identity.canonical_root.clone();

   let resolved_store_id = store_id.unwrap_or(identity.store_id.clone());
   let store = Arc::new(LanceStore::new()?);

   let snapshot_manager = SnapshotManager::new(
      store.clone(),
      resolved_store_id.clone(),
      identity.config_fingerprint.clone(),
      identity.ignore_fingerprint.clone(),
   );

   let snapshot_view = snapshot_manager.open_snapshot_view().await?;
   let snapshot_id = snapshot_view.snapshot_id.clone();
   let manifest = snapshot_view.manifest.clone();

   let mapping_path = snapshot_manager
      .snapshot_dir(&snapshot_id)
      .join("segment_file_index.jsonl");
   if !mapping_path.exists() {
      return Err(
         Error::Server {
            op:     "repair",
            reason: "reindex required: segment index missing".to_string(),
         }
         .into(),
      );
   }

   let mapping = read_segment_file_index(&mapping_path)?;
   let mut missing_segments = std::collections::HashSet::new();
   for segment in &manifest.segments {
      if store
         .segment_metadata(&resolved_store_id, &segment.table)
         .await
         .is_err()
      {
         missing_segments.insert(segment.table.clone());
      }
   }

   if missing_segments.is_empty() {
      println!("{}", style("No missing segments detected.").green());
      return Ok(());
   }

   let mut missing_paths = Vec::new();
   for (path_key, segment_id) in mapping {
      if missing_segments.contains(&segment_id) {
         missing_paths.push(path_key);
      }
   }

   if missing_paths.is_empty() {
      return Err(
         Error::Server {
            op:     "repair",
            reason: "segment mapping missing for damaged segments; reindex required".to_string(),
         }
         .into(),
      );
   }

   println!(
      "{}",
      style(format!(
         "Repairing {} file(s) across {} missing segment(s)...",
         missing_paths.len(),
         missing_segments.len()
      ))
      .yellow()
   );

   let mut changeset = ChangeSet::default();
   for path_key in missing_paths {
      let candidate = root.join(&path_key);
      match resolve_candidate(&root, &candidate)? {
         Some(resolved) => changeset.modify.push(resolved),
         None => changeset.delete.push(PathBuf::from(path_key)),
      }
   }

   let embedder: Arc<dyn Embedder> = Arc::new(CandleEmbedder::new()?);
   let sync_engine =
      SyncEngine::new(LocalFileSystem::new(), Chunker::default(), embedder, store);

   let result = sync_engine
      .initial_sync(&resolved_store_id, &root, Some(changeset), false, &mut ())
      .await?;

   println!(
      "{}",
      style(format!(
         "Repair complete (indexed={}, skipped={}, deleted={})",
         result.indexed, result.skipped, result.deleted
      ))
      .green()
   );

   Ok(())
}
