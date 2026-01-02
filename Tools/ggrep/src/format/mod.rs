//! Output formatting for search results.
//!
//! Provides different formatters for displaying search results:
//! - [`HumanFormatter`](text::HumanFormatter): Rich TTY output with syntax
//!   highlighting and colors
//! - [`AgentFormatter`](text::AgentFormatter): Compact pipe-friendly output for
//!   LLMs
//! - [`CompactFormatter`](text::CompactFormatter): File paths only (like `grep
//!   -l`)
//! - [`JsonFormatter`](json::JsonFormatter): Machine-readable JSON output

pub mod json;
pub mod text;

use std::{borrow::Cow, path::Path};

use crate::types::SearchResult;

/// Output format mode for search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
   /// Rich TTY output with syntax highlighting and colors
   Human,
   /// Compact pipe-friendly output for LLMs
   Agent,
   /// File paths only (like `grep -l`)
   Compact,
   /// Machine-readable JSON output
   Json,
}

/// Formats search results for display.
pub trait Formatter {
   /// Formats search results into a string representation.
   fn format(&self, results: &[SearchResult], show_scores: bool, show_content: bool) -> String;
}

/// Creates a formatter instance for the specified output mode.
pub fn create_formatter(mode: OutputMode) -> Box<dyn Formatter> {
   match mode {
      OutputMode::Human => Box::new(text::HumanFormatter::new()),
      OutputMode::Agent => Box::new(text::AgentFormatter::new()),
      OutputMode::Compact => Box::new(text::CompactFormatter),
      OutputMode::Json => Box::new(json::JsonFormatter),
   }
}

/// Detects the appropriate output mode based on flags and terminal detection.
///
/// Selects JSON if `json` is true, Compact if `compact` is true, Human if
/// stdout is a TTY, otherwise Agent.
pub fn detect_output_mode(json: bool, compact: bool) -> OutputMode {
   if json {
      OutputMode::Json
   } else if compact {
      OutputMode::Compact
   } else if is_terminal::is_terminal(std::io::stdout()) {
      OutputMode::Human
   } else {
      OutputMode::Agent
   }
}

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
   if needle.is_empty() {
      return true;
   }
   let needle_bytes = needle.as_bytes();
   haystack
      .as_bytes()
      .windows(needle_bytes.len())
      .any(|window| window.eq_ignore_ascii_case(needle_bytes))
}

/// Extracts semantic tags from a search result based on chunk type and file
/// path.
///
/// Returns tags like "Definition", "Test", or "Anchor" that describe the nature
/// of the result.
pub fn get_semantic_tags(result: &SearchResult) -> Vec<&'static str> {
   use crate::types::ChunkType;
   let mut tags = Vec::new();

   if let Some(
      ChunkType::Function | ChunkType::Class | ChunkType::Interface | ChunkType::TypeAlias,
   ) = result.chunk_type
   {
      tags.push("Definition");
   }

   let path_str = result.path.to_string_lossy();
   if contains_ignore_ascii_case(&path_str, "test") || contains_ignore_ascii_case(&path_str, "spec")
   {
      tags.push("Test");
   }

   if result.is_anchor.unwrap_or(false) {
      tags.push("Anchor");
   }

   tags
}

/// Truncates a line to a maximum length, appending "..." if truncated.
pub fn truncate_line(line: &str, max_len: usize) -> Cow<'_, str> {
   if line.len() <= max_len {
      Cow::Borrowed(line)
   } else {
      Cow::Owned(format!("{}...", &line[..max_len]))
   }
}

/// Detects the programming language from a file extension for syntax
/// highlighting.
pub fn detect_language(path: &Path) -> Option<&'static str> {
   path
      .extension()
      .and_then(|ext| ext.to_str())
      .and_then(|ext| match ext.to_lowercase().as_str() {
         "ts" | "tsx" => Some("typescript"),
         "js" | "jsx" => Some("javascript"),
         "py" => Some("python"),
         "rs" => Some("rust"),
         "go" => Some("go"),
         "java" => Some("java"),
         "c" => Some("c"),
         "cpp" | "cc" | "cxx" => Some("cpp"),
         "h" | "hpp" => Some("cpp"),
         "cs" => Some("csharp"),
         "rb" => Some("ruby"),
         "php" => Some("php"),
         "swift" => Some("swift"),
         "kt" => Some("kotlin"),
         "scala" => Some("scala"),
         "json" => Some("json"),
         "xml" => Some("xml"),
         "html" | "htm" => Some("html"),
         "css" => Some("css"),
         "scss" | "sass" => Some("scss"),
         "md" => Some("markdown"),
         "yaml" | "yml" => Some("yaml"),
         "toml" => Some("toml"),
         "sh" | "bash" => Some("bash"),
         "sql" => Some("sql"),
         _ => None,
      })
}

/// Formats chunk text with contextual header information.
///
/// Constructs a header from file path and context breadcrumbs, then appends the
/// content.
pub fn format_chunk_text(context: &[String], file_path: &str, content: &str) -> String {
   let file_label = if file_path.is_empty() {
      "unknown"
   } else {
      file_path
   };
   let has_file_label = context.iter().any(|entry| entry.starts_with("File: "));

   let header = if context.is_empty() {
      format!("File: {file_label}")
   } else if has_file_label {
      context.join(" > ")
   } else {
      format!("File: {file_label} > {}", context.join(" > "))
   };

   format!("{header}\n---\n{content}")
}
