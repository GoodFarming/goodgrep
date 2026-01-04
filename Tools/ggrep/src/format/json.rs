//! JSON output formatter for search results.

use serde::Serialize;

use super::Formatter;
use crate::types::SearchResult;

#[derive(Debug, Serialize)]
struct JsonOutput {
   results: Vec<JsonResult>,
   count:   usize,
}

#[derive(Debug, Serialize)]
struct JsonResult {
   path:       String,
   content:    String,
   score:      f32,
   chunk_type: String,
   start_line: u32,
   num_lines:  u32,
   is_anchor:  bool,
}

impl From<&SearchResult> for JsonResult {
   fn from(result: &SearchResult) -> Self {
      let chunk_type = result
         .chunk_type
         .map_or_else(String::new, |ct| ct.as_lowercase_str().to_string());

      Self {
         path: result.path.display().to_string(),
         content: result.content.to_string(),
         score: result.score,
         chunk_type,
         start_line: result.start_line,
         num_lines: result.num_lines,
         is_anchor: result.is_anchor.unwrap_or(false),
      }
   }
}

/// Formats search results as JSON with structured metadata.
pub struct JsonFormatter;

impl Formatter for JsonFormatter {
   fn format(&self, results: &[SearchResult], _show_scores: bool, _show_content: bool) -> String {
      let json_results: Vec<JsonResult> = results.iter().map(JsonResult::from).collect();

      let output = JsonOutput { count: json_results.len(), results: json_results };

      serde_json::to_string(&output).unwrap_or_else(|_| r#"{"results":[],"count":0}"#.to_string())
   }
}

#[cfg(test)]
mod tests {
   use super::*;
   use crate::types::ChunkType;

   #[test]
   fn test_json_formatter() {
      let results = vec![
         SearchResult {
            path:            "src/main.rs".into(),
            content:         "fn main() {}".into(),
            score:           0.95,
            secondary_score: None,
            row_id:          None,
            segment_table:   None,
            start_line:      10,
            num_lines:       1,
            chunk_type:      Some(ChunkType::Function),
            is_anchor:       Some(false),
         },
         SearchResult {
            path:            "src/lib.rs".into(),
            content:         "pub fn test() {}".into(),
            score:           0.87,
            secondary_score: None,
            row_id:          None,
            segment_table:   None,
            start_line:      5,
            num_lines:       1,
            chunk_type:      Some(ChunkType::Function),
            is_anchor:       Some(true),
         },
      ];

      let formatter = JsonFormatter;
      let output = formatter.format(&results, false, false);

      assert!(output.contains("\"count\":2"));
      assert!(output.contains("src/main.rs"));
      assert!(output.contains("src/lib.rs"));
      assert!(output.contains("\"score\":0.95"));
      assert!(output.contains("\"is_anchor\":true"));
      assert!(output.contains("\"chunk_type\":\"function\""));
   }

   #[test]
   fn test_json_formatter_empty() {
      let results = vec![];
      let formatter = JsonFormatter;
      let output = formatter.format(&results, false, false);

      assert!(output.contains("\"count\":0"));
      assert!(output.contains("\"results\":[]"));
   }
}
