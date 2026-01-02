//! Index-time preprocessing to improve recall on non-code artifacts.
//!
//! This module is designed to be "no-touch" for the repository: it does not
//! require creating or maintaining additional reference docs. Instead it
//! augments text that is fed into embedding models (dense + ColBERT) to improve
//! recall for hybrid corpora (plans, docs, Mermaid diagrams, etc.).

use std::{collections::HashMap, path::Path};

use regex::Regex;

use crate::{Str, config};

const MAX_MERMAID_EDGES: usize = 48;
const MAX_MERMAID_MESSAGES: usize = 48;
const MAX_MERMAID_LINKS: usize = 24;
const MAX_MERMAID_NODES: usize = 48;

pub fn augment_for_embedding(content: &Str, path: &Path) -> Str {
   let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
   let is_mermaid_file = matches!(ext.to_ascii_lowercase().as_str(), "mmd" | "mermaid");

   let mut summaries = Vec::new();

   if is_mermaid_file {
      if let Some(summary) = summarize_mermaid_block(content.as_str()) {
         summaries.push(summary);
      }
   } else {
      for block in extract_mermaid_fences(content.as_str()) {
         if let Some(summary) = summarize_mermaid_block(&block) {
            summaries.push(summary);
         }
      }
   }

   if summaries.is_empty() {
      return content.clone();
   }

   let mut out =
      String::with_capacity(content.len() + 256 + summaries.iter().map(String::len).sum::<usize>());
   out.push_str("[ggrep-mermaid-summary]\n");
   for summary in summaries {
      out.push_str(&summary);
      out.push('\n');
   }
   out.push('\n');
   out.push_str(content.as_str());
   Str::from_string(out)
}

pub fn prepare_for_embedding(content: &Str, path: &Path) -> Str {
   let augmented = augment_for_embedding(content, path);
   let prefix = config::get().doc_prefix.as_str();
   if prefix.is_empty() || !should_apply_doc_prefix(path) {
      return augmented;
   }

   let mut out = String::with_capacity(prefix.len() + augmented.len());
   out.push_str(prefix);
   out.push_str(augmented.as_str());
   Str::from_string(out)
}

fn should_apply_doc_prefix(path: &Path) -> bool {
   let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
   matches!(
      ext.to_ascii_lowercase().as_str(),
      "mmd" | "mermaid" | "md" | "mdx" | "markdown" | "txt" | "json" | "yaml" | "yml" | "toml"
   )
}

fn extract_mermaid_fences(input: &str) -> Vec<String> {
   let mut blocks = Vec::new();

   let mut in_mermaid = false;
   let mut fence_delim: Option<&str> = None;
   let mut buf = String::new();

   for line in input.lines() {
      let trimmed = line.trim_end();
      let start = strip_blockquote_prefix(trimmed);

      if !in_mermaid {
         if let Some(delim) = parse_mermaid_fence_start(start) {
            in_mermaid = true;
            fence_delim = Some(delim);
            buf.clear();
         }
         continue;
      }

      if let Some(delim) = fence_delim
         && strip_blockquote_prefix(trimmed).starts_with(delim)
      {
         in_mermaid = false;
         fence_delim = None;
         let block = buf.trim();
         if !block.is_empty() {
            blocks.push(block.to_string());
         }
         buf.clear();
         continue;
      }

      let stripped_line = strip_blockquote_prefix(line);
      buf.push_str(stripped_line);
      buf.push('\n');
   }

   blocks
}

fn strip_blockquote_prefix(line: &str) -> &str {
   let mut current = line.trim_start();
   loop {
      if let Some(rest) = current.strip_prefix('>') {
         current = rest.trim_start();
      } else {
         return current;
      }
   }
}

fn parse_mermaid_fence_start(line: &str) -> Option<&'static str> {
   let start = line.trim_start();
   if let Some(rest) = start.strip_prefix("```") {
      if rest
         .trim_start()
         .to_ascii_lowercase()
         .starts_with("mermaid")
      {
         return Some("```");
      }
   }
   if let Some(rest) = start.strip_prefix("~~~") {
      if rest
         .trim_start()
         .to_ascii_lowercase()
         .starts_with("mermaid")
      {
         return Some("~~~");
      }
   }
   None
}

fn summarize_mermaid_block(block: &str) -> Option<String> {
   let first = block
      .lines()
      .map(str::trim)
      .find(|l| !l.is_empty())
      .unwrap_or("");
   if first.is_empty() {
      return None;
   }

   let first_lower = first.to_ascii_lowercase();
   if first_lower.starts_with("graph ") || first_lower.starts_with("flowchart ") {
      return summarize_flowchart(block);
   }
   if first_lower.starts_with("sequencediagram") {
      return summarize_sequence(block);
   }

   // Fallback: still try flowchart heuristics even if the block doesn't start
   // with an explicit "graph"/"flowchart" line.
   summarize_flowchart(block).or_else(|| summarize_sequence(block))
}

#[derive(Clone)]
struct FlowEdge {
   src_id:    String,
   src_label: Option<String>,
   dst_id:    String,
   dst_label: Option<String>,
   label:     Option<String>,
}

#[derive(Clone)]
struct NodeLink {
   id:     String,
   target: String,
}

fn summarize_flowchart(block: &str) -> Option<String> {
   let mut edges = Vec::new();
   let mut links = Vec::new();
   let mut node_labels = HashMap::new();
   let mut node_defs = Vec::new();

   for raw in block.lines() {
      let line = raw.trim();
      if line.is_empty() {
         continue;
      }
      if is_flowchart_directive(line) {
         continue;
      }
      if let Some(link) = parse_click_link(line) {
         if links.len() < MAX_MERMAID_LINKS {
            links.push(link);
         }
         continue;
      }
      if let Some(edge) = parse_flow_edge(line) {
         edges.push(edge);
         continue;
      }
      if let Some((id, label)) = parse_node_definition(line) {
         if !node_labels.contains_key(&id) {
            node_labels.insert(id.clone(), label.clone());
            node_defs.push((id, label));
         }
      }
   }

   let mut edge_summaries = Vec::new();
   for edge in edges {
      let src_name = edge
         .src_label
         .or_else(|| node_labels.get(&edge.src_id).cloned())
         .unwrap_or(edge.src_id);
      let dst_name = edge
         .dst_label
         .or_else(|| node_labels.get(&edge.dst_id).cloned())
         .unwrap_or(edge.dst_id);
      edge_summaries.push(format_edge_names(&src_name, &dst_name, edge.label.as_deref()));
      if edge_summaries.len() >= MAX_MERMAID_EDGES {
         break;
      }
   }

   let mut link_summaries = Vec::new();
   for link in links {
      let label = node_labels
         .get(&link.id)
         .map(String::as_str)
         .unwrap_or(&link.id);
      link_summaries.push(format!("{label} links to {}", link.target));
   }

   if edge_summaries.is_empty() && link_summaries.is_empty() {
      if node_defs.is_empty() {
         return None;
      }
      let mut node_summaries = Vec::new();
      for (id, label) in node_defs.into_iter().take(MAX_MERMAID_NODES) {
         if label == id {
            node_summaries.push(label);
         } else {
            node_summaries.push(format!("{label} ({id})"));
         }
      }
      let mut out = String::from("Mermaid flowchart nodes: ");
      out.push_str(&node_summaries.join("; "));
      return Some(out);
   }

   let mut out = String::new();
   if !edge_summaries.is_empty() {
      out.push_str("Mermaid flowchart edges: ");
      out.push_str(&edge_summaries.join("; "));
   }
   if !link_summaries.is_empty() {
      if !out.is_empty() {
         out.push('\n');
      }
      out.push_str("Mermaid flowchart links: ");
      out.push_str(&link_summaries.join("; "));
   }
   Some(out)
}

fn is_flowchart_directive(line: &str) -> bool {
   let lower = line.trim_start().to_ascii_lowercase();
   lower.starts_with("graph ")
      || lower.starts_with("flowchart ")
      || lower.starts_with("%%")
      || lower.starts_with("subgraph ")
      || lower == "end"
      || lower.starts_with("classdef ")
      || lower.starts_with("class ")
      || lower.starts_with("style ")
      || lower.starts_with("linkstyle ")
      || lower.starts_with("direction ")
}

fn parse_flow_edge(line: &str) -> Option<FlowEdge> {
   // Common patterns:
   // - A --> B
   // - A -- Yes --> B
   // - A -->|Yes| B
   // - A[Start] --> B{Decision}
   let arrow_pos = line.find("-->")?;

   let (lhs, rhs_with_arrow) = line.split_at(arrow_pos);
   let rhs = rhs_with_arrow.trim_start_matches("-->").trim();

   let mut src = lhs.trim();
   let mut label: Option<&str> = None;

   // A -- label
   if let Some((before, after)) = src.rsplit_once("--") {
      let candidate_label = after.trim();
      if !candidate_label.is_empty() {
         src = before.trim();
         label = Some(candidate_label);
      }
   }

   // A -->|label| B
   if label.is_none() && rhs.starts_with('|') {
      if let Some(end) = rhs[1..].find('|') {
         let label_text = rhs[1..(1 + end)].trim();
         label = Some(label_text);
         let rest = rhs[(end + 2)..].trim();
         return Some(build_flow_edge(src, rest, label));
      }
   }

   Some(build_flow_edge(src, rhs, label))
}

fn build_flow_edge(src_raw: &str, dst_raw: &str, label: Option<&str>) -> FlowEdge {
   let (src_id, src_label) = parse_node(src_raw);
   let (dst_id, dst_label) = parse_node(dst_raw);

   FlowEdge { src_id, src_label, dst_id, dst_label, label: label.map(str::to_string) }
}

fn parse_node_definition(line: &str) -> Option<(String, String)> {
   if line.contains("-->") {
      return None;
   }
   let (id, label) = parse_node(line);
   let label = label?;
   if id.is_empty() || label.is_empty() {
      return None;
   }
   Some((id, label))
}

fn parse_click_link(line: &str) -> Option<NodeLink> {
   let trimmed = line.trim_start();
   if !trimmed.to_ascii_lowercase().starts_with("click ") {
      return None;
   }
   let after = trimmed[5..].trim_start();
   let (id, rest) = split_first_token(after)?;
   let mut rest = rest.trim_start();
   if rest.is_empty() {
      return None;
   }

   if let Some((token, remaining)) = split_first_token(rest) {
      if token.eq_ignore_ascii_case("href") || token.eq_ignore_ascii_case("call") {
         rest = remaining.trim_start();
      }
   }

   if let Some(quoted) = first_quoted_string(rest) {
      return Some(NodeLink { id: id.to_string(), target: quoted });
   }

   let (target, _) = split_first_token(rest)?;
   Some(NodeLink { id: id.to_string(), target: target.to_string() })
}

fn split_first_token(input: &str) -> Option<(&str, &str)> {
   let trimmed = input.trim_start();
   if trimmed.is_empty() {
      return None;
   }
   let mut iter = trimmed.splitn(2, char::is_whitespace);
   let token = iter.next().unwrap_or("");
   if token.is_empty() {
      return None;
   }
   let rest = iter.next().unwrap_or("");
   Some((token, rest))
}

fn first_quoted_string(input: &str) -> Option<String> {
   let mut chars = input.char_indices();
   while let Some((start_idx, ch)) = chars.next() {
      if ch == '"' || ch == '\'' {
         let rest = &input[start_idx + ch.len_utf8()..];
         if let Some(end_idx) = rest.find(ch) {
            return Some(rest[..end_idx].to_string());
         }
      }
   }
   None
}

fn format_edge_names(src_name: &str, dst_name: &str, label: Option<&str>) -> String {
   if let Some(l) = label
      && !l.is_empty()
   {
      format!("{src_name} -[{l}]-> {dst_name}")
   } else {
      format!("{src_name} -> {dst_name}")
   }
}

fn parse_node(input: &str) -> (String, Option<String>) {
   // Examples:
   // - A
   // - A[Start]
   // - B{Decision?}
   // - C((Round))
   let stripped = strip_mermaid_decorators(input);
   let s = stripped.trim().trim_end_matches(';').trim();
   if s.is_empty() {
      return (String::new(), None);
   }

   static NODE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
      Regex::new(r#"^(?P<id>[A-Za-z0-9_.$:-]+)\s*(?P<label>\[.*\]|\{.*\}|\(\(.*\)\)|\(.*\))?\s*$"#)
         .unwrap()
   });

   if let Some(caps) = NODE_RE.captures(s) {
      let id = caps.name("id").map(|m| m.as_str()).unwrap_or(s).to_string();
      let label = caps.name("label").map(|m| m.as_str().to_string());
      let cleaned = label.as_deref().map(clean_mermaid_label);
      return (id, cleaned);
   }

   (s.to_string(), None)
}

fn strip_mermaid_decorators(input: &str) -> &str {
   let trimmed = input.trim();
   trimmed.split(":::").next().unwrap_or(trimmed).trim()
}

fn clean_mermaid_label(raw: &str) -> String {
   let mut s = raw.trim().to_string();
   for (open, close) in [('[', ']'), ('{', '}'), ('(', ')')] {
      if s.starts_with(open) && s.ends_with(close) && s.len() >= 2 {
         s = s[1..s.len() - 1].to_string();
      }
   }
   let trimmed = s.trim();
   if (trimmed.starts_with('"') && trimmed.ends_with('"'))
      || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
   {
      s = trimmed[1..trimmed.len() - 1].to_string();
   } else {
      s = trimmed.to_string();
   }
   s.trim().to_string()
}

fn summarize_sequence(block: &str) -> Option<String> {
   let mut messages = Vec::new();

   for raw in block.lines() {
      let line = raw.trim();
      if line.is_empty() || line.starts_with("sequenceDiagram") || line.starts_with("%%") {
         continue;
      }
      if let Some(msg) = parse_sequence_message(line) {
         messages.push(msg);
         if messages.len() >= MAX_MERMAID_MESSAGES {
            break;
         }
      }
   }

   if messages.is_empty() {
      return None;
   }

   let mut out = String::from("Mermaid sequence messages: ");
   out.push_str(&messages.join("; "));
   Some(out)
}

fn parse_sequence_message(line: &str) -> Option<String> {
   // Examples:
   // - Alice->>Bob: Hello
   // - Alice-->>Bob: Hello
   let (arrow, parts) = if let Some(pos) = line.find("-->>") {
      ("-->>", pos)
   } else if let Some(pos) = line.find("->>") {
      ("->>", pos)
   } else {
      return None;
   };

   let (lhs, rhs_with_arrow) = line.split_at(parts);
   let rhs = rhs_with_arrow.trim_start_matches(arrow).trim();

   let (dst, msg) = rhs.split_once(':').unwrap_or((rhs, ""));
   let src = lhs.trim();
   let dst = dst.trim();
   let msg = msg.trim();

   if src.is_empty() || dst.is_empty() {
      return None;
   }

   if msg.is_empty() {
      Some(format!("{src} -> {dst}"))
   } else {
      Some(format!("{src} -> {dst}: {msg}"))
   }
}

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn augments_markdown_mermaid_blocks() {
      let content = Str::from_static(
         r#"
# Plan

```mermaid
graph TD
  A[Start] --> B{Soil moisture low?}
  B -- Yes --> C[Run irrigation]
  B -- No --> D[Wait]
```
"#,
      );
      let augmented = augment_for_embedding(&content, Path::new("plan.md"));
      assert!(augmented.as_str().contains("Mermaid flowchart edges:"));
      assert!(augmented.as_str().contains("Soil moisture low?"));
      assert!(augmented.as_str().contains("Run irrigation"));
   }

   #[test]
   fn augments_mmd_files() {
      let content = Str::from_static("graph TD\nA[Start] --> B[End]\n");
      let augmented = augment_for_embedding(&content, Path::new("diagram.mmd"));
      assert!(augmented.as_str().contains("Mermaid flowchart edges:"));
      assert!(augmented.as_str().contains("Start"));
      assert!(augmented.as_str().contains("End"));
   }

   #[test]
   fn augments_mermaid_with_click_and_node_labels() {
      let content = Str::from_static(
         r#"
```mermaid
graph TD
  A["Engine/Docs/Plan/README.md"]
  B["Engine/Docs/Plan/Workstream.md"]
  A --> B
  click A "Engine/Docs/Plan/README.md"
```
"#,
      );
      let augmented = augment_for_embedding(&content, Path::new("plan.md"));
      let text = augmented.as_str();
      assert!(text.contains("Engine/Docs/Plan/README.md -> Engine/Docs/Plan/Workstream.md"));
      assert!(text.contains("links to Engine/Docs/Plan/README.md"));
   }

   #[test]
   fn augments_blockquoted_mermaid_fences() {
      let content = Str::from_static(
         r#"
> ``` mermaid
> flowchart TD
>   A[Start] --> B[End]
> ```
"#,
      );
      let augmented = augment_for_embedding(&content, Path::new("notes.md"));
      assert!(augmented.as_str().contains("Mermaid flowchart edges:"));
      assert!(augmented.as_str().contains("Start -> End"));
   }
}
