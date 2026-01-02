use std::{
   collections::{HashMap, HashSet},
   path::Path,
};

use crate::types::{SearchMode, SearchResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchBucket {
   Code,
   Docs,
   Graph,
}

pub fn bucket_for_path(path: &Path) -> SearchBucket {
   let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
   match ext.to_ascii_lowercase().as_str() {
      "mmd" | "mermaid" => SearchBucket::Graph,
      "md" | "mdx" | "markdown" | "txt" | "json" | "html" | "htm" | "css" | "yaml" | "yml"
      | "toml" => SearchBucket::Docs,
      _ => SearchBucket::Code,
   }
}

pub fn select_for_mode(
   results: Vec<SearchResult>,
   limit: usize,
   per_file_limit: usize,
   mode: SearchMode,
) -> Vec<SearchResult> {
   if limit == 0 || results.is_empty() {
      return Vec::new();
   }

   if mode == SearchMode::Balanced {
      return apply_per_file_then_truncate(results, limit, per_file_limit);
   }

   let quotas = quotas_for_mode(limit, mode);

   let all_results = results;
   let mut by_bucket: [Vec<SearchResult>; 3] = [Vec::new(), Vec::new(), Vec::new()];
   for result in &all_results {
      let bucket = bucket_for_path(&result.path);
      match bucket {
         SearchBucket::Code => by_bucket[0].push(result.clone()),
         SearchBucket::Docs => by_bucket[1].push(result.clone()),
         SearchBucket::Graph => by_bucket[2].push(result.clone()),
      }
   }

   let mut selected: Vec<SearchResult> = Vec::with_capacity(limit);
   let mut selected_keys: HashSet<(String, u32)> = HashSet::new();
   let mut per_file_counts: HashMap<String, usize> = HashMap::new();

   let mut code_selected = pick_from_bucket(
      &by_bucket[0],
      quotas.code,
      per_file_limit,
      &mut selected_keys,
      &mut per_file_counts,
   );
   let mut docs_selected = pick_from_bucket(
      &by_bucket[1],
      quotas.docs,
      per_file_limit,
      &mut selected_keys,
      &mut per_file_counts,
   );
   let mut graph_selected = pick_from_bucket(
      &by_bucket[2],
      quotas.graph,
      per_file_limit,
      &mut selected_keys,
      &mut per_file_counts,
   );

   selected.append(&mut code_selected);
   selected.append(&mut docs_selected);
   selected.append(&mut graph_selected);

   if selected.len() >= limit {
      selected.truncate(limit);
      return selected;
   }

   // Fill remaining slots with the best remaining results in overall score
   // order, preserving the input ranking.
   for result in &all_results {
      if selected.len() >= limit {
         break;
      }
      if can_take(result, per_file_limit, &mut selected_keys, &mut per_file_counts) {
         selected.push(result.clone());
      }
   }

   // Final fallback: accept anything not yet included (no per-file constraint).
   if selected.len() < limit {
      for result in &all_results {
         if selected.len() >= limit {
            break;
         }
         let key = (result.path.display().to_string(), result.start_line);
         if selected_keys.insert(key) {
            selected.push(result.clone());
         }
      }
   }

   selected.truncate(limit);
   selected
}

fn apply_per_file_then_truncate(
   mut results: Vec<SearchResult>,
   limit: usize,
   per_file_limit: usize,
) -> Vec<SearchResult> {
   if per_file_limit == 0 {
      results.truncate(limit);
      return results;
   }

   let mut counts: HashMap<String, usize> = HashMap::new();
   let mut out: Vec<SearchResult> = Vec::with_capacity(limit);

   for result in results {
      if out.len() >= limit {
         break;
      }
      let key = result.path.display().to_string();
      let count = counts.entry(key).or_insert(0);
      if *count >= per_file_limit {
         continue;
      }
      *count += 1;
      out.push(result);
   }

   out
}

fn pick_from_bucket(
   bucket_results: &[SearchResult],
   quota: usize,
   per_file_limit: usize,
   selected_keys: &mut HashSet<(String, u32)>,
   per_file_counts: &mut HashMap<String, usize>,
) -> Vec<SearchResult> {
   if quota == 0 {
      return Vec::new();
   }

   let mut out = Vec::with_capacity(quota.min(bucket_results.len()));
   for result in bucket_results {
      if out.len() >= quota {
         break;
      }
      if can_take(result, per_file_limit, selected_keys, per_file_counts) {
         out.push(result.clone());
      }
   }
   out
}

fn can_take(
   result: &SearchResult,
   per_file_limit: usize,
   selected_keys: &mut HashSet<(String, u32)>,
   per_file_counts: &mut HashMap<String, usize>,
) -> bool {
   if result.is_anchor.unwrap_or(false) {
      return false;
   }

   let path_key = result.path.display().to_string();
   let key = (path_key.clone(), result.start_line);
   if selected_keys.contains(&key) {
      return false;
   }

   if per_file_limit > 0 {
      let count = per_file_counts.get(&path_key).copied().unwrap_or(0);
      if count >= per_file_limit {
         return false;
      }
   }

   selected_keys.insert(key);
   if per_file_limit > 0 {
      let count = per_file_counts.entry(path_key).or_insert(0);
      *count += 1;
   }

   true
}

#[derive(Debug, Clone, Copy)]
struct Quotas {
   code:  usize,
   docs:  usize,
   graph: usize,
}

fn quotas_for_mode(limit: usize, mode: SearchMode) -> Quotas {
   let (w_code, w_docs, w_graph) = match mode {
      SearchMode::Discovery => (3, 4, 3),
      SearchMode::Implementation => (6, 2, 2),
      SearchMode::Planning => (2, 6, 2),
      SearchMode::Debug => (7, 2, 1),
      SearchMode::Balanced => (4, 3, 3),
   };

   let mut min = Quotas { code: 0, docs: 0, graph: 0 };
   if limit >= 3 {
      min.code = 1;
      min.docs = 1;
      min.graph = if mode == SearchMode::Debug { 0 } else { 1 };
   } else if limit == 2 {
      min.code = 1;
      min.docs = 1;
   } else if limit == 1 {
      min.code = 1;
   }

   let used = min.code + min.docs + min.graph;
   if used >= limit {
      let mut out = min;
      let overflow = used - limit;
      if overflow > 0 {
         out.graph = out.graph.saturating_sub(overflow);
      }
      return out;
   }

   let remaining = limit - used;
   let (mut a_code, mut a_docs, mut a_graph) = allocate(remaining, w_code, w_docs, w_graph);
   a_code += min.code;
   a_docs += min.docs;
   a_graph += min.graph;

   // Correct any rounding drift.
   let sum = a_code + a_docs + a_graph;
   if sum > limit {
      let extra = sum - limit;
      a_code = a_code.saturating_sub(extra);
   } else if sum < limit {
      a_code += limit - sum;
   }

   Quotas { code: a_code, docs: a_docs, graph: a_graph }
}

fn allocate(limit: usize, w_code: usize, w_docs: usize, w_graph: usize) -> (usize, usize, usize) {
   let total = w_code + w_docs + w_graph;
   if total == 0 || limit == 0 {
      return (0, 0, 0);
   }

   let mut q_code = limit * w_code / total;
   let mut q_docs = limit * w_docs / total;
   let mut q_graph = limit * w_graph / total;

   let mut remainder = limit.saturating_sub(q_code + q_docs + q_graph);
   if remainder == 0 {
      return (q_code, q_docs, q_graph);
   }

   let mut fracs = [
      (0, (limit * w_code) % total),
      (1, (limit * w_docs) % total),
      (2, (limit * w_graph) % total),
   ];
   fracs.sort_by(|a, b| b.1.cmp(&a.1));

   for (idx, _) in fracs {
      if remainder == 0 {
         break;
      }
      match idx {
         0 => q_code += 1,
         1 => q_docs += 1,
         2 => q_graph += 1,
         _ => {},
      }
      remainder -= 1;
   }

   // Any leftover remainder goes to code to preserve implementation usefulness.
   if remainder > 0 {
      q_code += remainder;
   }

   (q_code, q_docs, q_graph)
}
