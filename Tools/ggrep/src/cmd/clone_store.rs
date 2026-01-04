//! Store cloning command.
//!
//! Copies LanceDB data + metadata from one store id to another. Intended for
//! promoting an `-eval` store to a canonical store id without re-indexing.

use std::{
   fs, io,
   path::{Path, PathBuf},
};

use console::style;
use walkdir::WalkDir;

use crate::{Result, config, index_lock::IndexLock};

pub fn execute(from: String, to: String, overwrite: bool) -> Result<()> {
   if from == to {
      return Err(io::Error::other("--from and --to must be different").into());
   }

   let (first, second) = if from <= to {
      (from.as_str(), to.as_str())
   } else {
      (to.as_str(), from.as_str())
   };

   let _lock_a = IndexLock::acquire(first)?;
   let _lock_b = IndexLock::acquire(second)?;

   let data_dir = config::data_dir();
   let meta_dir = config::meta_dir();

   let src_data = data_dir.join(&from);
   if !src_data.exists() {
      return Err(
         io::Error::other(format!("source store data dir not found: {}", src_data.display()))
            .into(),
      );
   }

   let src_meta = meta_dir.join(format!("{from}.json"));

   let dst_data = data_dir.join(&to);
   let dst_meta = meta_dir.join(format!("{to}.json"));

   if (dst_data.exists() || dst_meta.exists()) && !overwrite {
      return Err(
         io::Error::other(format!(
            "destination store already exists; pass --overwrite to replace: {to}"
         ))
         .into(),
      );
   }

   if overwrite {
      if dst_data.exists() {
         fs::remove_dir_all(&dst_data)?;
      }
      if dst_meta.exists() {
         fs::remove_file(&dst_meta)?;
      }
   }

   fs::create_dir_all(&dst_data)?;
   copy_dir_recursive(&src_data, &dst_data)?;

   // LanceDB stores tables under <table_name>.lance within the DB directory.
   // ggrep uses a single table per store (table name == store id), so we must
   // rename the table directory to match the destination store id.
   let copied_src_table = dst_data.join(format!("{from}.lance"));
   let dst_table = dst_data.join(format!("{to}.lance"));
   if copied_src_table.exists() && !dst_table.exists() {
      fs::rename(&copied_src_table, &dst_table)?;
   }

   if !dst_table.exists() {
      return Err(
         io::Error::other(format!(
            "expected a Lance table directory at {} (copied from {})",
            dst_table.display(),
            copied_src_table.display()
         ))
         .into(),
      );
   }

   if src_meta.exists() {
      fs::create_dir_all(meta_dir)?;
      fs::copy(&src_meta, &dst_meta)?;
   } else {
      println!(
         "{} {}",
         style("warning:").yellow().bold(),
         style(format!("source meta not found: {}", src_meta.display())).dim()
      );
   }

   println!(
      "{} {} {} {}",
      style("Cloned store:").green().bold(),
      style(&from).cyan(),
      style("→").dim(),
      style(&to).cyan()
   );

   Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
   for entry in WalkDir::new(src) {
      let entry = entry.map_err(|e| io::Error::other(e))?;
      let src_path = entry.path();
      let rel = src_path
         .strip_prefix(src)
         .map_err(|e| io::Error::other(e))?;
      let dst_path: PathBuf = dst.join(rel);

      if entry.file_type().is_dir() {
         fs::create_dir_all(&dst_path)?;
         continue;
      }

      if entry.file_type().is_file() {
         if let Some(parent) = dst_path.parent() {
            fs::create_dir_all(parent)?;
         }
         if fs::hard_link(src_path, &dst_path).is_err() {
            fs::copy(src_path, &dst_path)?;
         }
      }
   }

   Ok(())
}
