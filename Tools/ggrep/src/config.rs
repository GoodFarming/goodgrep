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

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Application configuration loaded from config file and environment variables
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
   pub dense_model:   String,
   pub colbert_model: String,
   pub dense_dim:     usize,
   pub colbert_dim:   usize,

   pub query_prefix:         String,
   pub doc_prefix:           String,
   pub dense_max_length:     usize,
   pub colbert_max_length:   usize,
   pub default_batch_size:   usize,
   pub max_batch_size:       usize,
   pub sync_file_batch_size: usize,
   pub max_threads:          usize,

   pub port:                     u16,
   pub idle_timeout_secs:        u64,
   pub idle_check_interval_secs: u64,
   pub worker_timeout_ms:        u64,

   pub low_impact:      bool,
   pub disable_gpu:     bool,
   pub fast_mode:       bool,
   pub profile_enabled: bool,
   pub skip_meta_save:  bool,
   pub debug_models:    bool,
   pub debug_embed:     bool,
}

impl Default for Config {
   fn default() -> Self {
      Self {
         dense_model:              "ibm-granite/granite-embedding-small-english-r2".to_string(),
         colbert_model:            "answerdotai/answerai-colbert-small-v1".to_string(),
         dense_dim:                384,
         colbert_dim:              96,
         query_prefix:             String::new(),
         doc_prefix:               String::new(),
         dense_max_length:         256,
         colbert_max_length:       256,
         default_batch_size:       48,
         max_batch_size:           96,
         sync_file_batch_size:     8,
         max_threads:              32,
         port:                     4444,
         idle_timeout_secs:        30 * 60,
         idle_check_interval_secs: 60,
         worker_timeout_ms:        60000,
         low_impact:               false,
         disable_gpu:              false,
         fast_mode:                false,
         profile_enabled:          false,
         skip_meta_save:           false,
         debug_models:             false,
         debug_embed:              false,
      }
   }
}

impl Config {
   pub fn load() -> Self {
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
            Self::create_default_config(config_path);
         }
      }

      Figment::from(Serialized::defaults(Self::default()))
         .merge(Toml::file(config_path))
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
}

/// Returns the global configuration instance
pub fn get() -> &'static Config {
   CONFIG.get_or_init(Config::load)
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
