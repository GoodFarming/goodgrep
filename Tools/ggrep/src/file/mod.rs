//! File system operations for code discovery, ignore patterns, and watching.

pub mod discovery;
pub mod ignore;
pub mod watcher;

use std::path::Path;

pub use discovery::*;
pub use ignore::*;
pub use watcher::*;

/// Converts a path to a normalized string representation with forward slashes.
pub fn normalize_path(path: &Path) -> String {
   let s = path.to_string_lossy();
   if s.contains('\\') {
      s.replace('\\', "/")
   } else {
      s.into_owned()
   }
}
