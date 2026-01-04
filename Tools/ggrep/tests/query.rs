use std::path::PathBuf;

use ggrep::{
   SearchLimitHit, SearchResult, SearchWarning, Str, sort_and_dedup_limits,
   sort_and_dedup_warnings, sort_results_deterministic,
};

#[test]
fn deterministic_ordering_tiebreak() {
   let mut results = vec![
      SearchResult {
         path:            PathBuf::from("b.rs"),
         content:         Str::default(),
         score:           1.0,
         secondary_score: None,
         row_id:          Some("b2".to_string()),
         segment_table:   None,
         start_line:      10,
         num_lines:       1,
         chunk_type:      None,
         is_anchor:       None,
      },
      SearchResult {
         path:            PathBuf::from("a.rs"),
         content:         Str::default(),
         score:           1.0,
         secondary_score: None,
         row_id:          Some("a1".to_string()),
         segment_table:   None,
         start_line:      5,
         num_lines:       1,
         chunk_type:      None,
         is_anchor:       None,
      },
      SearchResult {
         path:            PathBuf::from("a.rs"),
         content:         Str::default(),
         score:           1.0,
         secondary_score: None,
         row_id:          Some("a0".to_string()),
         segment_table:   None,
         start_line:      5,
         num_lines:       1,
         chunk_type:      None,
         is_anchor:       None,
      },
   ];

   sort_results_deterministic(&mut results);

   assert_eq!(results[0].path, PathBuf::from("a.rs"));
   assert_eq!(results[0].row_id.as_deref(), Some("a0"));
   assert_eq!(results[1].row_id.as_deref(), Some("a1"));
   assert_eq!(results[2].path, PathBuf::from("b.rs"));
}

#[test]
fn deterministic_limits_and_warnings_ordering() {
   let mut limits = vec![
      SearchLimitHit {
         code:     "b".to_string(),
         limit:    1,
         observed: None,
         path_key: Some("b".to_string()),
      },
      SearchLimitHit {
         code:     "a".to_string(),
         limit:    1,
         observed: None,
         path_key: Some("b".to_string()),
      },
      SearchLimitHit { code: "a".to_string(), limit: 1, observed: None, path_key: None },
      SearchLimitHit { code: "a".to_string(), limit: 1, observed: None, path_key: None },
      SearchLimitHit {
         code:     "a".to_string(),
         limit:    1,
         observed: None,
         path_key: Some("a".to_string()),
      },
   ];

   sort_and_dedup_limits(&mut limits);
   assert_eq!(limits.len(), 4);
   assert_eq!(limits[0].code, "a");
   assert!(limits[0].path_key.is_none());
   assert_eq!(limits[1].path_key.as_deref(), Some("a"));
   assert_eq!(limits[2].path_key.as_deref(), Some("b"));
   assert_eq!(limits[3].code, "b");

   let mut warnings = vec![
      SearchWarning {
         code:     "warn-b".to_string(),
         message:  "x".to_string(),
         path_key: Some("b".to_string()),
      },
      SearchWarning { code: "warn-a".to_string(), message: "x".to_string(), path_key: None },
      SearchWarning { code: "warn-a".to_string(), message: "x".to_string(), path_key: None },
      SearchWarning {
         code:     "warn-a".to_string(),
         message:  "x".to_string(),
         path_key: Some("a".to_string()),
      },
   ];

   sort_and_dedup_warnings(&mut warnings);
   assert_eq!(warnings.len(), 3);
   assert_eq!(warnings[0].code, "warn-a");
   assert!(warnings[0].path_key.is_none());
   assert_eq!(warnings[1].path_key.as_deref(), Some("a"));
   assert_eq!(warnings[2].code, "warn-b");
}
