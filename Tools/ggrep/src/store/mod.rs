//! Vector storage abstraction with `LanceDB` implementation.

pub mod lance;

use std::{
   collections::HashMap,
   path::{Path, PathBuf},
   sync::Arc,
};

use ndarray::Array2;

use crate::{
   error::Result,
   meta::FileHash,
   types::{SearchResponse, StoreInfo, VectorRecord},
};

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
   pub query_text:      &'a str,
   pub query_vector:    &'a [f32],
   pub query_colbert:   &'a Array2<f32>,
   pub limit:           usize,
   pub path_filter:     Option<&'a Path>,
   pub rerank:          bool,
   pub include_anchors: bool,
}

/// Storage backend for vector embeddings, supporting search, indexing, and file
/// management.
#[async_trait::async_trait]
pub trait Store: Send + Sync {
   /// Inserts a batch of vector records into the store.
   async fn insert_batch(&self, store_id: &str, records: Vec<VectorRecord>) -> Result<()>;

   /// Searches the store using dense vectors, `ColBERT` embeddings, and
   /// full-text search.
   async fn search(&self, params: SearchParams<'_>) -> Result<SearchResponse>;

   /// Deletes all records associated with a single file.
   async fn delete_file(&self, store_id: &str, file_path: &Path) -> Result<()>;

   /// Deletes all records associated with multiple files.
   async fn delete_files(&self, store_id: &str, file_paths: &[PathBuf]) -> Result<()>;

   /// Deletes an entire store.
   async fn delete_store(&self, store_id: &str) -> Result<()>;

   /// Retrieves metadata about a store.
   async fn get_info(&self, store_id: &str) -> Result<StoreInfo>;

   /// Lists all files currently indexed in the store.
   async fn list_files(&self, store_id: &str) -> Result<Vec<PathBuf>>;

   /// Checks whether a store contains any records.
   async fn is_empty(&self, store_id: &str) -> Result<bool>;

   /// Creates a full-text search index on the content column.
   async fn create_fts_index(&self, store_id: &str) -> Result<()>;

   /// Creates an IVF-PQ vector index for approximate nearest neighbor search.
   async fn create_vector_index(&self, store_id: &str) -> Result<()>;

   /// Retrieves file hashes for all indexed files.
   async fn get_file_hashes(&self, store_id: &str) -> Result<HashMap<PathBuf, FileHash>>;
}

#[async_trait::async_trait]
impl<T: Store + ?Sized> Store for Arc<T> {
   async fn insert_batch(&self, store_id: &str, records: Vec<VectorRecord>) -> Result<()> {
      (**self).insert_batch(store_id, records).await
   }

   async fn search(&self, params: SearchParams<'_>) -> Result<SearchResponse> {
      (**self).search(params).await
   }

   async fn delete_file(&self, store_id: &str, file_path: &Path) -> Result<()> {
      (**self).delete_file(store_id, file_path).await
   }

   async fn delete_files(&self, store_id: &str, file_paths: &[PathBuf]) -> Result<()> {
      (**self).delete_files(store_id, file_paths).await
   }

   async fn delete_store(&self, store_id: &str) -> Result<()> {
      (**self).delete_store(store_id).await
   }

   async fn get_info(&self, store_id: &str) -> Result<StoreInfo> {
      (**self).get_info(store_id).await
   }

   async fn list_files(&self, store_id: &str) -> Result<Vec<PathBuf>> {
      (**self).list_files(store_id).await
   }

   async fn is_empty(&self, store_id: &str) -> Result<bool> {
      (**self).is_empty(store_id).await
   }

   async fn create_fts_index(&self, store_id: &str) -> Result<()> {
      (**self).create_fts_index(store_id).await
   }

   async fn create_vector_index(&self, store_id: &str) -> Result<()> {
      (**self).create_vector_index(store_id).await
   }

   async fn get_file_hashes(&self, store_id: &str) -> Result<HashMap<PathBuf, FileHash>> {
      (**self).get_file_hashes(store_id).await
   }
}

pub use lance::LanceStore;

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
