//! OpenCode MCP installation command.
//!
//! Registers ggrep as an MCP server in OpenCode's config.

use std::{
   fs,
   path::{Path, PathBuf},
   time::{SystemTime, UNIX_EPOCH},
};

use console::style;
use directories::BaseDirs;
use serde_json::{Value, json};

use crate::{
   Result,
   error::{ConfigError, Error},
};

fn opencode_config_path() -> Result<PathBuf> {
   let base = BaseDirs::new().ok_or(ConfigError::GetUserDirectories)?;
   Ok(base.config_dir().join("opencode").join("opencode.json"))
}

fn load_config(path: &Path) -> Result<Value> {
   if path.exists() {
      let contents = fs::read_to_string(path)?;
      Ok(serde_json::from_str(&contents)?)
   } else {
      Ok(json!({ "$schema": "https://opencode.ai/config.json" }))
   }
}

fn ensure_mcp_object(config: &mut Value) -> Result<&mut serde_json::Map<String, Value>> {
   let obj = config.as_object_mut().ok_or_else(|| Error::Server {
      op:     "opencode install",
      reason: "opencode.json must be a JSON object".to_string(),
   })?;

   let mcp = obj.entry("mcp").or_insert_with(|| json!({}));
   mcp.as_object_mut().ok_or_else(|| Error::Server {
      op:     "opencode install",
      reason: "opencode.json mcp must be a JSON object".to_string(),
   })
}

fn backup_existing(path: &Path) -> Result<Option<PathBuf>> {
   if !path.exists() {
      return Ok(None);
   }

   let ts = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
   let backup = path.with_file_name(format!("opencode.json.bak.{ts}"));
   fs::copy(path, &backup)?;
   Ok(Some(backup))
}

fn write_config(path: &Path, config: &Value) -> Result<()> {
   if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)?;
   }

   let payload = format!("{}\n", serde_json::to_string_pretty(config)?);
   fs::write(path, payload)?;
   Ok(())
}

/// Executes the OpenCode MCP installation command.
pub fn execute() -> Result<()> {
   println!(
      "{}",
      style("Installing ggrep MCP server for OpenCode...")
         .cyan()
         .bold()
   );

   let exe = std::env::current_exe()?;
   let exe = exe.to_string_lossy().to_string();

   let config_path = opencode_config_path()?;
   println!("Config: {}", style(config_path.display()).dim());

   let mut config = load_config(&config_path)?;
   let mcp = ensure_mcp_object(&mut config)?;

   let entry = json!({
      "type": "local",
      "command": [exe, "mcp"],
      "enabled": true
   });

   if mcp.get("ggrep") == Some(&entry) {
      println!("{}", style("✓ OpenCode already has ggrep configured").green());
      return Ok(());
   }

   mcp.insert("ggrep".to_string(), entry);

   if let Some(backup) = backup_existing(&config_path)? {
      println!("Backup: {}", style(backup.display()).dim());
   }

   write_config(&config_path, &config)?;
   println!("{}", style("✓ Added ggrep MCP server").green());

   println!();
   println!("{}", style("Next steps:").bold());
   println!("  1. Restart OpenCode if it's running");
   println!("  2. Use ggrep via MCP in OpenCode sessions");

   Ok(())
}
