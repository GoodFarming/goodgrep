//! Code search engine combining vector embeddings, `ColBERT` reranking, and
//! result ranking.

pub mod colbert;
pub mod profile;
pub mod ranking;

use std::{path::Path, sync::Arc};

use crate::{
   config,
   embed::{Embedder, limiter},
   error::Result,
   snapshot::SnapshotView,
   store::{LanceStore, SearchParams},
   types::{
      SearchLimitHit, SearchMode, SearchResponse, SearchTimings, sort_and_dedup_limits,
      sort_and_dedup_warnings, sort_results_deterministic,
   },
};

/// High-level search engine orchestrating embeddings, vector search, and
/// reranking.
pub struct SearchEngine {
   store:    Arc<LanceStore>,
   embedder: Arc<dyn Embedder>,
}

impl SearchEngine {
   pub fn new(store: Arc<LanceStore>, embedder: Arc<dyn Embedder>) -> Self {
      Self { store, embedder }
   }

   /// Searches a store for code matching a natural language query.
   ///
   /// Performs vector search, applies structural boosting, and optionally
   /// reranks with `ColBERT`. Results are limited both globally and per-file.
   pub async fn search(
      &self,
      snapshot: &SnapshotView,
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
            snapshot,
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
      snapshot: &SnapshotView,
      store_id: &str,
      query: &str,
      limit: usize,
      per_file_limit: usize,
      path_filter: Option<&Path>,
      rerank: bool,
      include_anchors: bool,
      mode: SearchMode,
   ) -> Result<SearchResponse> {
      let embed_start = std::time::Instant::now();
      let _permit = limiter::acquire().await?;
      let query_enc = self.embedder.encode_query(query).await?;
      let embed_ms = embed_start.elapsed().as_millis() as u64;

      let store_limit = match mode {
         SearchMode::Balanced => limit.saturating_mul(2).max(limit),
         _ => limit.saturating_mul(10).max(limit),
      };

      let retrieve_start = std::time::Instant::now();
      let mut response = self
         .store
         .search_segments(SearchParams {
            store_id,
            tables: snapshot.segment_tables(),
            query_text: query,
            query_vector: &query_enc.dense,
            query_colbert: &query_enc.colbert,
            limit: store_limit,
            path_filter,
            rerank,
            include_anchors,
         })
         .await?;
      let retrieve_ms = retrieve_start.elapsed().as_millis() as u64 + embed_ms;

      let cfg = config::get();
      let mut limits_hit = std::mem::take(&mut response.limits_hit);
      let mut warnings = std::mem::take(&mut response.warnings);

      let max_candidates = cfg.effective_max_candidates();
      if response.results.len() > max_candidates {
         let observed = response.results.len() as u64;
         response.results.truncate(max_candidates);
         limits_hit.push(SearchLimitHit {
            code:     "max_candidates".to_string(),
            limit:    max_candidates as u64,
            observed: Some(observed),
            path_key: None,
         });
      }

      let rank_start = std::time::Instant::now();
      ranking::apply_structural_boost_with_mode(&mut response.results, mode);

      sort_results_deterministic(&mut response.results);

      response.results.retain(|r| {
         let key = r.path.to_string_lossy();
         snapshot.is_visible(key.as_ref(), r.segment_table.as_deref())
      });

      response.results = profile::select_for_mode(response.results, limit, per_file_limit, mode);
      let rank_ms = rank_start.elapsed().as_millis() as u64;

      apply_snippet_caps(
         &mut response.results,
         cfg.effective_max_total_snippet_bytes(),
         cfg.effective_max_snippet_bytes_per_result(),
         &mut limits_hit,
      );
      sort_and_dedup_limits(&mut limits_hit);
      sort_and_dedup_warnings(&mut warnings);

      response.timings_ms = Some(SearchTimings {
         admission_ms: 0,
         snapshot_read_ms: 0,
         retrieve_ms,
         rank_ms,
         format_ms: 0,
      });
      response.limits_hit = limits_hit;
      response.warnings = warnings;

      Ok(response)
   }
}

fn apply_snippet_caps(
   results: &mut [crate::types::SearchResult],
   max_total_bytes: usize,
   max_bytes_per_result: usize,
   limits_hit: &mut Vec<SearchLimitHit>,
) {
   let mut total_bytes: usize = 0;
   for result in results.iter_mut() {
      let original_len = result.content.len();
      if original_len > max_bytes_per_result {
         let (truncated, changed) = truncate_str_bytes(&result.content, max_bytes_per_result);
         if changed {
            let path_key = result.path.to_string_lossy().to_string();
            limits_hit.push(SearchLimitHit {
               code:     "max_snippet_bytes_per_result".to_string(),
               limit:    max_bytes_per_result as u64,
               observed: Some(original_len as u64),
               path_key: Some(path_key),
            });
            result.content = truncated;
         }
      }
      total_bytes = total_bytes.saturating_add(result.content.len());
   }

   if total_bytes <= max_total_bytes {
      return;
   }

   let observed_total = total_bytes as u64;
   let mut remaining = max_total_bytes;
   for result in results.iter_mut() {
      if remaining == 0 {
         if !result.content.is_empty() {
            result.content = crate::Str::from_string(String::new());
         }
         continue;
      }
      let len = result.content.len();
      if len <= remaining {
         remaining = remaining.saturating_sub(len);
         continue;
      }
      let (truncated, _changed) = truncate_str_bytes(&result.content, remaining);
      result.content = truncated;
      remaining = 0;
   }

   limits_hit.push(SearchLimitHit {
      code:     "max_total_snippet_bytes".to_string(),
      limit:    max_total_bytes as u64,
      observed: Some(observed_total),
      path_key: None,
   });
}

fn truncate_str_bytes(input: &crate::Str, max_bytes: usize) -> (crate::Str, bool) {
   if max_bytes == 0 {
      return (crate::Str::from_string(String::new()), !input.is_empty());
   }
   let s = input.as_str();
   if s.len() <= max_bytes {
      return (input.clone(), false);
   }
   let mut idx = max_bytes.min(s.len());
   while idx > 0 && !s.is_char_boundary(idx) {
      idx -= 1;
   }
   let truncated = crate::Str::copy_from_str(&s[..idx]);
   (truncated, true)
}
