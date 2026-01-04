//! Snapshot segment file index helpers.

use std::{
   collections::HashMap,
   fs::{self, File},
   io::{BufRead, BufReader, Write},
   path::Path,
};

use serde::{Deserialize, Serialize};

use crate::{Result, error::Error};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentFileIndexEntry {
   pub path_key:   String,
   pub segment_id: String,
}

pub fn read_segment_file_index(path: &Path) -> Result<HashMap<String, String>> {
   let file = File::open(path)?;
   let reader = BufReader::new(file);
   let mut map = HashMap::new();
   for (idx, line) in reader.lines().enumerate() {
      let line = line?;
      if line.trim().is_empty() {
         continue;
      }
      let entry: SegmentFileIndexEntry = serde_json::from_str(&line).map_err(|e| {
         Error::Server {
            op:     "segment_index",
            reason: format!("invalid segment index entry at line {}: {e}", idx + 1),
         }
      })?;
      map.insert(entry.path_key, entry.segment_id);
   }
   Ok(map)
}

pub fn write_segment_file_index(path: &Path, mapping: &HashMap<String, String>) -> Result<()> {
   if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)?;
   }
   let mut keys: Vec<&String> = mapping.keys().collect();
   keys.sort();

   let mut file = File::create(path)?;
   for key in keys {
      if let Some(segment_id) = mapping.get(key) {
         let entry = SegmentFileIndexEntry {
            path_key:   key.clone(),
            segment_id: segment_id.clone(),
         };
         let line = serde_json::to_string(&entry)?;
         writeln!(file, "{line}")?;
      }
   }
   file.sync_all()?;
   Ok(())
}
