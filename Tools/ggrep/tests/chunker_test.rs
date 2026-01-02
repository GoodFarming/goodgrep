use std::path::Path;

use ggrep::{
   Str,
   chunker::{Chunker, anchor::create_anchor_chunk},
   types::ChunkType,
};

#[test]
fn test_create_anchor_chunk() {
   let content = Str::from_static(
      r"
// This is a comment
import { foo } from 'bar';
export const baz = 42;

function test() {
  return true;
}
",
   );
   let path = Path::new("test.ts");
   let chunk = create_anchor_chunk(&content, path);

   assert!(chunk.is_anchor.unwrap_or(false));
   assert_eq!(chunk.chunk_type, Some(ChunkType::Block));
   assert!(chunk.content.as_str().contains("Imports:"));
   assert!(chunk.content.as_str().contains("Exports:"));
}

#[tokio::test]
async fn test_treesitter_chunker_typescript() {
   let chunker = Chunker::default();
   let content = Str::from_static(
      r"
export function greet(name: string): string {
  return `Hello, ${name}`;
}

export class Person {
  constructor(private name: string) {}

  getName(): string {
    return this.name;
  }
}
",
   );
   let path = Path::new("test.ts");

   let result = chunker.chunk(&content, path).await;
   assert!(result.is_ok());
   let chunks = result.unwrap();

   assert!(!chunks.is_empty());
   let has_function = chunks
      .iter()
      .any(|c| c.chunk_type == Some(ChunkType::Function));
   let has_class = chunks
      .iter()
      .any(|c| c.chunk_type == Some(ChunkType::Class));

   assert!(has_function || has_class);
}

#[tokio::test]
async fn test_markdown_chunking_produces_non_empty_chunks() {
   let chunker = Chunker::default();
   let content = Str::from_static(
      r#"
# Project Plan

Intro text.

## Irrigation

- Check soil moisture
- Decide irrigation window

## Harvest

Details here.
"#,
   );
   let path = Path::new("plan.md");

   let chunks = chunker.chunk(&content, path).await.unwrap();
   assert!(!chunks.is_empty());
   assert!(
      chunks
         .iter()
         .any(|c| c.content.as_str().contains("## Irrigation"))
   );
}
