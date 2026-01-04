use std::{path::PathBuf, sync::LazyLock};

use clap::{Parser, Subcommand};
use ggrep::{
   Error, Result,
   cmd::{self, search::SearchOptions},
   types::SearchMode,
   version,
};
use tracing::Level;
use tracing_subscriber::EnvFilter;

static VERSION_STRING: LazyLock<String> = LazyLock::new(version::version_string);

fn version_string() -> &'static str {
   &VERSION_STRING
}

/// Command-line arguments for the ggrep application
#[derive(Parser)]
#[command(name = "ggrep")]
#[command(about = "Semantic search across code + docs")]
#[command(version = version_string())]
struct Cli {
   #[arg(long, env = "GGREP_STORE")]
   store: Option<String>,

   #[command(subcommand)]
   command: Option<Cmd>,

   #[arg(trailing_var_arg = true)]
   query: Vec<String>,
}

/// Available subcommands for ggrep
#[derive(Subcommand)]
enum Cmd {
   #[command(about = "Search indexed code semantically")]
   Search {
      #[arg(help = "Search query")]
      query: String,

      #[arg(help = "Directory to search (default: cwd)")]
      path: Option<PathBuf>,

      #[arg(
         short = 'm',
         long,
         alias = "max-count",
         default_value = "10",
         help = "Maximum total results"
      )]
      max: usize,

      #[arg(long, default_value = "1", help = "Maximum results per file")]
      per_file: usize,

      #[arg(
         short = 'd',
         long,
         help = "Discovery mode (favor breadth across code + docs + graphs)",
         conflicts_with_all = ["implementation", "planning", "debug_mode"]
      )]
      discovery: bool,

      #[arg(
         short = 'i',
         long,
         help = "Implementation mode (favor code)",
         conflicts_with_all = ["discovery", "planning", "debug_mode"]
      )]
      implementation: bool,

      #[arg(
         short = 'p',
         long,
         help = "Planning mode (favor docs + graphs)",
         conflicts_with_all = ["discovery", "implementation", "debug_mode"]
      )]
      planning: bool,

      #[arg(
         short = 'b',
         long = "debug",
         help = "Debug mode (favor debugging code paths)",
         conflicts_with_all = ["discovery", "implementation", "planning"]
      )]
      debug_mode: bool,

      #[arg(short = 'c', long, help = "Show full content")]
      content: bool,

      #[arg(short = 'n', long, help = "Show file + line only (no snippet)")]
      no_snippet: bool,

      #[arg(short = 's', long, help = "Show a short snippet preview")]
      short_snippet: bool,

      #[arg(short = 'l', long, help = "Show a long snippet preview")]
      long_snippet: bool,

      #[arg(long, help = "Show file paths only (like grep -l)")]
      compact: bool,

      #[arg(long, help = "Show relevance scores")]
      scores: bool,

      #[arg(long, help = "Force re-index before search")]
      sync: bool,

      #[arg(long, help = "Show what would be indexed")]
      dry_run: bool,

      #[arg(long, help = "Allow degraded snapshots when syncing")]
      allow_degraded: bool,

      #[arg(long, help = "JSON output")]
      json: bool,

      #[arg(long, help = "Show explainability metadata")]
      explain: bool,

      #[arg(long, help = "Skip ColBERT reranking")]
      no_rerank: bool,

      #[arg(long, help = "Use the default store id with an '-eval' suffix")]
      eval_store: bool,

      #[arg(long, help = "Disable ANSI colors and use simpler formatting")]
      plain: bool,
   },

   #[command(about = "Evaluate semantic search quality on a query suite")]
   Eval {
      #[arg(long, help = "Path to eval suite TOML file (default: Datasets/ggrep/eval_cases.toml)")]
      cases: Option<PathBuf>,

      #[arg(long, help = "Output JSON report path (default: temp dir)")]
      out: Option<PathBuf>,

      #[arg(long, help = "Directory to search/index (default: cwd)")]
      path: Option<PathBuf>,

      #[arg(long, help = "Run only the given case id(s) (repeatable)")]
      only: Vec<String>,

      #[arg(long, help = "Skip indexing/sync and evaluate existing store only")]
      no_sync: bool,

      #[arg(short = 'm', long, help = "Override max results (k) for all cases")]
      max: Option<usize>,

      #[arg(long, help = "Override per-file limit for all cases")]
      per_file: Option<usize>,

      #[arg(
         long,
         help = "Override search mode for all cases \
                 (balanced|discovery|implementation|planning|debug)"
      )]
      mode: Option<String>,

      #[arg(long, help = "Skip ColBERT reranking for all cases")]
      no_rerank: bool,

      #[arg(long, help = "Include anchor chunks in evaluation")]
      include_anchors: bool,

      #[arg(long, help = "Use the default store id with an '-eval' suffix")]
      eval_store: bool,

      #[arg(long, help = "Fail if pass-rate is below this threshold (0..1)")]
      fail_under_pass_rate: Option<f32>,

      #[arg(long, help = "Fail if mean MRR is below this threshold (0..1)")]
      fail_under_mrr: Option<f32>,

      #[arg(long, help = "Baseline eval JSON for regression gating")]
      baseline: Option<PathBuf>,

      #[arg(long, help = "Allowed pass-rate drop vs baseline (0..1)")]
      baseline_max_drop_pass_rate: Option<f32>,

      #[arg(long, help = "Allowed mean MRR drop vs baseline (0..1)")]
      baseline_max_drop_mrr: Option<f32>,
   },

   #[command(about = "Index a directory for semantic search")]
   Index {
      #[arg(short = 'p', long, help = "Directory to index (default: cwd)")]
      path: Option<PathBuf>,

      #[arg(short = 'd', long, help = "Show what would be indexed")]
      dry_run: bool,

      #[arg(short = 'r', long, help = "Delete and re-index")]
      reset: bool,

      #[arg(long, help = "Use the default store id with an '-eval' suffix")]
      eval_store: bool,

      #[arg(long, help = "Allow degraded snapshots when syncing")]
      allow_degraded: bool,
   },

   #[command(about = "Start a background daemon for faster searches")]
   Serve {
      #[arg(long, help = "Directory to serve (default: cwd)")]
      path: Option<PathBuf>,

      #[arg(long, help = "Allow degraded snapshots when syncing")]
      allow_degraded: bool,
   },

   #[command(about = "Stop the daemon for a directory")]
   Stop {
      #[arg(long, help = "Directory of server to stop (default: cwd)")]
      path: Option<PathBuf>,
   },

   #[command(name = "stop-all", about = "Stop all running daemons")]
   StopAll,

   #[command(about = "Show status of running daemons")]
   Status {
      #[arg(long, help = "JSON output")]
      json: bool,
   },

   #[command(about = "Run health checks and report status")]
   Health {
      #[arg(long, help = "JSON output")]
      json: bool,
   },

   #[command(about = "Audit snapshot counts for drift")]
   Audit {
      #[arg(short = 'p', long, help = "Directory to audit (default: cwd)")]
      path: Option<PathBuf>,

      #[arg(long, help = "JSON output")]
      json: bool,
   },

   #[command(about = "Compact index segments and prune tombstones")]
   Compact {
      #[arg(short = 'p', long, help = "Directory to compact (default: cwd)")]
      path: Option<PathBuf>,

      #[arg(long, help = "Force compaction even if thresholds not exceeded")]
      force: bool,

      #[arg(long, help = "JSON output")]
      json: bool,
   },

   #[command(name = "upgrade-store", about = "Upgrade store format (placeholder)")]
   UpgradeStore {
      #[arg(short = 'p', long, help = "Directory to upgrade (default: cwd)")]
      path: Option<PathBuf>,
   },

   #[command(about = "Repair missing segments using snapshot mapping")]
   Repair {
      #[arg(short = 'p', long, help = "Directory to repair (default: cwd)")]
      path: Option<PathBuf>,
   },

   #[command(about = "Remove index data and metadata for a store")]
   Clean {
      #[arg(help = "Store ID to clean (default: current directory's store)")]
      store_id: Option<String>,

      #[arg(long, help = "Clean all stores")]
      all: bool,
   },

   #[command(name = "clone-store", about = "Clone a store to a new store id")]
   CloneStore {
      #[arg(help = "Source store id")]
      from: String,

      #[arg(help = "Destination store id")]
      to: String,

      #[arg(long, help = "Overwrite destination if it exists")]
      overwrite: bool,
   },

   #[command(name = "promote-eval", about = "Clone <store>-eval into <store>")]
   PromoteEval {
      #[arg(long, help = "Directory to promote (default: cwd)")]
      path: Option<PathBuf>,

      #[arg(long, help = "Overwrite destination if it exists")]
      overwrite: bool,
   },

   #[command(about = "Garbage-collect stores and artifacts")]
   Gc {
      #[arg(short = 'p', long, help = "Directory to GC (default: cwd)")]
      path: Option<PathBuf>,

      #[arg(long, help = "GC orphaned stores under ~/.ggrep/data")]
      stores: bool,

      #[arg(long, help = "Delete instead of dry-run")]
      force: bool,

      #[arg(long, help = "JSON output")]
      json: bool,
   },

   #[command(about = "Download and configure embedding models")]
   Setup,

   #[command(about = "Check system configuration and dependencies")]
   Doctor,

   #[command(about = "List available stores")]
   List {
      #[arg(long, help = "JSON output")]
      json: bool,
   },

   #[command(about = "List all stores")]
   Stores {
      #[arg(long, help = "JSON output")]
      json: bool,
   },

   #[command(name = "claude-install", about = "Install ggrep as a Claude Code MCP server")]
   ClaudeInstall,

   #[command(name = "codex-install", about = "Install ggrep as a Codex MCP server")]
   CodexInstall,

   #[command(name = "gemini-install", about = "Install ggrep as a Gemini MCP server")]
   GeminiInstall,

   #[command(name = "opencode-install", about = "Install ggrep as an OpenCode MCP server")]
   OpencodeInstall,

   #[command(name = "mcp", about = "Run as an MCP server (stdio transport)")]
   Mcp,
}

#[tokio::main]
async fn main() {
   tracing_subscriber::fmt()
      .with_env_filter(EnvFilter::from_default_env().add_directive(Level::WARN.into()))
      .init();

   let cli = Cli::parse();
   if let Err(err) = run(cli).await {
      if !matches!(err, Error::Reported { .. }) {
         eprintln!("{err}");
      }
      std::process::exit(err.exit_code());
   }
}

async fn run(cli: Cli) -> Result<()> {
   if cli.command.is_none() && !cli.query.is_empty() {
      let query = cli.query.join(" ");
      return cmd::search::execute(query, None, 10, 1, SearchOptions::default(), false, cli.store)
         .await;
   }

   match cli.command {
      Some(Cmd::Search {
         query,
         path,
         max,
         per_file,
         discovery,
         implementation,
         planning,
         debug_mode,
         content,
         no_snippet,
         short_snippet,
         long_snippet,
         compact,
         scores,
         sync,
         dry_run,
         allow_degraded,
         json,
         explain,
         no_rerank,
         eval_store,
         plain,
      }) => {
         cmd::search::execute(
            query,
            path,
            max,
            per_file,
            SearchOptions {
               content,
               no_snippet,
               short_snippet,
               long_snippet,
               compact,
               scores,
               sync,
               dry_run,
               allow_degraded,
               json,
               explain,
               no_rerank,
               plain,
               mode: if discovery {
                  SearchMode::Discovery
               } else if implementation {
                  SearchMode::Implementation
               } else if planning {
                  SearchMode::Planning
               } else if debug_mode {
                  SearchMode::Debug
               } else {
                  SearchMode::Balanced
               },
            },
            eval_store,
            cli.store,
         )
         .await
      },
      Some(Cmd::Eval {
         cases,
         out,
         path,
         only,
         no_sync,
         max,
         per_file,
         mode,
         no_rerank,
         include_anchors,
         eval_store,
         fail_under_pass_rate,
         fail_under_mrr,
         baseline,
         baseline_max_drop_pass_rate,
         baseline_max_drop_mrr,
      }) => {
         cmd::eval::execute(
            cases,
            out,
            path,
            only,
            no_sync,
            max,
            per_file,
            mode,
            no_rerank,
            include_anchors,
            eval_store,
            fail_under_pass_rate,
            fail_under_mrr,
            baseline,
            baseline_max_drop_pass_rate,
            baseline_max_drop_mrr,
            cli.store,
         )
         .await
      },
      Some(Cmd::Index { path, dry_run, reset, eval_store, allow_degraded }) => {
         cmd::index::execute(path, dry_run, reset, eval_store, allow_degraded, cli.store).await
      },
      Some(Cmd::Serve { path, allow_degraded }) => {
         cmd::serve::execute(path, cli.store, allow_degraded).await
      },
      Some(Cmd::Stop { path }) => cmd::stop::execute(path).await,
      Some(Cmd::StopAll) => cmd::stop_all::execute().await,
      Some(Cmd::Status { json }) => cmd::status::execute(json).await,
      Some(Cmd::Health { json }) => cmd::health::execute(json).await,
      Some(Cmd::Audit { path, json }) => cmd::audit::execute(path, json, cli.store).await,
      Some(Cmd::Compact { path, force, json }) => {
         cmd::compact::execute(path, force, json, cli.store).await
      }
      Some(Cmd::UpgradeStore { path }) => cmd::upgrade_store::execute(path, cli.store),
      Some(Cmd::Repair { path }) => cmd::repair::execute(path, cli.store).await,
      Some(Cmd::Clean { store_id, all }) => cmd::clean::execute(store_id, all),
      Some(Cmd::CloneStore { from, to, overwrite }) => {
         cmd::clone_store::execute(from, to, overwrite)
      },
      Some(Cmd::PromoteEval { path, overwrite }) => {
         cmd::promote_eval::execute(path, overwrite, cli.store)
      },
      Some(Cmd::Gc { path, stores, force, json }) => {
         cmd::gc::execute(stores, force, json, path, cli.store).await
      }
      Some(Cmd::Setup) => cmd::setup::execute().await,
      Some(Cmd::Doctor) => cmd::doctor::execute(),
      Some(Cmd::List { json }) => cmd::list::execute(json),
      Some(Cmd::Stores { json }) => cmd::list::execute(json),
      Some(Cmd::ClaudeInstall) => cmd::claude_install::execute(),
      Some(Cmd::CodexInstall) => cmd::codex_install::execute(),
      Some(Cmd::GeminiInstall) => cmd::gemini_install::execute(),
      Some(Cmd::OpencodeInstall) => cmd::opencode_install::execute(),
      Some(Cmd::Mcp) => cmd::mcp::execute().await,
      None => {
         Err(Error::Server { op: "cli", reason: "no command or query provided".to_string() }.into())
      },
   }
}
