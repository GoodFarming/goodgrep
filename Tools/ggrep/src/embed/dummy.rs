//! Lightweight deterministic embedder for tests and tooling.

use ndarray::Array2;

use crate::{
   Str,
   embed::{Embedder, HybridEmbedding, QueryEmbedding},
   error::Result,
};

#[derive(Debug, Clone)]
pub struct DummyEmbedder {
   dense_dim: usize,
}

impl DummyEmbedder {
   pub fn new(dense_dim: usize) -> Self {
      Self { dense_dim }
   }
}

#[async_trait::async_trait]
impl Embedder for DummyEmbedder {
   async fn compute_hybrid(&self, texts: &[Str]) -> Result<Vec<HybridEmbedding>> {
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

   async fn encode_query(&self, text: &str) -> Result<QueryEmbedding> {
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
