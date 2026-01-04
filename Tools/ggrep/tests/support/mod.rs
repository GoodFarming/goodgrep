#![allow(dead_code)]

use ggrep::{
   Str,
   embed::{Embedder, HybridEmbedding, QueryEmbedding},
};
use ndarray::Array2;

pub struct TestEmbedder {
   dense_dim: usize,
}

impl TestEmbedder {
   pub fn new(dense_dim: usize) -> Self {
      Self { dense_dim }
   }
}

#[async_trait::async_trait]
impl Embedder for TestEmbedder {
   async fn compute_hybrid(&self, texts: &[Str]) -> ggrep::Result<Vec<HybridEmbedding>> {
      let mut out = Vec::with_capacity(texts.len());
      for text in texts {
         let mut dense = vec![0.0; self.dense_dim];
         if !dense.is_empty() {
            dense[0] = text.as_str().len() as f32;
         }
         out.push(HybridEmbedding { dense, colbert: Vec::new(), colbert_scale: 1.0 });
      }
      Ok(out)
   }

   async fn encode_query(&self, text: &str) -> ggrep::Result<QueryEmbedding> {
      let mut dense = vec![0.0; self.dense_dim];
      if !dense.is_empty() {
         dense[0] = text.len() as f32;
      }
      Ok(QueryEmbedding { dense, colbert: Array2::zeros((0, 0)) })
   }

   fn is_ready(&self) -> bool {
      true
   }
}

pub fn set_temp_home(dir: &tempfile::TempDir) {
   // Safe in test harness: set before touching config paths to isolate data.
   unsafe {
      std::env::set_var("HOME", dir.path());
   }
}
