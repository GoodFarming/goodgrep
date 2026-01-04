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
   file::{LocalFileSystem, normalize_relative},
   git, identity,
   ipc::{self, Request, Response},
   meta::MetaStore,
   snapshot::SnapshotManager,
   search::SearchEngine,
   store::LanceStore,
   sync::{SyncEngine, SyncOptions},
   types::{SearchLimitHit, SearchMode, SearchStatus, SearchTimings, SearchWarning},
   usock,
   util::sanitize_output,
};

/// A single search result with metadata and content.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SearchResult {
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
pub(crate) struct SearchJsonOutput {
   #[serde(flatten)]
   meta:    SearchMeta,
   results: Vec<SearchResult>,
   #[serde(skip_serializing_if = "Option::is_none")]
   explain: Option<SearchExplain>,
}

#[derive(Debug)]
pub(crate) struct SearchOutcome {
   results:    Vec<SearchResult>,
   status:     SearchStatus,
   progress:   Option<u8>,
   timings_ms: Option<SearchTimings>,
   limits_hit: Vec<SearchLimitHit>,
   warnings:   Vec<SearchWarning>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchErrorJson {
   error: SearchErrorPayload,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchErrorPayload {
   code:           String,
   message:        String,
   #[serde(skip_serializing_if = "Option::is_none")]
   retry_after_ms: Option<u64>,
   #[serde(skip_serializing_if = "Option::is_none")]
   snapshot_id:    Option<String>,
   #[serde(skip_serializing_if = "Option::is_none")]
   request_id:     Option<String>,
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
   pub explain:       bool,
   pub no_rerank:     bool,
   pub allow_degraded: bool,
   pub plain:         bool,
   pub mode:          SearchMode,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SnippetMode {
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

#[derive(Debug, Serialize, Clone)]
pub(crate) struct SearchMeta {
   schema_version: u32,
   request_id: String,
   store_id: String,
   config_fingerprint: String,
   ignore_fingerprint: String,
   query_fingerprint: String,
   embed_config_fingerprint: String,
   snapshot_id: Option<String>,
   degraded: bool,
   git: Option<GitExplain>,
   mode: SearchMode,
   limits: ExplainLimits,
   #[serde(skip_serializing_if = "Vec::is_empty")]
   limits_hit: Vec<SearchLimitHit>,
   #[serde(skip_serializing_if = "Vec::is_empty")]
   warnings: Vec<SearchWarning>,
   #[serde(skip_serializing_if = "Option::is_none")]
   timings_ms: Option<JsonTimings>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct JsonTimings {
   admission:     u64,
   snapshot_read: u64,
   retrieve:      u64,
   rank:          u64,
   format:        u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SearchExplain {
   #[serde(flatten)]
   meta:          SearchMeta,
   candidate_mix: CandidateMix,
}

#[derive(Debug, Serialize, Clone)]
struct GitExplain {
   head_sha:           Option<String>,
   dirty:              Option<bool>,
   untracked_included: bool,
}

#[derive(Debug, Serialize, Clone)]
struct ExplainLimits {
   max_results: usize,
   per_file: usize,
   snippet: String,
   max_candidates: usize,
   max_total_snippet_bytes: usize,
   max_snippet_bytes_per_result: usize,
   max_open_segments_per_query: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct CandidateMix {
   total:   usize,
   code:    usize,
   docs:    usize,
   graph:   usize,
   anchors: usize,
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
   let request_id = uuid::Uuid::new_v4().to_string();
   match execute_inner(query, path, max, per_file, options, eval_store, store_id, &request_id).await
   {
      Ok(()) => Ok(()),
      Err(err) => {
         if options.json {
            emit_json_error(&err, &request_id)?;
            return Err(Error::Reported {
               message:   "json error emitted".to_string(),
               exit_code: err.exit_code(),
            });
         }
         Err(err)
      },
   }
}

async fn execute_inner(
   query: String,
   path: Option<PathBuf>,
   max: usize,
   per_file: usize,
   options: SearchOptions,
   eval_store: bool,
   store_id: Option<String>,
   request_id: &str,
) -> Result<()> {
   let cwd = std::env::current_dir()?.canonicalize()?;
   // Default to searching "here" (current directory) while still using the
   // repo-root store when in a git repo.
   let filter_path = path.unwrap_or_else(|| cwd.clone()).canonicalize()?;
   let index_identity = identity::resolve_index_identity(&filter_path)?;
   let index_root = index_identity.canonical_root.clone();

   let resolved_store_id = match store_id {
      Some(s) => {
         if eval_store && !s.ends_with("-eval") {
            format!("{s}-eval")
         } else {
            s
         }
      },
      None => {
         let base = index_identity.store_id.clone();
         if eval_store {
            format!("{base}-eval")
         } else {
            base
         }
      },
   };

   let cfg = config::get();
   let capped_max = max.min(cfg.max_query_results).max(1);
   let capped_per_file = per_file.min(cfg.max_query_per_file).max(1);

   let scope_rel = if filter_path != index_root {
      let rel = filter_path
         .strip_prefix(&index_root)
         .ok()
         .and_then(normalize_relative)
         .unwrap_or_else(|| PathBuf::from(crate::file::normalize_path(&filter_path)));
      Some(rel)
   } else {
      None
   };

   if options.dry_run {
      if options.json {
         let snippet_mode = resolve_snippet_mode(options);
         let outcome = SearchOutcome {
            results:    vec![],
            status:     SearchStatus::Ready,
            progress:   None,
            timings_ms: None,
            limits_hit: vec![],
            warnings:   vec![],
         };
         let meta = build_meta(
            &query,
            &index_identity,
            &resolved_store_id,
            scope_rel.as_deref(),
            snippet_mode,
            capped_max,
            capped_per_file,
            !options.no_rerank,
            options.mode,
            &request_id,
            &outcome,
         )?;
         let explain = if options.explain {
            Some(build_explain(&meta, &outcome))
         } else {
            None
         };
         println!(
            "{}",
            serde_json::to_string(&SearchJsonOutput { meta, results: vec![], explain })?
         );
      } else {
         println!("Dry run: would search for '{query}' in {}", index_root.display());
         if let Some(scope) = &scope_rel {
            println!("Scope: {}", scope.display());
         }
         println!("Store ID: {resolved_store_id}");
         println!("Max results: {capped_max}");
      }
      return Ok(());
   }

   let request_path = scope_rel.as_deref();

   if let Some(outcome) = try_daemon_search(
      &query,
      capped_max,
      capped_per_file,
      options.mode,
      !options.no_rerank,
      &index_root,
      request_path,
      &resolved_store_id,
   )
   .await?
   {
      let snippet_mode = resolve_snippet_mode(options);
      let meta = if options.json || options.explain {
         Some(build_meta(
            &query,
            &index_identity,
            &resolved_store_id,
            request_path,
            snippet_mode,
            capped_max,
            capped_per_file,
            !options.no_rerank,
            options.mode,
            &request_id,
            &outcome,
         )?)
      } else {
         None
      };
      let explain = if options.explain {
         meta.as_ref().map(|meta| build_explain(meta, &outcome))
      } else {
         None
      };

      if options.json {
         let meta = meta.expect("meta required for json output");
         println!(
            "{}",
            serde_json::to_string(&SearchJsonOutput { meta, results: outcome.results, explain })?
         );
      } else {
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
         if let Some(explain) = explain {
            print_explain(&explain, options.plain);
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

   let outcome = perform_search(
      &query,
      &index_root,
      request_path,
      &resolved_store_id,
      capped_max,
      capped_per_file,
      !options.no_rerank,
      options.mode,
      options.allow_degraded,
   )
   .await?;

   let snippet_mode = resolve_snippet_mode(options);
   let meta = if options.json || options.explain {
      Some(build_meta(
         &query,
         &index_identity,
         &resolved_store_id,
         request_path,
         snippet_mode,
         capped_max,
         capped_per_file,
         !options.no_rerank,
         options.mode,
         &request_id,
         &outcome,
      )?)
   } else {
      None
   };
   let explain = if options.explain {
      meta.as_ref().map(|meta| build_explain(meta, &outcome))
   } else {
      None
   };

   if outcome.results.is_empty() {
      if options.json {
         let meta = meta.expect("meta required for json output");
         println!(
            "{}",
            serde_json::to_string(&SearchJsonOutput { meta, results: vec![], explain })?
         );
      } else {
         println!("No results found for '{query}'");
         if !options.sync {
            println!("\nTip: Use --sync to re-index before searching");
         }
         if let Some(explain) = explain {
            print_explain(&explain, options.plain);
         }
      }
      return Ok(());
   }

   if options.json {
      let meta = meta.expect("meta required for json output");
      println!(
         "{}",
         serde_json::to_string(&SearchJsonOutput { meta, results: outcome.results, explain })?
      );
   } else {
      let format_opts = FormatOptions {
         compact: options.compact,
         scores: options.scores,
         plain: options.plain,
         snippet_mode,
         mode: options.mode,
      };
      format_results(
         &outcome.results,
         &query,
         &index_root,
         request_path,
         format_opts,
         outcome.status,
         outcome.progress,
      );
      if let Some(explain) = explain {
         print_explain(&explain, options.plain);
      }
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
) -> Result<Option<SearchOutcome>> {
   let Ok(stream) = daemon::connect_matching_daemon(index_root, store_id).await else {
      return Ok(None);
   };

   match send_search_request(stream, query, max, per_file, mode, rerank, path, index_root).await {
      Ok(outcome) => Ok(Some(outcome)),
      Err(e) => {
         tracing::debug!("daemon search failed; falling back to in-process search: {}", e);
         Ok(None)
      },
   }
}

/// Sends a search request to a daemon over the given stream and returns
/// results.
pub(crate) async fn send_search_request(
   mut stream: usock::Stream,
   query: &str,
   max: usize,
   per_file: usize,
   mode: SearchMode,
   rerank: bool,
   path: Option<&Path>,
   index_root: &Path,
) -> Result<SearchOutcome> {
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
      buffer
         .recv_with_limit(&mut stream, config::get().max_response_bytes)
         .await
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
         let timings_ms = search_response.timings_ms;

         let mut results: Vec<SearchResult> = search_response
            .results
            .into_iter()
            .map(|r| SearchResult {
               path:       PathBuf::from(sanitize_output(&r.path.to_string_lossy())),
               score:      r.score,
               match_pct:  None,
               content:    sanitize_output(&r.content.into_string()),
               chunk_type: r.chunk_type.map(|ct| ct.as_lowercase_str().to_string()),
               start_line: Some(r.start_line as usize),
               end_line:   Some((r.start_line + r.num_lines) as usize),
               is_anchor:  r.is_anchor,
            })
            .collect();

         apply_match_pcts(&mut results);
         let limits_hit = sanitize_limits(search_response.limits_hit, index_root);
         let warnings = sanitize_warnings(search_response.warnings, index_root);
         Ok(SearchOutcome { results, status, progress, timings_ms, limits_hit, warnings })
      },
      Response::Error { code, message } => {
         Err(Error::Server { op: "search", reason: format!("{code}: {message}") })
      },
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
   allow_degraded: bool,
) -> Result<SearchOutcome> {
   let store = Arc::new(LanceStore::new()?);
   let embedder = Arc::new(EmbedWorker::new()?);

   let file_system = LocalFileSystem::new();
   let chunker = Chunker::default();
   let sync_engine = SyncEngine::new(file_system, chunker, embedder.clone(), store.clone());

   sync_engine
      .initial_sync_with_options(
         store_id,
         index_root,
         None,
         false,
         SyncOptions { allow_degraded, ..SyncOptions::default() },
         &mut (),
      )
      .await?;

   let fingerprints = identity::compute_fingerprints(index_root)?;
   let snapshot_manager = SnapshotManager::new(
      store.clone(),
      store_id.to_string(),
      fingerprints.config_fingerprint,
      fingerprints.ignore_fingerprint,
   );
   let snapshot_start = std::time::Instant::now();
   let snapshot_view = snapshot_manager.open_snapshot_view().await?;
   let snapshot_read_ms = snapshot_start.elapsed().as_millis() as u64;

   let engine = SearchEngine::new(store, embedder);
   let include_anchors = config::get().fast_mode;
   let response = engine
      .search_with_mode(
         &snapshot_view,
         store_id,
         query,
         max,
         per_file,
         path,
         rerank,
         include_anchors,
         mode,
      )
      .await?;

   let mut response = response;
   if let Some(ref mut timings) = response.timings_ms {
      timings.snapshot_read_ms = snapshot_read_ms;
   } else {
      response.timings_ms = Some(SearchTimings {
         snapshot_read_ms,
         ..SearchTimings::default()
      });
   }

   let root_str = index_root.to_string_lossy().into_owned();

   let mut results: Vec<SearchResult> = response
      .results
      .into_iter()
      .map(|r| {
         let rel_path_str = r
            .path
            .strip_prefix(&root_str)
            .unwrap_or(&r.path)
            .to_string_lossy()
            .trim_start_matches('/')
            .to_string();

         SearchResult {
            path:       PathBuf::from(sanitize_output(&rel_path_str)),
            score:      r.score,
            match_pct:  None,
            content:    sanitize_output(&r.content.into_string()),
            chunk_type: r.chunk_type.map(|ct| ct.as_lowercase_str().to_string()),
            start_line: Some(r.start_line as usize),
            end_line:   Some((r.start_line + r.num_lines) as usize),
            is_anchor:  r.is_anchor,
         }
      })
      .collect();

   apply_match_pcts(&mut results);
   let limits_hit = sanitize_limits(response.limits_hit, index_root);
   let warnings = sanitize_warnings(response.warnings, index_root);
   Ok(SearchOutcome {
      results,
      status: response.status,
      progress: response.progress,
      timings_ms: response.timings_ms,
      limits_hit,
      warnings,
   })
}

fn sanitize_limits(limits: Vec<SearchLimitHit>, root: &Path) -> Vec<SearchLimitHit> {
   limits
      .into_iter()
      .map(|mut hit| {
         hit.code = sanitize_output(&hit.code);
         if let Some(path_key) = hit.path_key.take() {
            let path = PathBuf::from(path_key);
            let rel_path = path.strip_prefix(root).map(PathBuf::from).unwrap_or(path);
            hit.path_key = Some(sanitize_output(&rel_path.to_string_lossy()));
         }
         hit
      })
      .collect()
}

fn sanitize_warnings(warnings: Vec<SearchWarning>, root: &Path) -> Vec<SearchWarning> {
   warnings
      .into_iter()
      .map(|mut warning| {
         warning.code = sanitize_output(&warning.code);
         warning.message = sanitize_output(&warning.message);
         if let Some(path_key) = warning.path_key.take() {
            let path = PathBuf::from(path_key);
            let rel_path = path.strip_prefix(root).map(PathBuf::from).unwrap_or(path);
            warning.path_key = Some(sanitize_output(&rel_path.to_string_lossy()));
         }
         warning
      })
      .collect()
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
         let scope = if scope.is_absolute() {
            scope.strip_prefix(root).unwrap_or(scope)
         } else {
            scope
         };
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
         let scope = if scope.is_absolute() {
            scope.strip_prefix(root).unwrap_or(scope)
         } else {
            scope
         };
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
         let scope = if scope.is_absolute() {
            scope.strip_prefix(root).unwrap_or(scope)
         } else {
            scope
         };
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
         let scope = if scope.is_absolute() {
            scope.strip_prefix(root).unwrap_or(scope)
         } else {
            scope
         };
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

fn snippet_mode_label(mode: SnippetMode) -> &'static str {
   match mode {
      SnippetMode::Full => "full",
      SnippetMode::Long => "long",
      SnippetMode::Short => "short",
      SnippetMode::None => "none",
      SnippetMode::Default => "default",
   }
}

const SEARCH_SCHEMA_VERSION: u32 = 1;

pub(crate) fn build_meta(
   query: &str,
   index_identity: &identity::IndexIdentity,
   store_id: &str,
   scope: Option<&Path>,
   snippet_mode: SnippetMode,
   max_results: usize,
   per_file: usize,
   rerank: bool,
   mode: SearchMode,
   request_id: &str,
   outcome: &SearchOutcome,
) -> Result<SearchMeta> {
   let cfg = config::get();
   let query_fingerprint =
      identity::compute_query_fingerprint(query, identity::QueryFingerprintOptions {
         mode,
         per_file,
         max_results,
         rerank,
         scope,
         snippet: snippet_mode_label(snippet_mode),
      })?;
   let embed_config_fingerprint = identity::compute_embed_config_fingerprint(cfg)?;
   let meta_store = MetaStore::load(store_id).ok();
   let snapshot_id = meta_store
      .as_ref()
      .and_then(|meta| meta.snapshot_id().map(|s| s.to_string()));
   let degraded = meta_store
      .as_ref()
      .map(|meta| meta.snapshot_degraded())
      .unwrap_or(false);

   let head_sha = git::get_head_sha(&index_identity.canonical_root);
   let dirty = git::is_dirty(&index_identity.canonical_root);
   let git_info = if head_sha.is_some() || dirty.is_some() {
      Some(GitExplain { head_sha, dirty, untracked_included: true })
   } else {
      None
   };

   Ok(SearchMeta {
      schema_version: SEARCH_SCHEMA_VERSION,
      request_id: request_id.to_string(),
      store_id: store_id.to_string(),
      config_fingerprint: index_identity.config_fingerprint.clone(),
      ignore_fingerprint: index_identity.ignore_fingerprint.clone(),
      query_fingerprint,
      embed_config_fingerprint,
      snapshot_id,
      degraded,
      git: git_info,
      mode,
      limits: ExplainLimits {
         max_results,
         per_file,
         snippet: snippet_mode_label(snippet_mode).to_string(),
         max_candidates: cfg.effective_max_candidates(),
         max_total_snippet_bytes: cfg.effective_max_total_snippet_bytes(),
         max_snippet_bytes_per_result: cfg.effective_max_snippet_bytes_per_result(),
         max_open_segments_per_query: cfg.effective_max_open_segments_per_query(),
      },
      limits_hit: outcome.limits_hit.clone(),
      warnings: outcome.warnings.clone(),
      timings_ms: outcome.timings_ms.map(|timings| JsonTimings {
         admission:     timings.admission_ms,
         snapshot_read: timings.snapshot_read_ms,
         retrieve:      timings.retrieve_ms,
         rank:          timings.rank_ms,
         format:        timings.format_ms,
      }),
   })
}

pub(crate) fn build_explain(meta: &SearchMeta, outcome: &SearchOutcome) -> SearchExplain {
   SearchExplain { meta: meta.clone(), candidate_mix: candidate_mix(&outcome.results) }
}

pub(crate) fn build_json_output(
   meta: SearchMeta,
   outcome: SearchOutcome,
   explain: Option<SearchExplain>,
) -> SearchJsonOutput {
   SearchJsonOutput { meta, results: outcome.results, explain }
}

fn candidate_mix(results: &[SearchResult]) -> CandidateMix {
   use crate::search::profile::{SearchBucket, bucket_for_path};

   let mut mix =
      CandidateMix { total: results.len(), code: 0, docs: 0, graph: 0, anchors: 0 };

   for result in results {
      if result.is_anchor.unwrap_or(false) {
         mix.anchors += 1;
      }
      match bucket_for_path(&result.path) {
         SearchBucket::Code => mix.code += 1,
         SearchBucket::Docs => mix.docs += 1,
         SearchBucket::Graph => mix.graph += 1,
      }
   }

   mix
}

fn classify_error(err: &Error) -> (String, String) {
   if let Error::Server { reason, .. } = err {
      if let Some((code, message)) = reason.split_once(':') {
         let code = code.trim().to_lowercase();
         let message = message.trim().to_string();
         if matches!(
            code.as_str(),
            "busy" | "timeout" | "cancelled" | "invalid_request" | "internal" | "incompatible"
         ) {
            return (code, message);
         }
      }
   }

   let message = err.to_string();
   let lower = message.to_lowercase();
   let code = if lower.contains("busy") {
      "busy"
   } else if lower.contains("timeout") {
      "timeout"
   } else if lower.contains("cancel") {
      "cancelled"
   } else if lower.contains("invalid") {
      "invalid_request"
   } else if lower.contains("incompatible") {
      "incompatible"
   } else {
      "internal"
   };

   (code.to_string(), message)
}

pub(crate) fn build_json_error(err: &Error, request_id: &str) -> SearchErrorJson {
   let (code, message) = classify_error(err);
   SearchErrorJson {
      error: SearchErrorPayload {
         code,
         message,
         retry_after_ms: None,
         snapshot_id: None,
         request_id: Some(request_id.to_string()),
      },
   }
}

fn emit_json_error(err: &Error, request_id: &str) -> Result<()> {
   let payload = build_json_error(err, request_id);
   println!("{}", serde_json::to_string(&payload)?);
   Ok(())
}

fn print_explain(explain: &SearchExplain, plain: bool) {
   if plain {
      println!("\nExplain:");
   } else {
      println!("\n{}", style("Explain").bold());
   }

   let meta = &explain.meta;
   println!("  request_id: {}", meta.request_id);
   println!("  store_id: {}", meta.store_id);
   println!("  config_fingerprint: {}", meta.config_fingerprint);
   println!("  ignore_fingerprint: {}", meta.ignore_fingerprint);
   println!("  query_fingerprint: {}", meta.query_fingerprint);
   println!("  embed_config_fingerprint: {}", meta.embed_config_fingerprint);
   if let Some(snapshot_id) = &meta.snapshot_id {
      println!("  snapshot_id: {}", snapshot_id);
   }
   if let Some(git) = &meta.git {
      if let Some(head_sha) = &git.head_sha {
         println!("  git_head: {}", head_sha);
      }
      if let Some(dirty) = git.dirty {
         println!("  git_dirty: {}", dirty);
      }
      println!("  untracked_included: {}", git.untracked_included);
   }

   println!(
      "  limits: max_results={}, per_file={}, snippet={}, max_candidates={}, \
       max_total_snippet_bytes={}, max_snippet_bytes_per_result={}, max_open_segments_per_query={}",
      meta.limits.max_results,
      meta.limits.per_file,
      meta.limits.snippet,
      meta.limits.max_candidates,
      meta.limits.max_total_snippet_bytes,
      meta.limits.max_snippet_bytes_per_result,
      meta.limits.max_open_segments_per_query
   );

   println!(
      "  candidate_mix: total={}, code={}, docs={}, graph={}, anchors={}",
      explain.candidate_mix.total,
      explain.candidate_mix.code,
      explain.candidate_mix.docs,
      explain.candidate_mix.graph,
      explain.candidate_mix.anchors
   );

   if let Some(timings) = &meta.timings_ms {
      println!(
         "  timings_ms: admission={}, snapshot_read={}, retrieve={}, rank={}, format={}",
         timings.admission, timings.snapshot_read, timings.retrieve, timings.rank, timings.format
      );
   }

   if !meta.limits_hit.is_empty() {
      println!("  limits_hit:");
      for hit in &meta.limits_hit {
         if let Some(path_key) = &hit.path_key {
            println!(
               "    - {} (limit={}, observed={:?}, path={})",
               hit.code, hit.limit, hit.observed, path_key
            );
         } else {
            println!("    - {} (limit={}, observed={:?})", hit.code, hit.limit, hit.observed);
         }
      }
   }

   if !meta.warnings.is_empty() {
      println!("  warnings:");
      for warning in &meta.warnings {
         if let Some(path_key) = &warning.path_key {
            println!("    - {}: {} ({})", warning.code, warning.message, path_key);
         } else {
            println!("    - {}: {}", warning.code, warning.message);
         }
      }
   }
}
