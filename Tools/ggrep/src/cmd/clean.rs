//! Store cleanup command.
//!
//! Removes both lance data and metadata for a store, ensuring a clean slate for
//! re-indexing.

use console::style;

use crate::{Result, config, identity, reader_lock::ReaderLock};

pub fn execute(store_id: Option<String>, all: bool) -> Result<()> {
   if all {
      return clean_all();
   }

   let resolved_store_id = if let Some(id) = store_id {
      id
   } else {
      let cwd = std::env::current_dir()?;
      identity::resolve_index_identity(&cwd)?.store_id
   };

   clean_store(&resolved_store_id)?;

   println!("{}", style(format!("Cleaned store: {resolved_store_id}")).green());
   Ok(())
}

fn clean_store(store_id: &str) -> Result<()> {
   let _lock = ReaderLock::acquire_exclusive(store_id)?;
   // Delete metadata file
   let meta_path = config::meta_dir().join(format!("{store_id}.json"));
   if meta_path.exists() {
      std::fs::remove_file(&meta_path)?;
   }

   // Delete entire lance database directory (not just drop_table which leaves
   // fragments)
   let data_path = config::data_dir().join(store_id);
   if data_path.exists() {
      std::fs::remove_dir_all(&data_path)?;
   }

   Ok(())
}

fn clean_all() -> Result<()> {
   let meta_dir = config::meta_dir();
   let data_dir = config::data_dir();

   let mut cleaned = 0;

   // Clean stores found in meta directory
   if meta_dir.exists() {
      for entry in std::fs::read_dir(meta_dir)? {
         let entry = entry?;
         let path = entry.path();
         if path.extension().is_some_and(|e| e == "json")
            && let Some(stem) = path.file_stem()
         {
            let store_id = stem.to_string_lossy();
            println!("{}", style(format!("Cleaning: {store_id}")).dim());
            clean_store(&store_id)?;
            cleaned += 1;
         }
      }
   }

   // Also clean any orphaned data directories (no meta file)
   if data_dir.exists() {
      for entry in std::fs::read_dir(data_dir)? {
         let entry = entry?;
         let path = entry.path();
         if path.is_dir()
            && let Some(name) = path.file_name()
         {
            let store_id = name.to_string_lossy();
            let meta_path = meta_dir.join(format!("{store_id}.json"));
            if !meta_path.exists() {
               println!("{}", style(format!("Cleaning orphaned: {store_id}")).dim());
               let _ = std::fs::remove_dir_all(&path);
               cleaned += 1;
            }
         }
      }
   }

   if cleaned == 0 {
      println!("{}", style("No stores to clean").yellow());
   } else {
      println!("{}", style(format!("Cleaned {cleaned} store(s)")).green());
   }

   Ok(())
}
