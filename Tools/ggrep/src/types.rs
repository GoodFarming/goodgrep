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
   pub id:           String,
   #[serde(serialize_with = "crate::serde_arc_pathbuf::serialize")]
   #[serde(deserialize_with = "crate::serde_arc_pathbuf::deserialize")]
   pub path:         Arc<PathBuf>,
   pub hash:         FileHash,
   pub content:      Str,
   pub start_line:   u32,
   pub end_line:     u32,
   pub chunk_index:  Option<u32>,
   pub is_anchor:    Option<bool>,
   pub chunk_type:   Option<ChunkType>,
   pub context_prev: Option<Str>,
   pub context_next: Option<Str>,
}

/// Chunk with embedding vectors ready for storage in vector database
#[derive(Debug, Clone)]
pub struct VectorRecord {
   pub id:            String,
   pub path:          Arc<PathBuf>,
   pub hash:          FileHash,
   pub content:       Str,
   pub start_line:    u32,
   pub end_line:      u32,
   pub chunk_index:   Option<u32>,
   pub is_anchor:     Option<bool>,
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
   pub path:       PathBuf,
   pub content:    Str,
   pub score:      f32,
   pub start_line: u32,
   pub num_lines:  u32,
   pub chunk_type: Option<ChunkType>,
   pub is_anchor:  Option<bool>,
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

/// Response from a semantic search query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
   pub results:  Vec<SearchResult>,
   pub status:   SearchStatus,
   pub progress: Option<u8>,
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
