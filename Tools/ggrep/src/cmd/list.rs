//! List all vector stores command.
//!
//! Displays information about all existing stores including their size and
//! modification time.

use std::{fs, time::SystemTime};

use console::style;
use serde::Serialize;

use crate::{
   Result, config,
   util::{format_size, get_dir_size},
};

#[derive(Serialize)]
struct StoresJson {
   schema_version: u32,
   stores:         Vec<StoreInfoJson>,
}

#[derive(Serialize)]
struct StoreInfoJson {
   store_id:   String,
   size_bytes: u64,
   modified_at: String,
}

/// Executes the list command to display all available stores.
pub fn execute(json: bool) -> Result<()> {
   let data_dir = config::data_dir();

   if !data_dir.exists() {
      if json {
         let payload = StoresJson { schema_version: 1, stores: Vec::new() };
         println!("{}", serde_json::to_string_pretty(&payload)?);
         return Ok(());
      }
      println!("No stores found.");
      println!(
         "\nRun {} in a repository to create your first store.",
         style("ggrep index").green()
      );
      return Ok(());
   }

   let mut stores = Vec::new();

   for entry in fs::read_dir(data_dir)? {
      let entry = entry?;
      let path = entry.path();

      if path.is_dir()
         && let Some(name) = path.file_name().and_then(|n| n.to_str())
      {
         let metadata = fs::metadata(&path)?;
         let modified = metadata.modified()?;
         let size = get_dir_size(&path)?;

         stores.push(StoreInfo { name: name.to_string(), size, modified });
      }
   }

   if stores.is_empty() {
      if json {
         let payload = StoresJson { schema_version: 1, stores: Vec::new() };
         println!("{}", serde_json::to_string_pretty(&payload)?);
         return Ok(());
      }
      println!("No stores found.");
      println!(
         "\nRun {} in a repository to create your first store.",
         style("ggrep index").green()
      );
      return Ok(());
   }

   stores.sort_by(|a, b| b.modified.cmp(&a.modified));

   if json {
      let payload = StoresJson {
         schema_version: 1,
         stores: stores
            .iter()
            .map(|store| StoreInfoJson {
               store_id: store.name.clone(),
               size_bytes: store.size,
               modified_at: format_time_rfc3339(store.modified),
            })
            .collect(),
      };
      println!("{}", serde_json::to_string_pretty(&payload)?);
      return Ok(());
   }

   println!(
      "\n{} {}",
      style(format!("Found {} store(s):", stores.len())).bold(),
      style(format!("(in {})", data_dir.display())).dim()
   );
   println!();

   for store in stores {
      println!("  {}", style(&store.name).green().bold());
      println!(
         "    Size: {} • Modified: {}",
         style(format_size(store.size)).dim(),
         style(format_time_ago(store.modified)).dim()
      );
      println!();
   }

   println!(
      "{}",
      style(format!("To clean up a store: rm -rf {}/<store-name>", data_dir.display())).dim()
   );
   println!("{}", style("To use a specific store: ggrep --store <store-name> <query>").dim());

   Ok(())
}

/// Information about a store on disk.
struct StoreInfo {
   name:     String,
   size:     u64,
   modified: SystemTime,
}

/// Formats a `SystemTime` as a human-readable "time ago" string.
fn format_time_ago(time: SystemTime) -> String {
   let now = SystemTime::now();
   let duration = now.duration_since(time).unwrap_or_default();

   let seconds = duration.as_secs();
   let minutes = seconds / 60;
   let hours = minutes / 60;
   let days = hours / 24;

   if days > 0 {
      format!("{days}d ago")
   } else if hours > 0 {
      format!("{hours}h ago")
   } else if minutes > 0 {
      format!("{minutes}m ago")
   } else {
      "just now".to_string()
   }
}

fn format_time_rfc3339(time: SystemTime) -> String {
   let dt: chrono::DateTime<chrono::Utc> = time.into();
   dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
