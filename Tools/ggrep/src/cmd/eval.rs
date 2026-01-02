//! Search quality evaluation harness.
//!
//! Runs a suite of natural-language queries against an indexed repo and writes
//! a JSON report with hit-rates + MRR for tuning recall/embedding behavior.

use std::{
   collections::{BTreeMap, HashSet},
   ffi::OsStr,
   io,
   path::{Path, PathBuf},
   sync::Arc,
   time::Instant,
};

use chrono::Utc;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{
   Result,
   chunker::Chunker,
   config,
   embed::worker::EmbedWorker,
   file::LocalFileSystem,
   git,
   search::{SearchEngine, profile::bucket_for_path},
   store::{LanceStore, Store},
   sync::{SyncEngine, SyncResult},
   types::{ChunkType, SearchMode},
   version,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvalSuite {
   #[serde(default = "default_suite_version")]
   version: u32,

   #[serde(default)]
   defaults: EvalDefaults,

   cases: Vec<EvalCase>,
}

fn default_suite_version() -> u32 {
   1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvalDefaults {
   #[serde(default = "default_k")]
   k: usize,

   #[serde(default = "default_per_file")]
   per_file: usize,

   #[serde(default = "default_rerank")]
   rerank: bool,

   #[serde(default)]
   mode: SearchMode,
}

impl Default for EvalDefaults {
   fn default() -> Self {
      Self {
         k:        default_k(),
         per_file: default_per_file(),
         rerank:   default_rerank(),
         mode:     SearchMode::Balanced,
      }
   }
}

fn default_k() -> usize {
   20
}

fn default_per_file() -> usize {
   3
}

fn default_rerank() -> bool {
   true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvalCase {
   id:    String,
   query: String,

   #[serde(default)]
   mode: Option<SearchMode>,

   #[serde(default)]
   k: Option<usize>,

   #[serde(default)]
   per_file: Option<usize>,

   #[serde(default)]
   rerank: Option<bool>,

   #[serde(default)]
   expect_any_path_contains: Vec<String>,

   #[serde(default)]
   expect_all_path_contains: Vec<String>,

   #[serde(default)]
   expect_any_path_regex: Vec<String>,

   #[serde(default)]
   expect_all_path_regex: Vec<String>,

   #[serde(default)]
   notes: Option<String>,
}

#[derive(Debug, Serialize)]
struct EvalReport {
   meta:    EvalMeta,
   sync:    EvalSync,
   summary: EvalSummary,
   cases:   Vec<EvalCaseReport>,
}

#[derive(Debug, Serialize)]
struct EvalMeta {
   started_at_utc: String,
   elapsed_ms:     u128,
   suite_path:     String,
   suite_version:  u32,
   store_id:       String,
   root:           String,
   ggrep_version:  String,
   config:         EvalConfig,
   overrides:      EvalOverrides,
}

#[derive(Debug, Serialize)]
struct EvalConfig {
   dense_model:        String,
   colbert_model:      String,
   dense_dim:          usize,
   colbert_dim:        usize,
   dense_max_length:   usize,
   colbert_max_length: usize,
   query_prefix:       String,
   doc_prefix:         String,
   disable_gpu:        bool,
   fast_mode:          bool,
   low_impact:         bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct EvalOverrides {
   k:               Option<usize>,
   per_file:        Option<usize>,
   mode:            Option<SearchMode>,
   no_rerank:       bool,
   include_anchors: bool,
   no_sync:         bool,
}

#[derive(Debug, Serialize)]
struct EvalSync {
   processed: usize,
   indexed:   usize,
   skipped:   usize,
   deleted:   usize,
}

#[derive(Debug, Serialize)]
struct EvalSummary {
   total:         usize,
   passed:        usize,
   pass_rate:     f32,
   mean_mrr:      f32,
   mean_hit_rank: Option<f32>,
   by_mode:       BTreeMap<SearchMode, EvalModeSummary>,
}

#[derive(Debug, Serialize)]
struct EvalModeSummary {
   total:     usize,
   passed:    usize,
   pass_rate: f32,
   mean_mrr:  f32,
}

#[derive(Debug, Serialize)]
struct EvalCaseReport {
   id:             String,
   query:          String,
   mode:           SearchMode,
   k:              usize,
   per_file:       usize,
   rerank:         bool,
   passed:         bool,
   first_hit_rank: Option<usize>,
   mrr:            f32,
   missing_all:    Vec<String>,
   notes:          Option<String>,
   hits:           Vec<EvalHit>,
}

#[derive(Debug, Serialize)]
struct EvalHit {
   rank:       usize,
   path:       String,
   bucket:     String,
   score:      f32,
   match_pct:  Option<u8>,
   start_line: u32,
   chunk_type: Option<String>,
   preview:    String,
}

#[derive(Debug)]
struct CaseMatchers {
   any_contains: Vec<String>,
   all_contains: Vec<String>,
   any_regex:    Vec<Regex>,
   all_regex:    Vec<Regex>,
}

pub async fn execute(
   suite_path: Option<PathBuf>,
   out_path: Option<PathBuf>,
   path: Option<PathBuf>,
   only: Vec<String>,
   no_sync: bool,
   k_override: Option<usize>,
   per_file_override: Option<usize>,
   mode_override: Option<String>,
   no_rerank: bool,
   include_anchors: bool,
   eval_store: bool,
   fail_under_pass_rate: Option<f32>,
   fail_under_mrr: Option<f32>,
   store_id: Option<String>,
) -> Result<()> {
   let root = std::env::current_dir()?;
   let search_path = path.unwrap_or_else(|| root.clone()).canonicalize()?;

   let resolved_store_id = match store_id {
      Some(s) => {
         if eval_store && !s.ends_with("-eval") {
            format!("{s}-eval")
         } else {
            s
         }
      },
      None => {
         let base = git::resolve_store_id(&search_path)?;
         if eval_store {
            format!("{base}-eval")
         } else {
            base
         }
      },
   };

   let resolved_suite_path = resolve_suite_path(&search_path, suite_path)?;
   let mut suite = load_suite(&resolved_suite_path)?;

   if !only.is_empty() {
      let only_set: HashSet<String> = only.into_iter().collect();
      let before = suite.cases.len();
      suite.cases.retain(|c| only_set.contains(&c.id));

      if suite.cases.is_empty() {
         return Err(
            io::Error::new(
               io::ErrorKind::InvalidInput,
               "no eval cases matched --only (check case ids)",
            )
            .into(),
         );
      }

      let after = suite.cases.len();
      println!("{}", style(format!("Filtering cases: {before} -> {after}")).dim());
   }

   if suite.cases.is_empty() {
      return Err(
         io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("eval suite has no cases: {}", resolved_suite_path.display()),
         )
         .into(),
      );
   }

   let mode_override = mode_override
      .as_deref()
      .map(parse_mode)
      .transpose()
      .map_err(|m| io::Error::new(io::ErrorKind::InvalidInput, m))?;

   let overrides = EvalOverrides {
      k: k_override,
      per_file: per_file_override,
      mode: mode_override,
      no_rerank,
      include_anchors,
      no_sync,
   };

   let resolved_out_path = resolve_out_path(out_path, &resolved_store_id);

   println!(
      "{}",
      style(format!(
         "ggrep eval: {} cases (store: {}, root: {})",
         suite.cases.len(),
         resolved_store_id,
         search_path.display()
      ))
      .bold()
   );

   let started_at = Utc::now();
   let run_t0 = Instant::now();

   let store = Arc::new(LanceStore::new()?);
   let embedder = Arc::new(EmbedWorker::new()?);

   let file_system = LocalFileSystem::new();
   let chunker = Chunker::default();
   let sync_engine = SyncEngine::new(file_system, chunker, embedder.clone(), store.clone());

   let sync_result = if overrides.no_sync {
      if store.is_empty(&resolved_store_id).await? {
         return Err(
            io::Error::new(
               io::ErrorKind::InvalidInput,
               "store appears empty; omit --no-sync (or run `ggrep index` first)",
            )
            .into(),
         );
      }
      SyncResult { processed: 0, indexed: 0, skipped: 0, deleted: 0 }
   } else {
      let mut pb = ProgressBar::new(0);
      pb.set_style(
         ProgressStyle::default_bar()
            .template("{spinner:.green} {msg} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%)")
            .unwrap()
            .progress_chars("█▓░"),
      );
      pb.set_prefix("Indexing:");
      pb.set_message("...");

      let sync_result = sync_engine
         .initial_sync(&resolved_store_id, &search_path, false, &mut pb)
         .await?;
      pb.finish_with_message(format!(
         "Index sync complete (indexed={}, skipped={}, deleted={})",
         sync_result.indexed, sync_result.skipped, sync_result.deleted
      ));
      sync_result
   };

   let engine = SearchEngine::new(store, embedder);

   let mut case_reports = Vec::with_capacity(suite.cases.len());
   for (idx, case) in suite.cases.iter().enumerate() {
      println!("{}", style(format!("[{}/{}] {}", idx + 1, suite.cases.len(), case.id)).cyan());
      let report =
         evaluate_case(&engine, &resolved_store_id, &search_path, &suite.defaults, case, overrides)
            .await?;
      println!(
         "  {}  first_hit={}  mrr={:.3}",
         if report.passed {
            style("PASS").green().bold()
         } else {
            style("FAIL").red().bold()
         },
         report
            .first_hit_rank
            .map_or_else(|| "-".to_string(), |r| r.to_string()),
         report.mrr
      );
      case_reports.push(report);
   }

   let elapsed_ms = run_t0.elapsed().as_millis();

   let cfg = config::get().clone();
   let report = EvalReport {
      meta:    EvalMeta {
         started_at_utc: started_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
         elapsed_ms,
         suite_path: resolved_suite_path.display().to_string(),
         suite_version: suite.version,
         store_id: resolved_store_id.clone(),
         root: search_path.display().to_string(),
         ggrep_version: version::version_string(),
         config: EvalConfig {
            dense_model:        cfg.dense_model,
            colbert_model:      cfg.colbert_model,
            dense_dim:          cfg.dense_dim,
            colbert_dim:        cfg.colbert_dim,
            dense_max_length:   cfg.dense_max_length,
            colbert_max_length: cfg.colbert_max_length,
            query_prefix:       cfg.query_prefix,
            doc_prefix:         cfg.doc_prefix,
            disable_gpu:        cfg.disable_gpu,
            fast_mode:          cfg.fast_mode,
            low_impact:         cfg.low_impact,
         },
         overrides,
      },
      sync:    EvalSync {
         processed: sync_result.processed,
         indexed:   sync_result.indexed,
         skipped:   sync_result.skipped,
         deleted:   sync_result.deleted,
      },
      summary: summarize(&case_reports),
      cases:   case_reports,
   };

   let json = serde_json::to_string_pretty(&report)?;
   if let Some(parent) = resolved_out_path.parent() {
      std::fs::create_dir_all(parent)?;
   }
   std::fs::write(&resolved_out_path, json)?;

   println!();
   println!(
      "{}",
      style(format!(
         "Summary: {}/{} passed ({:.1}%), mean_mrr={:.3}",
         report.summary.passed,
         report.summary.total,
         report.summary.pass_rate * 100.0,
         report.summary.mean_mrr
      ))
      .bold()
   );
   println!("Report: {}", style(resolved_out_path.display()).dim());
   println!("Build:  {}", style(version::GIT_HASH).dim());

   if let Some(threshold) = fail_under_pass_rate
      && report.summary.pass_rate < threshold
   {
      return Err(
         io::Error::new(
            io::ErrorKind::Other,
            format!(
               "pass_rate {:.3} is below threshold {:.3}",
               report.summary.pass_rate, threshold
            ),
         )
         .into(),
      );
   }

   if let Some(threshold) = fail_under_mrr
      && report.summary.mean_mrr < threshold
   {
      return Err(
         io::Error::new(
            io::ErrorKind::Other,
            format!("mean_mrr {:.3} is below threshold {:.3}", report.summary.mean_mrr, threshold),
         )
         .into(),
      );
   }

   Ok(())
}

fn resolve_suite_path(search_path: &Path, suite_path: Option<PathBuf>) -> Result<PathBuf> {
   if let Some(p) = suite_path {
      if p.exists() {
         return Ok(p);
      }
      return Err(
         io::Error::new(io::ErrorKind::NotFound, format!("eval suite not found: {}", p.display()))
            .into(),
      );
   }

   let cwd_default = PathBuf::from("Datasets/ggrep/eval_cases.toml");
   if cwd_default.exists() {
      return Ok(cwd_default);
   }

   if let Some(repo_root) = git::get_repo_root(search_path) {
      let repo_default = repo_root.join("Datasets/ggrep/eval_cases.toml");
      if repo_default.exists() {
         return Ok(repo_default);
      }
   }

   Err(io::Error::new(io::ErrorKind::NotFound, "eval suite not found (pass --cases <path>)").into())
}

fn resolve_out_path(out_path: Option<PathBuf>, store_id: &str) -> PathBuf {
   let mut out = out_path.unwrap_or_else(|| default_out_path(store_id));
   if out.is_dir() {
      let default_path = default_out_path(store_id);
      let file_name = default_path
         .file_name()
         .unwrap_or_else(|| OsStr::new("ggrep-eval.json"));
      out = out.join(file_name);
   }
   out
}

fn default_out_path(store_id: &str) -> PathBuf {
   let ts = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
   let file_name = format!("ggrep-eval-{store_id}-{ts}.json");
   std::env::temp_dir().join(file_name)
}

fn load_suite(path: &Path) -> Result<EvalSuite> {
   let content = std::fs::read_to_string(path)?;
   Ok(toml::from_str(&content)?)
}

fn parse_mode(mode: &str) -> std::result::Result<SearchMode, String> {
   match mode.trim().to_ascii_lowercase().as_str() {
      "balanced" => Ok(SearchMode::Balanced),
      "discovery" => Ok(SearchMode::Discovery),
      "implementation" | "impl" => Ok(SearchMode::Implementation),
      "planning" | "plan" => Ok(SearchMode::Planning),
      "debug" => Ok(SearchMode::Debug),
      other => Err(format!(
         "invalid mode '{other}' (expected: balanced|discovery|implementation|planning|debug)"
      )),
   }
}

async fn evaluate_case(
   engine: &SearchEngine,
   store_id: &str,
   root: &Path,
   defaults: &EvalDefaults,
   case: &EvalCase,
   overrides: EvalOverrides,
) -> Result<EvalCaseReport> {
   let mode = overrides
      .mode
      .unwrap_or_else(|| case.mode.unwrap_or(defaults.mode));
   let k = overrides.k.unwrap_or_else(|| case.k.unwrap_or(defaults.k));
   let per_file = overrides
      .per_file
      .unwrap_or_else(|| case.per_file.unwrap_or(defaults.per_file));
   let rerank = if overrides.no_rerank {
      false
   } else {
      case.rerank.unwrap_or(defaults.rerank)
   };

   let matchers = build_matchers(case)?;

   let include_anchors = overrides.include_anchors || config::get().fast_mode;

   let response = engine
      .search_with_mode(
         store_id,
         &case.query,
         k,
         per_file,
         Some(root),
         rerank,
         include_anchors,
         mode,
      )
      .await?;

   let mut hits: Vec<EvalHit> = response
      .results
      .into_iter()
      .filter(|r| include_anchors || !r.is_anchor.unwrap_or(false))
      .enumerate()
      .map(|(idx, r)| {
         let bucket = match bucket_for_path(&r.path) {
            crate::search::profile::SearchBucket::Code => "code",
            crate::search::profile::SearchBucket::Docs => "docs",
            crate::search::profile::SearchBucket::Graph => "graph",
         };
         EvalHit {
            rank:       idx + 1,
            path:       display_path(root, &r.path),
            bucket:     bucket.to_string(),
            score:      r.score,
            match_pct:  None,
            start_line: r.start_line,
            chunk_type: r
               .chunk_type
               .map(ChunkType::as_lowercase_str)
               .map(ToString::to_string),
            preview:    preview(r.content.as_str(), 220, 10),
         }
      })
      .collect();

   apply_match_pcts(&mut hits);

   let (passed, first_hit_rank, mrr, missing_all) = score_case(&hits, &matchers);

   Ok(EvalCaseReport {
      id: case.id.clone(),
      query: case.query.clone(),
      mode,
      k,
      per_file,
      rerank,
      passed,
      first_hit_rank,
      mrr,
      missing_all,
      notes: case.notes.clone(),
      hits,
   })
}

fn apply_match_pcts(hits: &mut [EvalHit]) {
   if hits.is_empty() {
      return;
   }

   let scores: Vec<f32> = hits.iter().map(|h| h.score).collect();
   let pcts = crate::util::compute_match_pcts(&scores);
   for (h, pct) in hits.iter_mut().zip(pcts) {
      h.match_pct = pct;
   }
}

fn build_matchers(case: &EvalCase) -> Result<CaseMatchers> {
   let any_contains: Vec<String> = case
      .expect_any_path_contains
      .iter()
      .map(|s| s.to_ascii_lowercase())
      .collect();
   let all_contains: Vec<String> = case
      .expect_all_path_contains
      .iter()
      .map(|s| s.to_ascii_lowercase())
      .collect();

   let any_regex = case
      .expect_any_path_regex
      .iter()
      .map(|p| Regex::new(p))
      .collect::<std::result::Result<Vec<_>, _>>()?;
   let all_regex = case
      .expect_all_path_regex
      .iter()
      .map(|p| Regex::new(p))
      .collect::<std::result::Result<Vec<_>, _>>()?;

   if any_contains.is_empty()
      && all_contains.is_empty()
      && any_regex.is_empty()
      && all_regex.is_empty()
   {
      return Err(
         io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("case '{}' has no expectations (expect_any_* / expect_all_*)", case.id),
         )
         .into(),
      );
   }

   Ok(CaseMatchers { any_contains, all_contains, any_regex, all_regex })
}

fn score_case(
   hits: &[EvalHit],
   matchers: &CaseMatchers,
) -> (bool, Option<usize>, f32, Vec<String>) {
   let first_hit_rank = first_hit_rank(hits, matchers);
   let mrr = first_hit_rank.map_or(0.0, |r| 1.0 / r as f32);

   let any_ok = if matchers.any_contains.is_empty() && matchers.any_regex.is_empty() {
      true
   } else {
      has_any_match(hits, &matchers.any_contains, &matchers.any_regex)
   };

   let (all_ok, missing_all) = all_matches(hits, &matchers.all_contains, &matchers.all_regex);

   (any_ok && all_ok, first_hit_rank, mrr, missing_all)
}

fn first_hit_rank(hits: &[EvalHit], matchers: &CaseMatchers) -> Option<usize> {
   let union_contains = matchers
      .any_contains
      .iter()
      .chain(matchers.all_contains.iter());
   let union_regex = matchers.any_regex.iter().chain(matchers.all_regex.iter());

   for hit in hits {
      let path_lc = hit.path.to_ascii_lowercase();
      if union_contains.clone().any(|p| path_lc.contains(p)) {
         return Some(hit.rank);
      }
      if union_regex.clone().any(|re| re.is_match(&hit.path)) {
         return Some(hit.rank);
      }
   }
   None
}

fn has_any_match(hits: &[EvalHit], contains: &[String], regexes: &[Regex]) -> bool {
   for hit in hits {
      let path_lc = hit.path.to_ascii_lowercase();
      if contains.iter().any(|p| path_lc.contains(p)) {
         return true;
      }
      if regexes.iter().any(|re| re.is_match(&hit.path)) {
         return true;
      }
   }
   false
}

fn all_matches(hits: &[EvalHit], contains: &[String], regexes: &[Regex]) -> (bool, Vec<String>) {
   let mut missing = Vec::new();

   for p in contains {
      let mut found = false;
      for hit in hits {
         if hit.path.to_ascii_lowercase().contains(p) {
            found = true;
            break;
         }
      }
      if !found {
         missing.push(p.clone());
      }
   }

   for re in regexes {
      let mut found = false;
      for hit in hits {
         if re.is_match(&hit.path) {
            found = true;
            break;
         }
      }
      if !found {
         missing.push(re.as_str().to_string());
      }
   }

   (missing.is_empty(), missing)
}

fn summarize(cases: &[EvalCaseReport]) -> EvalSummary {
   let total = cases.len();
   let passed = cases.iter().filter(|c| c.passed).count();
   let pass_rate = if total == 0 {
      0.0
   } else {
      passed as f32 / total as f32
   };

   let mean_mrr = if total == 0 {
      0.0
   } else {
      cases.iter().map(|c| c.mrr).sum::<f32>() / total as f32
   };

   let (hit_sum, hit_count) = cases.iter().fold((0usize, 0usize), |acc, c| {
      if let Some(r) = c.first_hit_rank {
         (acc.0 + r, acc.1 + 1)
      } else {
         acc
      }
   });
   let mean_hit_rank = if hit_count == 0 {
      None
   } else {
      Some(hit_sum as f32 / hit_count as f32)
   };

   let mut by_mode: BTreeMap<SearchMode, Vec<&EvalCaseReport>> = BTreeMap::new();
   for c in cases {
      by_mode.entry(c.mode).or_default().push(c);
   }

   let by_mode = by_mode
      .into_iter()
      .map(|(mode, mode_cases)| {
         let mode_total = mode_cases.len();
         let mode_passed = mode_cases.iter().filter(|c| c.passed).count();
         let mode_pass_rate = if mode_total == 0 {
            0.0
         } else {
            mode_passed as f32 / mode_total as f32
         };
         let mode_mean_mrr = if mode_total == 0 {
            0.0
         } else {
            mode_cases.iter().map(|c| c.mrr).sum::<f32>() / mode_total as f32
         };
         (mode, EvalModeSummary {
            total:     mode_total,
            passed:    mode_passed,
            pass_rate: mode_pass_rate,
            mean_mrr:  mode_mean_mrr,
         })
      })
      .collect();

   EvalSummary { total, passed, pass_rate, mean_mrr, mean_hit_rank, by_mode }
}

fn normalize_path(path: &Path) -> String {
   path.to_string_lossy().replace('\\', "/")
}

fn display_path(root: &Path, path: &Path) -> String {
   let rel = path.strip_prefix(root).unwrap_or(path);
   normalize_path(rel)
}

fn preview(content: &str, max_chars: usize, max_lines: usize) -> String {
   if max_chars == 0 || max_lines == 0 || content.is_empty() {
      return String::new();
   }

   let mut out = String::new();
   for (idx, line) in content.lines().enumerate() {
      if idx >= max_lines {
         break;
      }

      if !out.is_empty() {
         out.push('\n');
      }

      if out.len() + line.len() > max_chars {
         let remaining = max_chars.saturating_sub(out.len());
         if remaining > 0 {
            out.push_str(&line.chars().take(remaining).collect::<String>());
         }
         out.push('…');
         return out;
      }

      out.push_str(line);
   }

   if content.lines().count() > max_lines {
      if out.len() + 1 <= max_chars {
         out.push('…');
      }
   }

   out
}
