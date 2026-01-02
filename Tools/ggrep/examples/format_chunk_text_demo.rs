use ggrep::format::format_chunk_text;

fn main() {
   let context = vec!["class UserService".to_string(), "method authenticate".to_string()];
   let file_path = "src/auth.rs";
   let content = "pub fn authenticate() {\n    verify_token()\n}";

   let result = format_chunk_text(&context, file_path, content);
   println!("Example 1 - With context:");
   println!("{result}\n");

   let context = vec!["File: src/lib.rs".to_string(), "module network".to_string()];
   let file_path = "src/lib.rs";
   let content = "pub mod network;";

   let result = format_chunk_text(&context, file_path, content);
   println!("Example 2 - File label already present:");
   println!("{result}\n");

   let context: Vec<String> = vec![];
   let file_path = "test.rs";
   let content = "fn main() {}";

   let result = format_chunk_text(&context, file_path, content);
   println!("Example 3 - Empty context:");
   println!("{result}\n");

   let context = vec!["function main".to_string()];
   let file_path = "";
   let content = "println!(\"Hello\");";

   let result = format_chunk_text(&context, file_path, content);
   println!("Example 4 - Empty file path:");
   println!("{result}\n");
}
