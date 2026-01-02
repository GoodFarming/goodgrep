//! Promote an `-eval` store to the canonical store id.
//!
//! This is a convenience wrapper around `clone-store` so operators don't need
//! to manually type store ids.

use std::path::PathBuf;

use crate::{Result, cmd::clone_store, git};

pub fn execute(path: Option<PathBuf>, overwrite: bool, store_id: Option<String>) -> Result<()> {
   let root = std::env::current_dir()?;
   let store_path = path.unwrap_or(root);

   let resolved = store_id.map_or_else(|| git::resolve_store_id(&store_path), Ok)?;

   if let Some(base) = resolved.strip_suffix("-eval") {
      return clone_store::execute(resolved.clone(), base.to_string(), overwrite);
   }

   clone_store::execute(format!("{resolved}-eval"), resolved, overwrite)
}
