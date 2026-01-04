//! Configuration management for model settings, performance tuning, and paths.

use std::{
   fs,
   path::{Path, PathBuf},
   sync::OnceLock,
};

use directories::BaseDirs;
use figment::{
   Figment,
   providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};

static CONFIG: OnceLock<Config> = OnceLock::new();

pub const MAX_FILE_SIZE_BYTES_CAP: u64 = 10_485_760;
pub const MAX_CHUNKS_PER_FILE_CAP: usize = 2000;
pub const MAX_BYTES_PER_SYNC_CAP: u64 = 268_435_456;
pub const MAX_CANDIDATES_CAP: usize = 10_000;
pub const MAX_TOTAL_SNIPPET_BYTES_CAP: usize = 10_485_760;
pub const MAX_SNIPPET_BYTES_PER_RESULT_CAP: usize = 262_144;
pub const MAX_OPEN_SEGMENTS_PER_QUERY_CAP: usize = 512;
pub const MAX_OPEN_SEGMENTS_GLOBAL_CAP: usize = 4096;

/// Application configuration loaded from config file and environment variables
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
   pub dense_model:   String,
   pub colbert_model: String,
   pub dense_dim:     usize,
   pub colbert_dim:   usize,

   pub query_prefix: String,
   pub doc_prefix: String,
   pub dense_max_length: usize,
   pub colbert_max_length: usize,
   pub default_batch_size: usize,
   pub max_batch_size: usize,
   pub sync_file_batch_size: usize,
   pub max_file_size_bytes: u64,
   pub max_chunks_per_file: usize,
   pub max_bytes_per_sync: u64,
   pub max_threads: usize,
   pub max_concurrent_queries: usize,
   pub max_query_queue: usize,
   pub max_concurrent_queries_per_client: usize,
   pub query_timeout_ms: u64,
   pub max_query_results: usize,
   pub max_query_per_file: usize,
   pub max_candidates: usize,
   pub max_total_snippet_bytes: usize,
   pub max_snippet_bytes_per_result: usize,
   pub max_open_segments_per_query: usize,
   pub max_open_segments_global: usize,
   pub slow_query_ms: u64,
   pub budget_query_p50_ms: u64,
   pub budget_query_p95_ms: u64,
   pub budget_max_segments_touched: u64,
   pub budget_publish_ms: u64,
   pub budget_gc_ms: u64,
   pub budget_compaction_ms: u64,
   pub max_request_bytes: usize,
   pub max_response_bytes: usize,
   pub max_store_bytes: u64,
   pub max_cache_bytes: u64,
   pub max_log_bytes: u64,
   pub max_embed_global: usize,
   pub embed_lock_ttl_ms: u64,
   pub retain_snapshots_min: usize,
   pub retain_snapshots_min_age_secs: u64,
   pub staging_ttl_ms: u64,
   pub gc_safety_margin_ms: u64,
   pub lease_ttl_ms: u64,
   pub max_segments_per_snapshot: usize,
   pub max_total_segments_referenced: usize,
   pub max_tombstones_per_snapshot: usize,
   pub compaction_overdue_segments: usize,
   pub compaction_overdue_tombstones: usize,

   pub port:                     u16,
   pub idle_timeout_secs:        u64,
   pub idle_check_interval_secs: u64,
   pub worker_timeout_ms:        u64,

   pub low_impact:      bool,
   pub disable_gpu:     bool,
   pub fast_mode:       bool,
   pub offline:         bool,
   pub profile_enabled: bool,
   pub skip_meta_save:  bool,
   pub debug_models:    bool,
   pub debug_embed:     bool,
}

impl Default for Config {
   fn default() -> Self {
      Self {
         dense_model: "ibm-granite/granite-embedding-small-english-r2@\
                       c949f235cb63fcbd58b1b9e139ff63c8be764eeb"
            .to_string(),
         colbert_model: "answerdotai/answerai-colbert-small-v1@\
                         be1703c55532145a844da800eea4c9a692d7e267"
            .to_string(),
         dense_dim: 384,
         colbert_dim: 96,
         query_prefix: String::new(),
         doc_prefix: String::new(),
         dense_max_length: 256,
         colbert_max_length: 256,
         default_batch_size: 48,
         max_batch_size: 96,
         sync_file_batch_size: 8,
         max_file_size_bytes: MAX_FILE_SIZE_BYTES_CAP,
         max_chunks_per_file: MAX_CHUNKS_PER_FILE_CAP,
         max_bytes_per_sync: MAX_BYTES_PER_SYNC_CAP,
         max_threads: 32,
         max_concurrent_queries: 8,
         max_query_queue: 32,
         max_concurrent_queries_per_client: 4,
         query_timeout_ms: 60000,
         max_query_results: 200,
         max_query_per_file: 50,
         max_candidates: 2000,
         max_total_snippet_bytes: 1_048_576,
         max_snippet_bytes_per_result: 32_768,
         max_open_segments_per_query: 64,
         max_open_segments_global: 512,
         slow_query_ms: 2000,
         budget_query_p50_ms: 300,
         budget_query_p95_ms: 1500,
         budget_max_segments_touched: 64,
         budget_publish_ms: 600_000,
         budget_gc_ms: 120_000,
         budget_compaction_ms: 600_000,
         max_request_bytes: 1_048_576,
         max_response_bytes: 10_485_760,
         max_store_bytes: 0,
         max_cache_bytes: 0,
         max_log_bytes: 0,
         max_embed_global: 2,
         embed_lock_ttl_ms: 120_000,
         retain_snapshots_min: 5,
         retain_snapshots_min_age_secs: 600,
         staging_ttl_ms: 1_800_000,
         gc_safety_margin_ms: 120_000,
         lease_ttl_ms: 120_000,
         max_segments_per_snapshot: 64,
         max_total_segments_referenced: 256,
         max_tombstones_per_snapshot: 250_000,
         compaction_overdue_segments: 48,
         compaction_overdue_tombstones: 200_000,
         port: 4444,
         idle_timeout_secs: 30 * 60,
         idle_check_interval_secs: 60,
         worker_timeout_ms: 60000,
         low_impact: false,
         disable_gpu: false,
         fast_mode: false,
         offline: false,
         profile_enabled: false,
         skip_meta_save: false,
         debug_models: false,
         debug_embed: false,
      }
   }
}

impl Config {
   pub fn load() -> Self {
      Self::load_with_repo_path(None)
   }

   pub fn load_with_repo(root: &Path) -> Self {
      Self::load_with_repo_path(Some(root))
   }

   fn load_with_repo_path(repo_root: Option<&Path>) -> Self {
      let config_path = ensure_global_config();

      let mut figment =
         Figment::from(Serialized::defaults(Self::default())).merge(Toml::file(config_path));

      if let Some(root) = repo_root {
         let repo_path = repo_config_path(root);
         if repo_path.exists() {
            figment = figment.merge(Toml::file(repo_path));
         }
      }

      figment
         .merge(Env::prefixed("GGREP_").lowercase(true))
         .extract()
         .inspect_err(|e| tracing::warn!("failed to parse config: {e}"))
         .unwrap_or_default()
   }

   fn create_default_config(path: &Path) {
      if let Some(parent) = path.parent() {
         let _ = fs::create_dir_all(parent);
      }
      let default_config = Self::default();
      if let Ok(toml) = toml::to_string_pretty(&default_config) {
         let _ = fs::write(path, toml);
      }
   }

   /// Returns the configured batch size, capped at maximum
   pub fn batch_size(&self) -> usize {
      self.default_batch_size.min(self.max_batch_size)
   }

   /// Calculates default thread count based on available CPUs
   pub fn default_threads(&self) -> usize {
      (num_cpus::get().saturating_sub(4)).clamp(1, self.max_threads)
   }

   pub fn effective_max_file_size_bytes(&self) -> u64 {
      self.max_file_size_bytes.min(MAX_FILE_SIZE_BYTES_CAP)
   }

   pub fn effective_max_chunks_per_file(&self) -> usize {
      self.max_chunks_per_file.min(MAX_CHUNKS_PER_FILE_CAP)
   }

   pub fn effective_max_bytes_per_sync(&self) -> u64 {
      self.max_bytes_per_sync.min(MAX_BYTES_PER_SYNC_CAP)
   }

   pub fn effective_max_candidates(&self) -> usize {
      self.max_candidates.min(MAX_CANDIDATES_CAP).max(1)
   }

   pub fn effective_max_total_snippet_bytes(&self) -> usize {
      self
         .max_total_snippet_bytes
         .min(MAX_TOTAL_SNIPPET_BYTES_CAP)
         .max(1)
   }

   pub fn effective_max_snippet_bytes_per_result(&self) -> usize {
      self
         .max_snippet_bytes_per_result
         .min(MAX_SNIPPET_BYTES_PER_RESULT_CAP)
         .max(1)
   }

   pub fn effective_max_open_segments_per_query(&self) -> usize {
      self
         .max_open_segments_per_query
         .min(MAX_OPEN_SEGMENTS_PER_QUERY_CAP)
         .max(1)
   }

   pub fn effective_max_open_segments_global(&self) -> usize {
      self
         .max_open_segments_global
         .min(MAX_OPEN_SEGMENTS_GLOBAL_CAP)
         .max(1)
   }

   pub fn effective_max_concurrent_queries_per_client(&self) -> usize {
      if self.max_concurrent_queries_per_client == 0 {
         return self.max_concurrent_queries.max(1);
      }
      self
         .max_concurrent_queries_per_client
         .min(self.max_concurrent_queries.max(1))
         .max(1)
   }
}

/// Returns the global configuration instance
pub fn get() -> &'static Config {
   CONFIG.get_or_init(Config::load)
}

/// Initializes config using a repo-root `.ggrep.toml` if present.
pub fn init_for_root(root: &Path) -> &'static Config {
   let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
   CONFIG.get_or_init(|| Config::load_with_repo(&root))
}

/// Returns the base directory for ggrep data and configuration
pub fn base_dir() -> &'static PathBuf {
   static ONCE: OnceLock<PathBuf> = OnceLock::new();
   ONCE.get_or_init(|| resolve_base_dir(".ggrep"))
}

fn legacy_config_file_path() -> PathBuf {
   // Legacy smgrep config for seamless migration.
   resolve_base_dir(".smgrep").join("config.toml")
}

fn ensure_global_config() -> PathBuf {
   let config_path = config_file_path();
   if !config_path.exists() {
      let legacy_path = legacy_config_file_path();
      if legacy_path.exists() {
         if let Some(parent) = config_path.parent() {
            let _ = fs::create_dir_all(parent);
         }
         let _ = fs::copy(&legacy_path, config_path);
      }
      if !config_path.exists() {
         Config::create_default_config(config_path);
      }
   }
   config_path.to_path_buf()
}

pub fn repo_config_path(root: &Path) -> PathBuf {
   root.join(".ggrep.toml")
}

pub fn validate_repo_config(cfg: &Config) -> Result<()> {
   if cfg.max_file_size_bytes > MAX_FILE_SIZE_BYTES_CAP {
      return Err(
         ConfigError::InvalidRepoConfig(format!(
            "max_file_size_bytes {} exceeds hard cap {}",
            cfg.max_file_size_bytes, MAX_FILE_SIZE_BYTES_CAP
         ))
         .into(),
      );
   }
   if cfg.max_chunks_per_file > MAX_CHUNKS_PER_FILE_CAP {
      return Err(
         ConfigError::InvalidRepoConfig(format!(
            "max_chunks_per_file {} exceeds hard cap {}",
            cfg.max_chunks_per_file, MAX_CHUNKS_PER_FILE_CAP
         ))
         .into(),
      );
   }
   if cfg.max_bytes_per_sync > MAX_BYTES_PER_SYNC_CAP {
      return Err(
         ConfigError::InvalidRepoConfig(format!(
            "max_bytes_per_sync {} exceeds hard cap {}",
            cfg.max_bytes_per_sync, MAX_BYTES_PER_SYNC_CAP
         ))
         .into(),
      );
   }
   if cfg.max_candidates > MAX_CANDIDATES_CAP {
      return Err(
         ConfigError::InvalidRepoConfig(format!(
            "max_candidates {} exceeds hard cap {}",
            cfg.max_candidates, MAX_CANDIDATES_CAP
         ))
         .into(),
      );
   }
   if cfg.max_total_snippet_bytes > MAX_TOTAL_SNIPPET_BYTES_CAP {
      return Err(
         ConfigError::InvalidRepoConfig(format!(
            "max_total_snippet_bytes {} exceeds hard cap {}",
            cfg.max_total_snippet_bytes, MAX_TOTAL_SNIPPET_BYTES_CAP
         ))
         .into(),
      );
   }
   if cfg.max_snippet_bytes_per_result > MAX_SNIPPET_BYTES_PER_RESULT_CAP {
      return Err(
         ConfigError::InvalidRepoConfig(format!(
            "max_snippet_bytes_per_result {} exceeds hard cap {}",
            cfg.max_snippet_bytes_per_result, MAX_SNIPPET_BYTES_PER_RESULT_CAP
         ))
         .into(),
      );
   }
   if cfg.max_open_segments_per_query > MAX_OPEN_SEGMENTS_PER_QUERY_CAP {
      return Err(
         ConfigError::InvalidRepoConfig(format!(
            "max_open_segments_per_query {} exceeds hard cap {}",
            cfg.max_open_segments_per_query, MAX_OPEN_SEGMENTS_PER_QUERY_CAP
         ))
         .into(),
      );
   }
   if cfg.max_open_segments_global > MAX_OPEN_SEGMENTS_GLOBAL_CAP {
      return Err(
         ConfigError::InvalidRepoConfig(format!(
            "max_open_segments_global {} exceeds hard cap {}",
            cfg.max_open_segments_global, MAX_OPEN_SEGMENTS_GLOBAL_CAP
         ))
         .into(),
      );
   }
   Ok(())
}

fn resolve_base_dir(dir_name: &str) -> PathBuf {
   BaseDirs::new()
      .map(|d| d.home_dir().join(dir_name))
      .or_else(|| {
         std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join(dir_name))
      })
      .unwrap_or_else(|| {
         std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(dir_name)
      })
}

macro_rules! define_paths {
   ($($fn_name:ident: $path:literal),* $(,)?) => {
      $(
         pub fn $fn_name() -> &'static PathBuf {
            static ONCE: OnceLock<PathBuf> = OnceLock::new();
            ONCE.get_or_init(|| base_dir().join($path))
         }
      )*
   };
}

define_paths! {
   config_file_path: "config.toml",
   model_dir: "models",
   marketplace_dir: "marketplace",
   data_dir: "data",
   grammar_dir: "grammars",
   socket_dir: "sockets",
   meta_dir: "meta",
}
