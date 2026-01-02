//! Code search engine combining vector embeddings, `ColBERT` reranking, and
//! result ranking.

pub mod colbert;
pub mod profile;
pub mod ranking;

use std::{cmp::Ordering, path::Path, sync::Arc};

use crate::{
   embed::Embedder,
   error::Result,
   store::{SearchParams, Store},
   types::{SearchMode, SearchResponse},
};

/// High-level search engine orchestrating embeddings, vector search, and
/// reranking.
pub struct SearchEngine {
   store:    Arc<dyn Store>,
   embedder: Arc<dyn Embedder>,
}

impl SearchEngine {
   pub fn new(store: Arc<dyn Store>, embedder: Arc<dyn Embedder>) -> Self {
      Self { store, embedder }
   }

   /// Searches a store for code matching a natural language query.
   ///
   /// Performs vector search, applies structural boosting, and optionally
   /// reranks with `ColBERT`. Results are limited both globally and per-file.
   pub async fn search(
      &self,
      store_id: &str,
      query: &str,
      limit: usize,
      per_file_limit: usize,
      path_filter: Option<&Path>,
      rerank: bool,
      include_anchors: bool,
   ) -> Result<SearchResponse> {
      self
         .search_with_mode(
            store_id,
            query,
            limit,
            per_file_limit,
            path_filter,
            rerank,
            include_anchors,
            SearchMode::Balanced,
         )
         .await
   }

   pub async fn search_with_mode(
      &self,
      store_id: &str,
      query: &str,
      limit: usize,
      per_file_limit: usize,
      path_filter: Option<&Path>,
      rerank: bool,
      include_anchors: bool,
      mode: SearchMode,
   ) -> Result<SearchResponse> {
      let query_enc = self.embedder.encode_query(query).await?;

      let store_limit = match mode {
         SearchMode::Balanced => limit.saturating_mul(2).max(limit),
         _ => limit.saturating_mul(10).max(limit),
      };

      let mut response = self
         .store
         .search(SearchParams {
            store_id,
            query_text: query,
            query_vector: &query_enc.dense,
            query_colbert: &query_enc.colbert,
            limit: store_limit,
            path_filter,
            rerank,
            include_anchors,
         })
         .await?;

      ranking::apply_structural_boost_with_mode(&mut response.results, mode);

      response
         .results
         .sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));

      response.results = profile::select_for_mode(response.results, limit, per_file_limit, mode);

      Ok(response)
   }
}
