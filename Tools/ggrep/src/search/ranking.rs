//! Result ranking utilities for boosting code structure and limiting per-file
//! results.

use std::path::Path;

use crate::types::{ChunkType, SearchMode, SearchResult};

#[derive(Debug, Clone, Copy)]
pub struct RankingWeights {
   pub function_boost:   f32,
   pub test_penalty:     f32,
   pub doc_multiplier:   f32,
   pub graph_multiplier: f32,
}

impl RankingWeights {
   pub const fn balanced() -> Self {
      Self {
         function_boost:   1.25,
         test_penalty:     0.85,
         doc_multiplier:   0.5,
         graph_multiplier: 1.0,
      }
   }

   pub const fn for_mode(mode: SearchMode) -> Self {
      match mode {
         SearchMode::Balanced => Self::balanced(),
         SearchMode::Discovery => Self {
            function_boost:   1.15,
            test_penalty:     0.9,
            doc_multiplier:   1.0,
            graph_multiplier: 1.05,
         },
         SearchMode::Implementation => Self {
            function_boost:   1.25,
            test_penalty:     0.85,
            doc_multiplier:   0.65,
            graph_multiplier: 0.9,
         },
         SearchMode::Planning => Self {
            function_boost:   1.1,
            test_penalty:     0.9,
            doc_multiplier:   1.15,
            graph_multiplier: 1.1,
         },
         SearchMode::Debug => Self {
            function_boost:   1.2,
            test_penalty:     0.95,
            doc_multiplier:   0.85,
            graph_multiplier: 0.95,
         },
      }
   }
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
   haystack
      .as_bytes()
      .windows(needle.len())
      .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Applies score multipliers based on chunk type and file category.
///
/// Boosts functions, classes, interfaces, methods, and type aliases by 1.25x.
/// Penalizes test files (0.85x) and documentation/config files (0.5x).
pub fn apply_structural_boost(results: &mut [SearchResult]) {
   apply_structural_boost_with_weights(results, RankingWeights::balanced());
}

pub fn apply_structural_boost_with_mode(results: &mut [SearchResult], mode: SearchMode) {
   apply_structural_boost_with_weights(results, RankingWeights::for_mode(mode));
}

pub fn apply_structural_boost_with_weights(results: &mut [SearchResult], weights: RankingWeights) {
   for result in results.iter_mut() {
      if let Some(
         ChunkType::Function
         | ChunkType::Class
         | ChunkType::Interface
         | ChunkType::Method
         | ChunkType::TypeAlias,
      ) = result.chunk_type
      {
         result.score *= weights.function_boost;
      }

      if is_test_file(&result.path) {
         result.score *= weights.test_penalty;
      }

      if is_graph_file(&result.path) {
         result.score *= weights.graph_multiplier;
      } else if is_doc_or_config(&result.path) {
         result.score *= weights.doc_multiplier;
      }
   }
}

/// Deduplicates results by (path, `start_line`), keeping the highest-scoring
/// duplicate.
pub fn deduplicate(mut results: Vec<SearchResult>) -> Vec<SearchResult> {
   if results.is_empty() {
      return results;
   }

   // Sort by (path, start_line, score desc) so highest score comes first for each
   // group
   results.sort_by(|a, b| {
      a.path
         .cmp(&b.path)
         .then_with(|| a.start_line.cmp(&b.start_line))
         .then_with(|| {
            b.score
               .partial_cmp(&a.score)
               .unwrap_or(std::cmp::Ordering::Equal)
         })
   });

   // Deduplicate by keeping first of each (path, line) group (which has highest
   // score)
   let mut deduplicated: Vec<SearchResult> = Vec::with_capacity(results.len());

   for result in results {
      let dominated_by_last = deduplicated
         .last()
         .is_some_and(|last| last.path == result.path && last.start_line == result.start_line);
      if dominated_by_last {
         continue;
      }
      deduplicated.push(result);
   }

   deduplicated
}

/// Limits results to at most `limit` entries per file, preserving highest
/// scores.
pub fn apply_per_file_limit(mut results: Vec<SearchResult>, limit: usize) -> Vec<SearchResult> {
   results.sort_by(|a, b| {
      a.path.cmp(&b.path).then_with(|| {
         b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
      })
   });

   let mut final_results: Vec<SearchResult> = Vec::with_capacity(results.len());
   let mut count = 0;

   for (i, result) in results.into_iter().enumerate() {
      let is_new_path = i == 0 || final_results.last().unwrap().path != result.path;

      if is_new_path {
         count = 0;
      }

      if count < limit {
         count += 1;
         final_results.push(result);
      }
   }

   final_results.sort_by(|a, b| {
      b.score
         .partial_cmp(&a.score)
         .unwrap_or(std::cmp::Ordering::Equal)
   });

   final_results
}

fn is_test_file(path: &Path) -> bool {
   let Some(path_str) = path.to_str() else {
      return false;
   };
   contains_ci(path_str, ".test.")
      || contains_ci(path_str, ".spec.")
      || contains_ci(path_str, "__tests__")
}

fn is_doc_or_config(path: &Path) -> bool {
   if path.extension().is_some_and(|ext| {
      ext.eq_ignore_ascii_case("md")
         || ext.eq_ignore_ascii_case("mdx")
         || ext.eq_ignore_ascii_case("txt")
         || ext.eq_ignore_ascii_case("json")
         || ext.eq_ignore_ascii_case("html")
         || ext.eq_ignore_ascii_case("htm")
         || ext.eq_ignore_ascii_case("css")
         || ext.eq_ignore_ascii_case("yaml")
         || ext.eq_ignore_ascii_case("yml")
         || ext.eq_ignore_ascii_case("toml")
         || ext.eq_ignore_ascii_case("lock")
   }) {
      return true;
   }

   let Some(path_str) = path.to_str() else {
      return false;
   };
   contains_ci(path_str, "/docs/")
}

fn is_graph_file(path: &Path) -> bool {
   path
      .extension()
      .is_some_and(|ext| ext.eq_ignore_ascii_case("mmd") || ext.eq_ignore_ascii_case("mermaid"))
}

#[cfg(test)]
mod tests {
   use std::path::PathBuf;

   use super::*;
   use crate::Str;

   fn make_result(path: &str, start_line: u32, score: f32, chunk_type: ChunkType) -> SearchResult {
      SearchResult {
         path: PathBuf::from(path),
         content: Str::default(),
         score,
         secondary_score: None,
         row_id: None,
         segment_table: None,
         start_line,
         num_lines: 10,
         chunk_type: Some(chunk_type),
         is_anchor: Some(false),
      }
   }

   #[test]
   fn test_apply_structural_boost() {
      let mut results = vec![
         make_result("src/main.rs", 1, 1.0, ChunkType::Function),
         make_result("src/lib.rs", 1, 1.0, ChunkType::Block),
         make_result("src/test.rs", 1, 1.0, ChunkType::Function),
         make_result("README.md", 1, 1.0, ChunkType::Other),
      ];

      apply_structural_boost(&mut results);

      assert!((results[0].score - 1.25).abs() < 1e-6);
      assert!((results[1].score - 1.0).abs() < 1e-6);
      assert!((results[2].score - 1.25).abs() < 1e-6);
      assert!((results[3].score - 0.5).abs() < 1e-6);
   }

   #[test]
   fn test_deduplicate() {
      let results = vec![
         make_result("src/main.rs", 10, 1.0, ChunkType::Function),
         make_result("src/main.rs", 10, 2.0, ChunkType::Function),
         make_result("src/lib.rs", 20, 1.5, ChunkType::Class),
      ];

      let deduped = deduplicate(results);
      assert_eq!(deduped.len(), 2);
      // Find the main.rs result and verify the higher score (2.0) was kept
      let main_result = deduped
         .iter()
         .find(|r| r.path == Path::new("src/main.rs"))
         .unwrap();
      assert!((main_result.score - 2.0).abs() < 1e-6);
   }

   #[test]
   fn test_apply_per_file_limit() {
      let results = vec![
         make_result("file1.rs", 1, 5.0, ChunkType::Function),
         make_result("file1.rs", 2, 4.0, ChunkType::Function),
         make_result("file1.rs", 3, 3.0, ChunkType::Function),
         make_result("file2.rs", 1, 2.0, ChunkType::Function),
      ];

      let limited = apply_per_file_limit(results, 2);
      assert_eq!(limited.len(), 3);

      let file1_count = limited
         .iter()
         .filter(|r| r.path == Path::new("file1.rs"))
         .count();
      assert_eq!(file1_count, 2);
   }

   #[test]
   fn test_is_test_file() {
      assert!(is_test_file(Path::new("src/main.test.ts")));
      assert!(is_test_file(Path::new("src/component.spec.js")));
      assert!(is_test_file(Path::new("src/__tests__/utils.js")));
      assert!(!is_test_file(Path::new("src/main.rs")));
   }

   #[test]
   fn test_is_doc_or_config() {
      assert!(is_doc_or_config(Path::new("README.md")));
      assert!(is_doc_or_config(Path::new("package.json")));
      assert!(is_doc_or_config(Path::new("config.yaml")));
      assert!(is_doc_or_config(Path::new("docs/guide.md")));
      assert!(!is_doc_or_config(Path::new("src/main.rs")));
   }
}
