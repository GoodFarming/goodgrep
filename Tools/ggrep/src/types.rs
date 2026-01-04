use std::{path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{Str, meta::FileHash};

/// Type of code chunk extracted from source files
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkType {
   Function,
   Class,
   Interface,
   Method,
   TypeAlias,
   Block,
   Other,
}

impl ChunkType {
   pub const fn as_lowercase_str(self) -> &'static str {
      match self {
         Self::Function => "function",
         Self::Class => "class",
         Self::Interface => "interface",
         Self::Method => "method",
         Self::TypeAlias => "typealias",
         Self::Block => "block",
         Self::Other => "other",
      }
   }
}

/// Stack-optimized vector for context information (usually small)
pub type ContextVec = SmallVec<[Str; 4]>;

/// Parsed code chunk with location and context information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
   pub content:     Str,
   pub start_line:  usize,
   pub start_col:   usize,
   pub end_line:    usize,
   pub chunk_type:  Option<ChunkType>,
   pub context:     ContextVec,
   pub chunk_index: Option<i32>,
   pub is_anchor:   Option<bool>,
}

impl Chunk {
   pub fn new(
      content: Str,
      start_line: usize,
      end_line: usize,
      chunk_type: ChunkType,
      context: &[Str],
   ) -> Self {
      Self {
         content,
         start_line,
         start_col: 0,
         end_line,
         chunk_type: Some(chunk_type),
         context: context.iter().cloned().collect(),
         chunk_index: None,
         is_anchor: Some(false),
      }
   }

   pub const fn with_col(mut self, col: usize) -> Self {
      self.start_col = col;
      self
   }
}

/// Chunk prepared for embedding with file hash and identifier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedChunk {
   pub row_id:       String,
   pub chunk_id:     String,
   #[serde(serialize_with = "crate::serde_arc_pathbuf::serialize")]
   #[serde(deserialize_with = "crate::serde_arc_pathbuf::deserialize")]
   pub path_key:     Arc<PathBuf>,
   pub path_key_ci:  String,
   pub ordinal:      u32,
   pub file_hash:    FileHash,
   pub chunk_hash:   FileHash,
   pub chunker:      String,
   pub kind:         String,
   pub text:         Str,
   pub start_line:   u32,
   pub end_line:     u32,
   pub chunk_type:   Option<ChunkType>,
   pub context_prev: Option<Str>,
   pub context_next: Option<Str>,
}

/// Chunk with embedding vectors ready for storage in vector database
#[derive(Debug, Clone)]
pub struct VectorRecord {
   pub row_id:        String,
   pub chunk_id:      String,
   pub path_key:      Arc<PathBuf>,
   pub path_key_ci:   String,
   pub ordinal:       u32,
   pub file_hash:     FileHash,
   pub chunk_hash:    FileHash,
   pub chunker:       String,
   pub kind:          String,
   pub text:          Str,
   pub start_line:    u32,
   pub end_line:      u32,
   pub chunk_type:    Option<ChunkType>,
   pub context_prev:  Option<Str>,
   pub context_next:  Option<Str>,
   pub vector:        Vec<f32>,
   pub colbert:       Vec<u8>,
   pub colbert_scale: f64,
}

/// Individual search result with location and relevance score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
   pub path:            PathBuf,
   pub content:         Str,
   pub score:           f32,
   #[serde(skip)]
   pub secondary_score: Option<f32>,
   #[serde(skip)]
   pub row_id:          Option<String>,
   #[serde(skip)]
   pub segment_table:   Option<String>,
   pub start_line:      u32,
   pub num_lines:       u32,
   pub chunk_type:      Option<ChunkType>,
   pub is_anchor:       Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchLimitHit {
   pub code:     String,
   pub limit:    u64,
   pub observed: Option<u64>,
   #[serde(default)]
   pub path_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchWarning {
   pub code:     String,
   pub message:  String,
   #[serde(default)]
   pub path_key: Option<String>,
}

pub fn sort_results_deterministic(results: &mut [SearchResult]) {
   results.sort_by(cmp_results_deterministic);
}

pub fn cmp_results_deterministic(a: &SearchResult, b: &SearchResult) -> std::cmp::Ordering {
   const SCORE_EPSILON: f32 = 1e-6;

   let score_diff = a.score - b.score;
   if score_diff.abs() > SCORE_EPSILON {
      return b
         .score
         .partial_cmp(&a.score)
         .unwrap_or(std::cmp::Ordering::Equal);
   }

   let secondary_a = a.secondary_score.unwrap_or(0.0);
   let secondary_b = b.secondary_score.unwrap_or(0.0);
   let secondary_diff = secondary_a - secondary_b;
   if secondary_diff.abs() > SCORE_EPSILON {
      return secondary_b
         .partial_cmp(&secondary_a)
         .unwrap_or(std::cmp::Ordering::Equal);
   }

   let path_cmp = a.path.to_string_lossy().cmp(&b.path.to_string_lossy());
   if path_cmp != std::cmp::Ordering::Equal {
      return path_cmp;
   }

   let line_cmp = a.start_line.cmp(&b.start_line);
   if line_cmp != std::cmp::Ordering::Equal {
      return line_cmp;
   }

   let row_cmp = a
      .row_id
      .as_deref()
      .unwrap_or("")
      .cmp(b.row_id.as_deref().unwrap_or(""));
   if row_cmp != std::cmp::Ordering::Equal {
      return row_cmp;
   }

   a.num_lines.cmp(&b.num_lines)
}

pub fn sort_and_dedup_limits(limits: &mut Vec<SearchLimitHit>) {
   limits.sort_by(|a, b| {
      let code_cmp = a.code.cmp(&b.code);
      if code_cmp != std::cmp::Ordering::Equal {
         return code_cmp;
      }
      a.path_key.cmp(&b.path_key)
   });
   limits.dedup_by(|a, b| a.code == b.code && a.path_key == b.path_key);
}

pub fn sort_and_dedup_warnings(warnings: &mut Vec<SearchWarning>) {
   warnings.sort_by(|a, b| {
      let code_cmp = a.code.cmp(&b.code);
      if code_cmp != std::cmp::Ordering::Equal {
         return code_cmp;
      }
      a.path_key.cmp(&b.path_key)
   });
   warnings.dedup_by(|a, b| a.code == b.code && a.path_key == b.path_key);
}

/// Current indexing status of the search system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchStatus {
   Ready,
   Indexing,
}

/// High-level intent mode for search.
///
/// Used to tune candidate mixing and ranking for hybrid corpora (code + docs +
/// diagrams) without requiring changes to how documents are authored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
   /// Default behavior (mostly score-sorted results).
   #[default]
   Balanced,
   /// Favors breadth: plans/docs/diagrams alongside code.
   Discovery,
   /// Favors implementation code while still surfacing relevant docs/diagrams.
   Implementation,
   /// Favors planning/spec documents and diagrams.
   Planning,
   /// Favors debugging/incident triage code paths.
   Debug,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SearchTimings {
   pub admission_ms:     u64,
   pub snapshot_read_ms: u64,
   pub retrieve_ms:      u64,
   pub rank_ms:          u64,
   pub format_ms:        u64,
}

/// Response from a semantic search query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
   pub results:    Vec<SearchResult>,
   pub status:     SearchStatus,
   pub progress:   Option<u8>,
   #[serde(default)]
   pub timings_ms: Option<SearchTimings>,
   #[serde(default)]
   pub limits_hit: Vec<SearchLimitHit>,
   #[serde(default)]
   pub warnings:   Vec<SearchWarning>,
}

/// Metadata about a vector store instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreInfo {
   pub store_id:  String,
   pub row_count: u64,
   pub path:      PathBuf,
}

/// Progress tracking for indexing operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncProgress {
   pub processed:    usize,
   pub indexed:      usize,
   pub total:        usize,
   pub current_file: Option<Str>,
}
