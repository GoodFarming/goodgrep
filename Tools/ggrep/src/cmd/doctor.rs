//! System health check command.
//!
//! Verifies that all required components are present and properly configured,
//! including models, grammars, and data directories.

use std::path::Path;

use console::style;
use hf_hub::Cache;

use crate::{
   Result, config,
   grammar::{GRAMMAR_URLS, GrammarManager},
   models,
   util::{format_size, get_dir_size},
};

/// Executes the doctor command to check system health.
pub fn execute() -> Result<()> {
   println!("{}\n", style("ggrep Doctor").bold());

   let root = config::base_dir();
   let models = config::model_dir();
   let data = config::data_dir();
   let grammars = config::grammar_dir();

   check_dir("Root", root);
   check_dir("Models", models);
   check_dir("Data (Vector DB)", data);
   check_dir("Grammars", grammars);

   println!();

   let mut all_good = true;

   let cfg = config::get();
   let model_ids = [&cfg.dense_model, &cfg.colbert_model];
   const MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];
   let cache = Cache::new(models.clone());
   for model_id in model_ids {
      let cached_repo = cache.repo(models::repo_for_model(model_id));
      let missing: Vec<&str> = MODEL_FILES
         .iter()
         .copied()
         .filter(|f| cached_repo.get(f).is_none())
         .collect();
      let exists = missing.is_empty();

      let symbol = if exists {
         style("✓").green()
      } else {
         all_good = false;
         style("✗").red()
      };

      let model_root = cached_repo
         .get("config.json")
         .and_then(|p| p.parent().map(|p| p.to_path_buf()))
         .unwrap_or_else(|| models.join(format!("models--{}", model_id.replace('/', "--"))));
      let missing_str = if missing.is_empty() {
         String::new()
      } else {
         format!(" {}", style(format!("(missing: {})", missing.join(", "))).dim())
      };

      println!(
         "{} Model: {} ({}){}",
         symbol,
         style(model_id).dim(),
         style(model_root.display()).dim(),
         missing_str
      );
   }

   println!();

   let grammar_manager = if let Ok(gm) = GrammarManager::with_auto_download(false) {
      Some(gm)
   } else {
      println!("{} Grammar manager: {}", style("✗").red(), style("failed to initialize").dim());
      all_good = false;
      None
   };

   if let Some(gm) = &grammar_manager {
      let available = gm.available_languages();
      let missing = gm.missing_languages();

      for (lang, _) in GRAMMAR_URLS {
         let exists = available.clone().any(|l| &l == lang);

         let symbol = if exists {
            style("✓").green()
         } else {
            style("○").yellow()
         };

         let status = if exists {
            "installed".to_string()
         } else {
            "will download on first use".to_string()
         };

         println!("{} Grammar: {} ({})", symbol, style(lang).dim(), style(status).dim());
      }

      println!();
      println!(
         "{} {} of {} grammars installed",
         style("ℹ").cyan(),
         available.count(),
         GRAMMAR_URLS.len()
      );
      if missing.clone().next().is_some() {
         println!(
            "{} Missing grammars will be downloaded automatically when needed",
            style("ℹ").cyan()
         );
      }
   }

   if data.exists()
      && let Ok(size) = get_dir_size(data)
   {
      println!("\n{} {}", style("Data directory size:").dim(), style(format_size(size)).cyan());
   }

   println!(
      "\n{} {} {} | Rust: {}",
      style("System:").dim(),
      std::env::consts::OS,
      std::env::consts::ARCH,
      rustc_version_runtime::version()
   );

   if all_good {
      println!(
         "\n{}",
         style("✓ All checks passed! You are ready to grep.")
            .green()
            .bold()
      );
   } else {
      println!(
         "\n{}",
         style("✗ Some components are missing. Run 'ggrep setup' to download them.")
            .red()
            .bold()
      );
   }

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
