//! Utility functions for filesystem operations

use std::{fs, path::Path};

use crate::Result;

/// Converts raw relevance scores into a stable, relative 0–99 "match %" scale.
///
/// The goal is a simple, human/agent-friendly indicator that:
/// - is monotonic with the raw score,
/// - is comparable *within a single result set* (not globally calibrated),
/// - does not always force the top hit to 100%.
pub fn compute_match_pcts(scores: &[f32]) -> Vec<Option<u8>> {
   if scores.is_empty() {
      return Vec::new();
   }

   let finite: Vec<f64> = scores
      .iter()
      .copied()
      .filter(|s| s.is_finite())
      .map(|s| s as f64)
      .collect();

   // Fallback for degenerate cases (0–1 finite scores): show a bounded absolute
   // scale so the UI can still surface something.
   if finite.len() < 2 {
      return scores
         .iter()
         .map(|&s| {
            if !s.is_finite() {
               return None;
            }
            let pct = (s as f64 * 100.0).round().clamp(0.0, 99.0) as u8;
            Some(pct)
         })
         .collect();
   }

   let mean = finite.iter().sum::<f64>() / finite.len() as f64;
   let var = finite
      .iter()
      .map(|v| {
         let d = v - mean;
         d * d
      })
      .sum::<f64>()
      / finite.len() as f64;
   let std = var.sqrt();

   if !std.is_finite() || std <= 1e-9 {
      return scores
         .iter()
         .map(|&s| {
            if !s.is_finite() {
               return None;
            }
            let pct = (s as f64 * 100.0).round().clamp(0.0, 99.0) as u8;
            Some(pct)
         })
         .collect();
   }

   scores
      .iter()
      .map(|&s| {
         if !s.is_finite() {
            return None;
         }
         let z = (s as f64 - mean) / std;
         let sigmoid = 1.0 / (1.0 + (-z).exp());
         let pct = (sigmoid * 100.0).round().clamp(0.0, 99.0) as u8;
         Some(pct)
      })
      .collect()
}

/// Recursively calculates the total size of a directory in bytes
pub fn get_dir_size(path: &Path) -> Result<u64> {
   let mut total = 0u64;

   if path.is_dir() {
      for entry in fs::read_dir(path)? {
         let entry = entry?;
         let metadata = entry.metadata()?;

         if metadata.is_dir() {
            total += get_dir_size(&entry.path())?;
         } else {
            total += metadata.len();
         }
      }
   }

   Ok(total)
}

/// Formats a byte count as a human-readable size string
pub fn format_size(bytes: u64) -> String {
   const KB: u64 = 1024;
   const MB: u64 = KB * 1024;
   const GB: u64 = MB * 1024;

   if bytes < KB {
      format!("{bytes} B")
   } else if bytes < MB {
      format!("{:.1} KB", bytes as f64 / KB as f64)
   } else if bytes < GB {
      format!("{:.1} MB", bytes as f64 / MB as f64)
   } else {
      format!("{:.1} GB", bytes as f64 / GB as f64)
   }
}
