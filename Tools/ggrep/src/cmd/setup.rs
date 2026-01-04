//! Initial setup command.
//!
//! Downloads required models and grammars, creating necessary directories
//! for first-time use of ggrep.

use std::{fs, path::Path, time::Duration};

use console::style;
use hf_hub::{Cache, api::tokio::ApiBuilder};
use indicatif::{ProgressBar, ProgressStyle};

use crate::{
   Result, config,
   error::ConfigError,
   grammar::{GRAMMAR_URLS, GrammarManager},
   models,
};

/// Executes the setup command to download models and grammars.
pub async fn execute() -> Result<()> {
   println!("{}\n", style("ggrep Setup").bold());

   if config::get().offline {
      return Err(ConfigError::DownloadsDisabled { artifact: "setup".to_string() }.into());
   }

   let models = config::model_dir();
   let data = config::data_dir();
   let grammars = config::grammar_dir();
   fs::create_dir_all(models)?;
   fs::create_dir_all(data)?;
   fs::create_dir_all(grammars)?;

   println!("{}", style("Checking directories...").dim());
   check_dir("Models", models);
   check_dir("Data (Vector DB)", data);
   check_dir("Grammars", grammars);
   println!();

   println!("{}", style("Downloading models...").bold());
   download_models(models).await?;
   println!();

   println!("{}", style("Downloading grammars...").bold());
   download_grammars(grammars).await?;

   println!("\n{}", style("Setup Complete!").green().bold());
   println!("\n{}", style("You can now run:").dim());
   println!("   {} {}", style("ggrep index").green(), style("# Index your repository").dim());
   println!("   {} {}", style("ggrep \"search query\"").green(), style("# Search your code").dim());
   println!("   {} {}", style("ggrep doctor").green(), style("# Check health status").dim());
   println!("\n{}", style("Note: Grammars are also downloaded automatically on first use.").dim());

   Ok(())
}

/// Checks if a directory exists and prints its status.
fn check_dir(name: &str, path: &Path) {
   let exists = path.exists();
   let symbol = if exists {
      style("✓").green()
   } else {
      style("✗").red()
   };
   println!("{} {}: {}", symbol, name, style(path.display()).dim());
}

/// Downloads embedding models from Hugging Face.
async fn download_models(models_dir: &Path) -> Result<()> {
   let cfg = config::get();
   let models = [&cfg.dense_model, &cfg.colbert_model];

   const MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];

   let cache_dir = models_dir.to_path_buf();
   let cache = Cache::new(cache_dir);
   let api = ApiBuilder::from_cache(cache.clone()).build()?;

   for model_id in models {
      let repo_spec = models::repo_for_model(model_id);
      let cached_repo = cache.repo(repo_spec.clone());
      let is_cached = MODEL_FILES.iter().all(|f| cached_repo.get(f).is_some());

      if is_cached {
         println!("{} Model: {}", style("✓").green(), style(model_id).dim());
         continue;
      }

      let spinner = ProgressBar::new_spinner();
      spinner.set_style(
         ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
      );
      spinner.enable_steady_tick(Duration::from_millis(100));
      spinner.set_message(format!("Downloading {model_id}..."));

      match download_model_from_hf(&api, model_id, MODEL_FILES).await {
         Ok(()) => {
            spinner.finish_with_message(format!(
               "{} Downloaded: {}",
               style("✓").green(),
               style(model_id).dim()
            ));
         },
         Err(e) => {
            spinner.finish_with_message(format!(
               "{} Failed: {} - {}",
               style("✗").red(),
               model_id,
               e
            ));
         },
      }
   }

   Ok(())
}

/// Downloads tree-sitter grammar files for supported languages.
async fn download_grammars(grammars_dir: &Path) -> Result<()> {
   let grammar_manager = GrammarManager::with_auto_download(true)?;

   for pair @ (lang, _url) in GRAMMAR_URLS {
      let grammar_path = grammars_dir.join(format!("tree-sitter-{lang}.wasm"));

      if grammar_path.exists() {
         println!("{} Grammar: {}", style("✓").green(), style(lang).dim());
         continue;
      }

      let spinner = ProgressBar::new_spinner();
      spinner.set_style(
         ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
      );
      spinner.enable_steady_tick(Duration::from_millis(100));
      spinner.set_message(format!("Downloading {lang} grammar..."));

      match grammar_manager.download_grammar(*pair).await {
         Ok(_) => {
            spinner.finish_with_message(format!(
               "{} Downloaded: {}",
               style("✓").green(),
               style(lang).dim()
            ));
         },
         Err(e) => {
            spinner.finish_with_message(format!("{} Failed: {} - {}", style("✗").red(), lang, e));
         },
      }
   }

   Ok(())
}

/// Downloads a specific model from Hugging Face Hub to the destination
/// directory.
async fn download_model_from_hf(
   api: &hf_hub::api::tokio::Api,
   model_id: &str,
   files_to_download: &[&str],
) -> Result<()> {
   let repo = api.repo(models::repo_for_model(model_id));

   for file in files_to_download {
      let _path = repo.get(file).await?;
   }

   Ok(())
}
