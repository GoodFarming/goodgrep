//! Store upgrade command placeholder.
//!
//! Phase II requires an explicit upgrade entrypoint even if the only
//! supported action is to reindex from scratch.

use console::style;

use crate::{Result, identity};

pub fn execute(path: Option<std::path::PathBuf>, store_id: Option<String>) -> Result<()> {
   let resolved_store_id = if let Some(id) = store_id {
      id
   } else {
      let root = path.unwrap_or(std::env::current_dir()?);
      identity::resolve_index_identity(&root)?.store_id
   };

   println!(
      "{}",
      style(format!("Store upgrade not supported yet; reindex required for {resolved_store_id}"))
         .yellow()
   );

   Ok(())
}
