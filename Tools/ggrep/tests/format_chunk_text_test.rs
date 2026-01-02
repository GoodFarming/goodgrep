use ggrep::format::format_chunk_text;

#[test]
fn test_format_chunk_text_with_context() {
   let context = vec!["class UserService".to_string(), "method authenticate".to_string()];
   let file_path = "src/auth.rs";
   let content = "pub fn authenticate() {\n    verify_token()\n}";

   let result = format_chunk_text(&context, file_path, content);

   assert!(result.contains("File: src/auth.rs"));
   assert!(result.contains("class UserService"));
   assert!(result.contains("method authenticate"));
   assert!(result.contains("---"));
   assert!(result.contains("pub fn authenticate()"));
}

#[test]
fn test_format_chunk_text_with_file_label_already_present() {
   let context = vec!["File: src/lib.rs".to_string(), "module network".to_string()];
   let file_path = "src/lib.rs";
   let content = "pub mod network;";

   let result = format_chunk_text(&context, file_path, content);

   assert_eq!(result.lines().next().unwrap(), "File: src/lib.rs > module network");
   assert!(result.contains("---"));
   assert!(result.contains("pub mod network;"));
}

#[test]
fn test_format_chunk_text_empty_context() {
   let context: Vec<String> = vec![];
   let file_path = "test.rs";
   let content = "fn main() {}";

   let result = format_chunk_text(&context, file_path, content);

   assert!(result.starts_with("File: test.rs\n---\n"));
   assert!(result.contains("fn main() {}"));
}

#[test]
fn test_format_chunk_text_empty_file_path() {
   let context = vec!["function main".to_string()];
   let file_path = "";
   let content = "println!(\"Hello\");";

   let result = format_chunk_text(&context, file_path, content);

   assert!(result.contains("File: unknown"));
   assert!(result.contains("function main"));
   assert!(result.contains("---"));
}

#[test]
fn test_format_chunk_text_separator() {
   let context = vec!["context1".to_string(), "context2".to_string()];
   let file_path = "file.rs";
   let content = "code";

   let result = format_chunk_text(&context, file_path, content);

   assert!(result.contains("File: file.rs > context1 > context2"));
   assert!(result.contains("\n---\n"));
   assert!(result.ends_with("code"));
}
