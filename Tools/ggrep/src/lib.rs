//! Semantic code search tool with vector embeddings and `ColBERT` reranking.
//!
//! ggrep indexes source code repositories and enables semantic search through
//! natural language queries, finding conceptually similar code even when exact
//! keywords don't match.

#![feature(portable_simd)]

pub mod assert;
pub mod chunker;
pub mod cmd;
pub mod config;
pub mod embed;
pub mod error;
pub mod file;
pub mod format;
pub mod git;
pub mod grammar;
pub mod identity;
pub mod index_lock;
pub mod ipc;
pub mod lease;
pub mod meta;
pub mod models;
pub mod preprocess;
pub mod reader_lock;
pub mod search;
pub mod serde_arc_pathbuf;
pub mod snapshot;
mod sstr;
pub mod store;
pub mod sync;
pub mod types;
pub mod usock;
pub mod util;
pub mod version;

pub use error::{Error, Result};
pub use sstr::Str;
pub use types::*;
