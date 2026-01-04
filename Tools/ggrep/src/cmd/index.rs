//! Index creation and management command.
//!
//! Scans source files, chunks them, computes embeddings, and stores them in the
//! vector database. Supports dry-run mode and index reset operations.

use std::{
   path::{Path, PathBuf},
   sync::Arc,
};

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use walkdir::WalkDir;

use crate::{
   Result,
   chunker::Chunker,
   embed::{Embedder, candle::CandleEmbedder},
   file::LocalFileSystem,
   identity,
   index_lock::IndexLock,
   meta::MetaStore,
   store::LanceStore,
   sync::{SyncEngine, SyncOptions, SyncProgressCallback},
};

/// Executes the index command to create or update a code index.
pub async fn execute(
   path: Option<PathBuf>,
   dry_run: bool,
   reset: bool,
   eval_store: bool,
   allow_degraded: bool,
   store_id: Option<String>,
) -> Result<()> {
   let cwd = std::env::current_dir()?.canonicalize()?;
   let requested = path.unwrap_or(cwd).canonicalize()?;
   let index_identity = identity::resolve_index_identity(&requested)?;
   let index_path = index_identity.canonical_root.clone();

   let resolved_store_id = match store_id {
      Some(s) => {
         if eval_store && !s.ends_with("-eval") {
            format!("{s}-eval")
         } else {
            s
         }
      },
      None => {
         let base = index_identity.store_id;
         if eval_store {
            format!("{base}-eval")
         } else {
            base
         }
      },
   };

   if reset {
      println!("{}", style(format!("Resetting index for store: {resolved_store_id}")).yellow());
      delete_store(&resolved_store_id, &index_path).await?;
      println!("{}", style("Existing index removed. Re-indexing...").dim());
   }

   let spinner = ProgressBar::new_spinner();
   spinner.set_style(
      ProgressStyle::default_spinner()
         .template("{spinner:.green} {msg}")
         .unwrap(),
   );

   if dry_run {
      spinner.set_message("Scanning files (dry run)...");
      let file_count = scan_files(&index_path);
      spinner.finish_with_message(format!("Dry run complete: would index {file_count} files"));
      println!("\nWould index files in: {}", index_path.display());
      println!("Store ID: {resolved_store_id}");
      return Ok(());
   }

   let mut pb = ProgressBar::new(0);
   pb.set_style(
      ProgressStyle::default_bar()
         .template("{spinner:.green} {msg} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%)")
         .unwrap()
         .progress_chars("█▓░"),
   );
   pb.set_message("...");
   pb.set_prefix("Indexing: ");

   let result = index_files(&index_path, &resolved_store_id, &mut |u| {
      pb.progress(u);
      spinner.tick();
      pb.tick();
   }, allow_degraded)
   .await?;

   pb.finish_with_message(format!("Indexing complete: {} files indexed", result.indexed));

   println!("\n{}", style("Index created successfully!").green().bold());
   println!("Store ID: {}", style(&resolved_store_id).cyan());
   println!("Path: {}", style(index_path.display()).dim());
   println!("Files indexed: {}", result.indexed);
   println!("Total chunks: {}", style(result.total_chunks.to_string()).bold());

   Ok(())
}

/// Deletes an existing store and its associated metadata.
async fn delete_store(store_id: &str, _index_path: &Path) -> Result<()> {
   let _lock = IndexLock::acquire(store_id)?;

   let store = LanceStore::new()?;

   store.delete_store(store_id).await?;

   let mut meta_store = MetaStore::load(store_id)?;
   meta_store.clear_all();
   meta_store.save()?;

   Ok(())
}

/// Scans the directory tree and counts indexable source files.
fn scan_files(path: &Path) -> usize {
   let mut count = 0;
   if path.is_dir() {
      for entry in WalkDir::new(path)
         .follow_links(false)
         .into_iter()
         .filter_map(|e| e.ok())
      {
         if entry.file_type().is_file()
            && let Some(ext) = entry.path().extension()
         {
            let ext_str = ext.to_string_lossy();
            if matches!(ext_str.as_ref(), "rs" | "ts" | "js" | "py" | "go" | "tsx" | "jsx") {
               count += 1;
            }
         }
      }
   }
   count
}

/// Result of an indexing operation.
#[derive(Debug)]
struct IndexResult {
   indexed:      usize,
   total_chunks: usize,
}

/// Performs the actual file indexing using the sync engine.
async fn index_files(
   path: &Path,
   store_id: &str,
   callback: &mut dyn SyncProgressCallback,
   allow_degraded: bool,
) -> Result<IndexResult> {
   let file_system = LocalFileSystem::new();
   let embedder: Arc<dyn Embedder> = Arc::new(CandleEmbedder::new()?);
   let store: Arc<LanceStore> = Arc::new(LanceStore::new()?);

   let sync_engine = SyncEngine::new(file_system, Chunker::default(), embedder, store);

   let result = sync_engine
      .initial_sync_with_options(
         store_id,
         path,
         None,
         false,
         SyncOptions { allow_degraded, ..SyncOptions::default() },
         callback,
      )
      .await?;

   Ok(IndexResult { indexed: result.indexed, total_chunks: result.indexed })
}
