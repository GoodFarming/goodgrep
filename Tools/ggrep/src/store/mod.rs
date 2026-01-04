//! Vector storage abstraction with `LanceDB` implementation.

pub(crate) mod lance;

use std::path::Path;

use ndarray::Array2;



/// Converts a path to the exact string stored in the table.
pub fn path_to_store_value(path: &Path) -> String {
   match path.to_str() {
      Some(s) => s.to_owned(),
      None => hex::encode(path.as_os_str().as_encoded_bytes()),
   }
}

/// Escapes a file path for use in SQL `=` / `IN` predicates.
pub fn escape_path_literal(path: &Path) -> String {
   match path.to_str() {
      Some(s) => s.replace('\'', "''"),
      None => hex::encode(path.as_os_str().as_encoded_bytes()),
   }
}

/// Escapes a file path for use in SQL LIKE predicates.
///
/// Escapes backslashes, percent signs, underscores, and single quotes.
pub fn escape_path_for_like(path: &Path) -> String {
   path_to_store_value(path)
      .replace('\\', "\\\\")
      .replace('%', "\\%")
      .replace('_', "\\_")
      .replace('\'', "''")
}

/// Parameters for vector search queries.
pub struct SearchParams<'a> {
   pub store_id:        &'a str,
   pub tables:          &'a [String],
   pub query_text:      &'a str,
   pub query_vector:    &'a [f32],
   pub query_colbert:   &'a Array2<f32>,
   pub limit:           usize,
   pub path_filter:     Option<&'a Path>,
   pub rerank:          bool,
   pub include_anchors: bool,
}

pub use lance::LanceStore;

#[derive(Debug, Clone)]
pub struct SegmentMetadata {
   pub rows:       u64,
   pub size_bytes: u64,
   pub sha256:     String,
}

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn path_to_store_value_preserves_characters() {
      let path = Path::new("foo_bar%baz'qux");
      assert_eq!(path_to_store_value(path), "foo_bar%baz'qux");
   }

   #[test]
   fn escape_path_literal_escapes_single_quotes() {
      let path = Path::new("foo_bar%baz'qux");
      assert_eq!(escape_path_literal(path), "foo_bar%baz''qux");
   }

   #[test]
   fn escape_path_for_like_escapes_specials() {
      let path = Path::new("foo_bar%baz'qux");
      assert_eq!(escape_path_for_like(path), "foo\\_bar\\%baz''qux");
   }
}
