//! Semantic search command.
//!
//! Performs semantic code search using vector similarity, with support for
//! daemon-based or direct execution, JSON output, and various formatting
//! options.

use std::{
   path::{Path, PathBuf},
   sync::Arc,
   time::Duration,
};

use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use tokio::time;

use crate::{
   Result,
   chunker::Chunker,
   cmd::daemon,
   config,
   embed::worker::EmbedWorker,
   error::Error,
   file::LocalFileSystem,
   git,
   ipc::{self, Request, Response},
   search::SearchEngine,
   store::LanceStore,
   sync::SyncEngine,
   types::{SearchMode, SearchStatus},
   usock,
};

/// A single search result with metadata and content.
#[derive(Debug, Serialize, Deserialize)]
struct SearchResult {
   path:       PathBuf,
   score:      f32,
   #[serde(skip_serializing_if = "Option::is_none")]
   match_pct:  Option<u8>,
   content:    String,
   #[serde(skip_serializing_if = "Option::is_none")]
   chunk_type: Option<String>,
   #[serde(skip_serializing_if = "Option::is_none")]
   start_line: Option<usize>,
   #[serde(skip_serializing_if = "Option::is_none")]
   end_line:   Option<usize>,
   #[serde(skip_serializing_if = "Option::is_none")]
   is_anchor:  Option<bool>,
}

/// JSON output format for search results.
#[derive(Debug, Serialize)]
struct JsonOutput {
   results: Vec<SearchResult>,
}

#[derive(Debug)]
struct DaemonSearchOutcome {
   results:  Vec<SearchResult>,
   status:   SearchStatus,
   progress: Option<u8>,
}

/// Command-line options for search behavior.
#[derive(Default, Debug, Clone, Copy)]
pub struct SearchOptions {
   pub content:       bool,
   pub no_snippet:    bool,
   pub short_snippet: bool,
   pub long_snippet:  bool,
   pub compact:       bool,
   pub scores:        bool,
   pub sync:          bool,
   pub dry_run:       bool,
   pub json:          bool,
   pub no_rerank:     bool,
   pub plain:         bool,
   pub mode:          SearchMode,
}

#[derive(Debug, Clone, Copy)]
enum SnippetMode {
   Default,
   Short,
   Long,
   Full,
   None,
}

impl Default for SnippetMode {
   fn default() -> Self {
      Self::Default
   }
}

/// Options for formatting search results in human-readable output.
#[derive(Default, Debug, Clone, Copy)]
struct FormatOptions {
   compact:      bool,
   scores:       bool,
   plain:        bool,
   snippet_mode: SnippetMode,
   mode:         SearchMode,
}

/// Executes a semantic code search.
pub async fn execute(
   query: String,
   path: Option<PathBuf>,
   max: usize,
   per_file: usize,
   options: SearchOptions,
   eval_store: bool,
   store_id: Option<String>,
) -> Result<()> {
   let cwd = std::env::current_dir()?.canonicalize()?;
   // Default to searching "here" (current directory) while still using the
   // repo-root store when in a git repo.
   let filter_path = path.unwrap_or_else(|| cwd.clone()).canonicalize()?;
   let index_root = git::get_repo_root(&filter_path).unwrap_or_else(|| filter_path.clone());
   let index_root = index_root.canonicalize()?;

   let resolved_store_id = match store_id {
      Some(s) => {
         if eval_store && !s.ends_with("-eval") {
            format!("{s}-eval")
         } else {
            s
         }
      },
      None => {
         let base = git::resolve_store_id(&index_root)?;
         if eval_store {
            format!("{base}-eval")
         } else {
            base
         }
      },
   };

   if options.dry_run {
      if options.json {
         println!("{}", serde_json::to_string(&JsonOutput { results: vec![] })?);
      } else {
         println!("Dry run: would search for '{query}' in {}", index_root.display());
         if filter_path != index_root {
            println!("Scope: {}", filter_path.display());
         }
         println!("Store ID: {resolved_store_id}");
         println!("Max results: {max}");
      }
      return Ok(());
   }

   let request_path = (filter_path != index_root).then_some(filter_path.as_path());

   if let Some(outcome) = try_daemon_search(
      &query,
      max,
      per_file,
      options.mode,
      !options.no_rerank,
      &index_root,
      request_path,
      &resolved_store_id,
   )
   .await?
   {
      if options.json {
         println!("{}", serde_json::to_string(&JsonOutput { results: outcome.results })?);
      } else {
         let snippet_mode = resolve_snippet_mode(options);
         let format_opts = FormatOptions {
            compact: options.compact,
            scores: options.scores,
            plain: options.plain,
            snippet_mode,
            mode: options.mode,
         };
         if outcome.results.is_empty() {
            format_empty_results(
               &query,
               &index_root,
               request_path,
               outcome.status,
               outcome.progress,
               format_opts,
            );
         } else {
            format_results(
               &outcome.results,
               &query,
               &index_root,
               request_path,
               format_opts,
               outcome.status,
               outcome.progress,
            );
         }
      }
      return Ok(());
   }

   if options.sync && !options.json {
      let spinner = ProgressBar::new_spinner();
      spinner.set_style(
         ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
      );
      spinner.enable_steady_tick(Duration::from_millis(100));
      spinner.set_message("Syncing files to index...");

      time::sleep(Duration::from_millis(100)).await;

      spinner.finish_with_message("Sync complete");
   }

   let results = perform_search(
      &query,
      &index_root,
      request_path,
      &resolved_store_id,
      max,
      per_file,
      !options.no_rerank,
      options.mode,
   )
   .await?;

   if results.is_empty() {
      if options.json {
         println!("{}", serde_json::to_string(&JsonOutput { results: vec![] })?);
      } else {
         println!("No results found for '{query}'");
         if !options.sync {
            println!("\nTip: Use --sync to re-index before searching");
         }
      }
      return Ok(());
   }

   if options.json {
      println!("{}", serde_json::to_string(&JsonOutput { results })?);
   } else {
      let snippet_mode = resolve_snippet_mode(options);
      let format_opts = FormatOptions {
         compact: options.compact,
         scores: options.scores,
         plain: options.plain,
         snippet_mode,
         mode: options.mode,
      };
      format_results(
         &results,
         &query,
         &index_root,
         request_path,
         format_opts,
         SearchStatus::Ready,
         None,
      );
   }

   Ok(())
}

/// Attempts to execute the search via a running daemon, returning None if
/// unavailable.
async fn try_daemon_search(
   query: &str,
   max: usize,
   per_file: usize,
   mode: SearchMode,
   rerank: bool,
   index_root: &Path,
   path: Option<&Path>,
   store_id: &str,
) -> Result<Option<DaemonSearchOutcome>> {
   let Ok(stream) = daemon::connect_matching_daemon(index_root, store_id).await else {
      return Ok(None);
   };

   match send_search_request(stream, query, max, per_file, mode, rerank, path).await {
      Ok(outcome) => Ok(Some(outcome)),
      Err(e) => {
         tracing::debug!("daemon search failed; falling back to in-process search: {}", e);
         Ok(None)
      },
   }
}

/// Sends a search request to a daemon over the given stream and returns
/// results.
async fn send_search_request(
   mut stream: usock::Stream,
   query: &str,
   max: usize,
   per_file: usize,
   mode: SearchMode,
   rerank: bool,
   path: Option<&Path>,
) -> Result<DaemonSearchOutcome> {
   let timeout =
      Duration::from_millis(config::get().worker_timeout_ms).min(Duration::from_secs(45));

   let request = Request::Search {
      query: query.to_string(),
      limit: max,
      per_file,
      mode,
      path: path.map(Path::to_path_buf),
      rerank,
   };

   let mut buffer = ipc::SocketBuffer::new();
   let response: Response = match time::timeout(timeout, async {
      buffer.send(&mut stream, &request).await?;
      buffer.recv(&mut stream).await
   })
   .await
   {
      Ok(Ok(r)) => r,
      Ok(Err(e)) => return Err(e),
      Err(_) => {
         return Err(
            Error::Server {
               op:     "search",
               reason: format!("timeout waiting for daemon response ({}s)", timeout.as_secs()),
            }
            .into(),
         );
      },
   };

   match response {
      Response::Search(search_response) => {
         let status = search_response.status;
         let progress = search_response.progress;

         let mut results: Vec<SearchResult> = search_response
            .results
            .into_iter()
            .map(|r| SearchResult {
               path:       r.path,
               score:      r.score,
               match_pct:  None,
               content:    r.content.into_string(),
               chunk_type: r.chunk_type.map(|ct| ct.as_lowercase_str().to_string()),
               start_line: Some(r.start_line as usize),
               end_line:   Some((r.start_line + r.num_lines) as usize),
               is_anchor:  r.is_anchor,
            })
            .collect();

         apply_match_pcts(&mut results);
         Ok(DaemonSearchOutcome { results, status, progress })
      },
      Response::Error { message } => Err(Error::Server { op: "search", reason: message }),
      _ => Err(Error::UnexpectedResponse("search")),
   }
}

/// Performs a search directly without using a daemon, loading the search engine
/// in-process.
async fn perform_search(
   query: &str,
   index_root: &Path,
   path: Option<&Path>,
   store_id: &str,
   max: usize,
   per_file: usize,
   rerank: bool,
   mode: SearchMode,
) -> Result<Vec<SearchResult>> {
   let store = Arc::new(LanceStore::new()?);
   let embedder = Arc::new(EmbedWorker::new()?);

   let file_system = LocalFileSystem::new();
   let chunker = Chunker::default();
   let sync_engine = SyncEngine::new(file_system, chunker, embedder.clone(), store.clone());

   sync_engine
      .initial_sync(store_id, index_root, false, &mut ())
      .await?;

   let engine = SearchEngine::new(store, embedder);
   let include_anchors = config::get().fast_mode;
   let response = engine
      .search_with_mode(store_id, query, max, per_file, path, rerank, include_anchors, mode)
      .await?;

   let root_str = index_root.to_string_lossy().into_owned();

   let mut results: Vec<SearchResult> = response
      .results
      .into_iter()
      .map(|r| {
         let rel_path = r
            .path
            .strip_prefix(&root_str)
            .unwrap_or(&r.path)
            .to_string_lossy()
            .trim_start_matches('/')
            .into();

         SearchResult {
            path:       rel_path,
            score:      r.score,
            match_pct:  None,
            content:    r.content.into_string(),
            chunk_type: r.chunk_type.map(|ct| ct.as_lowercase_str().to_string()),
            start_line: Some(r.start_line as usize),
            end_line:   Some((r.start_line + r.num_lines) as usize),
            is_anchor:  r.is_anchor,
         }
      })
      .collect();

   apply_match_pcts(&mut results);
   Ok(results)
}

/// Formats and prints search results in human-readable form.
fn format_results(
   results: &[SearchResult],
   query: &str,
   root: &Path,
   scope: Option<&Path>,
   options: FormatOptions,
   status: SearchStatus,
   progress: Option<u8>,
) {
   const DEFAULT_PREVIEW_LINES: usize = 12;
   const SHORT_PREVIEW_LINES: usize = 8;
   const LONG_PREVIEW_LINES: usize = 24;

   if options.compact {
      let mut seen = std::collections::HashSet::<PathBuf>::new();
      for result in results {
         if seen.insert(result.path.clone()) {
            println!("{}", result.path.display());
         }
      }
      return;
   }

   if options.plain {
      println!("\nSearch results for: {query}");
      println!("Root: {}", root.display());
      if let Some(scope) = scope {
         let scope = scope.strip_prefix(root).unwrap_or(scope);
         println!("Scope: {}", scope.display());
      }
      if status == SearchStatus::Indexing {
         println!(
            "Status: indexing {}%",
            progress.map_or_else(|| "?".to_string(), |p| p.to_string())
         );
      }
      println!();
   } else {
      println!("\n{}", style(format!("Search results for: {query}")).bold());
      println!("{}", style(format!("Root: {}", root.display())).dim());
      if let Some(scope) = scope {
         let scope = scope.strip_prefix(root).unwrap_or(scope);
         println!("{}", style(format!("Scope: {}", scope.display())).dim());
      }
      if status == SearchStatus::Indexing {
         let p = progress.map_or_else(|| "?".to_string(), |p| p.to_string());
         println!("{}", style(format!("Status: indexing {p}%")).dim());
      }
      println!();
   }

   let include_anchors = config::get().fast_mode;
   let display_results: Vec<_> = results
      .iter()
      .filter(|r| include_anchors || !r.is_anchor.unwrap_or(false))
      .collect();

   let print_one = |idx: usize, result: &&SearchResult| {
      let start_line = result.start_line.unwrap_or(1);
      let lines: Vec<&str> = result.content.lines().collect();
      let total_lines = lines.len();
      let max_lines = match options.snippet_mode {
         SnippetMode::Full => usize::MAX,
         SnippetMode::Long => LONG_PREVIEW_LINES,
         SnippetMode::Short => SHORT_PREVIEW_LINES,
         SnippetMode::None => 0,
         SnippetMode::Default => DEFAULT_PREVIEW_LINES,
      };
      let show_all = max_lines == usize::MAX || total_lines <= max_lines;
      let display_lines = if show_all {
         total_lines
      } else {
         max_lines.min(total_lines)
      };
      let line_num_width = format!("{}", start_line + display_lines).len();

      if options.plain {
         print!("{idx}) {}:{}", result.path.display(), start_line);

         if options.scores {
            if let Some(match_pct) = result.match_pct {
               print!(" (match: {match_pct}%, score: {:.3})", result.score);
            } else {
               print!(" (score: {:.3})", result.score);
            }
         }

         println!();

         if display_lines > 0 {
            for (j, line) in lines.iter().take(display_lines).enumerate() {
               let line_num = start_line + j;
               println!("{line_num:>line_num_width$} | {line}");
            }
         }

         if !show_all && display_lines > 0 {
            let remaining = total_lines - display_lines;
            println!("{:>width$} | ... (+{} more lines)", "", remaining, width = line_num_width);
         }
      } else {
         print!("{}", style(format!("{idx}) ")).bold().cyan());
         print!("{}:{}", style(result.path.display()).green(), start_line);

         if options.scores {
            if let Some(match_pct) = result.match_pct {
               print!(
                  " {}",
                  style(format!("(match: {match_pct}%, score: {:.3})", result.score)).dim()
               );
            } else {
               print!(" {}", style(format!("(score: {:.3})", result.score)).dim());
            }
         }

         println!();

         if display_lines > 0 {
            for (j, line) in lines.iter().take(display_lines).enumerate() {
               let line_num = start_line + j;
               println!(
                  "{:>width$} {} {}",
                  style(line_num).dim(),
                  style("|").dim(),
                  line,
                  width = line_num_width
               );
            }
         }

         if !show_all && display_lines > 0 {
            let remaining = total_lines - display_lines;
            println!(
               "{:>width$} {} {}",
               "",
               style("|").dim(),
               style(format!("... (+{remaining} more lines)")).dim(),
               width = line_num_width
            );
         }
      }

      println!();
   };

   if options.mode == SearchMode::Balanced {
      for (i, result) in display_results.iter().enumerate() {
         print_one(i + 1, result);
      }
      return;
   }

   use crate::search::profile::{SearchBucket, bucket_for_path};
   let mut code = Vec::new();
   let mut docs = Vec::new();
   let mut graphs = Vec::new();
   for r in &display_results {
      match bucket_for_path(&r.path) {
         SearchBucket::Code => code.push(*r),
         SearchBucket::Docs => docs.push(*r),
         SearchBucket::Graph => graphs.push(*r),
      }
   }

   let sections = [("Code", code), ("Docs", docs), ("Graph", graphs)];

   let mut idx = 1usize;
   for (name, section_results) in sections {
      if section_results.is_empty() {
         continue;
      }

      if options.plain {
         println!("== {name} ==");
      } else {
         println!("{}", style(format!("== {name} ==")).bold());
      }

      for result in section_results {
         print_one(idx, &result);
         idx += 1;
      }
   }
}

fn format_empty_results(
   query: &str,
   root: &Path,
   scope: Option<&Path>,
   status: SearchStatus,
   progress: Option<u8>,
   options: FormatOptions,
) {
   // Keep the same header styling as normal results, but include a clear empty
   // state so indexing-from-scratch doesn't look like a crash.
   if options.plain {
      println!("\nSearch results for: {query}");
      println!("Root: {}", root.display());
      if let Some(scope) = scope {
         let scope = scope.strip_prefix(root).unwrap_or(scope);
         println!("Scope: {}", scope.display());
      }
      if status == SearchStatus::Indexing {
         println!(
            "Status: indexing {}%",
            progress.map_or_else(|| "?".to_string(), |p| p.to_string())
         );
      }
      println!();
      println!("No results found for '{query}'");
      if status == SearchStatus::Indexing {
         println!("Tip: Index is still building; try again in a bit.");
      } else {
         println!("Tip: Use --sync to re-index before searching.");
      }
   } else {
      println!("\n{}", style(format!("Search results for: {query}")).bold());
      println!("{}", style(format!("Root: {}", root.display())).dim());
      if let Some(scope) = scope {
         let scope = scope.strip_prefix(root).unwrap_or(scope);
         println!("{}", style(format!("Scope: {}", scope.display())).dim());
      }
      if status == SearchStatus::Indexing {
         let p = progress.map_or_else(|| "?".to_string(), |p| p.to_string());
         println!("{}", style(format!("Status: indexing {p}%")).dim());
      }
      println!();
      println!("{}", style(format!("No results found for '{query}'")).yellow());
      if status == SearchStatus::Indexing {
         println!("{}", style("Tip: Index is still building; try again in a bit.").dim());
      } else {
         println!("{}", style("Tip: Use --sync to re-index before searching.").dim());
      }
   }
}

fn apply_match_pcts(results: &mut [SearchResult]) {
   if results.is_empty() {
      return;
   }

   let scores: Vec<f32> = results.iter().map(|r| r.score).collect();
   let pcts = crate::util::compute_match_pcts(&scores);
   for (r, pct) in results.iter_mut().zip(pcts) {
      r.match_pct = pct;
   }
}

fn resolve_snippet_mode(options: SearchOptions) -> SnippetMode {
   if options.content {
      return SnippetMode::Full;
   }
   if options.long_snippet {
      return SnippetMode::Long;
   }
   if options.short_snippet {
      return SnippetMode::Short;
   }
   if options.no_snippet {
      return SnippetMode::None;
   }
   SnippetMode::Default
}
