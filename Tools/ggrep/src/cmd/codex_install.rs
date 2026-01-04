//! Codex MCP installation command.
//!
//! Registers ggrep as an MCP server in Codex CLI.

use std::process::{Command, Stdio};

use console::style;
use serde::Deserialize;

use crate::{Result, error::Error};

#[derive(Debug, Deserialize)]
struct CodexMcpEntry {
   transport: CodexTransport,
}

#[derive(Debug, Deserialize)]
struct CodexTransport {
   command: String,
   args:    Vec<String>,
}

fn codex_get_ggrep() -> Result<Option<CodexMcpEntry>> {
   let output = Command::new("codex")
      .args(["mcp", "get", "ggrep", "--json"])
      .output()
      .map_err(Error::CodexSpawn)?;

   if !output.status.success() {
      return Ok(None);
   }

   Ok(Some(serde_json::from_slice(&output.stdout)?))
}

fn run_codex_command(args: &[&str]) -> Result<()> {
   let status = Command::new("codex")
      .args(args)
      .stdin(Stdio::inherit())
      .stdout(Stdio::inherit())
      .stderr(Stdio::inherit())
      .status()
      .map_err(Error::CodexSpawn)?;

   if !status.success() {
      return Err(Error::CodexCommand(status.code().unwrap_or(-1)));
   }

   Ok(())
}

/// Executes the Codex MCP installation command.
pub fn execute() -> Result<()> {
   println!(
      "{}",
      style("Installing ggrep MCP server for Codex...")
         .cyan()
         .bold()
   );

   let exe = std::env::current_exe()?;
   let exe = exe.to_string_lossy().to_string();

   if let Some(existing) = codex_get_ggrep()? {
      let is_exact_match =
         existing.transport.command == exe && existing.transport.args == vec!["mcp".to_string()];

      if is_exact_match {
         println!("{}", style("✓ Codex already has ggrep configured").green());
         return Ok(());
      }

      println!("{}", style("Updating existing MCP server config...").dim());
      run_codex_command(&["mcp", "remove", "ggrep"])?;
   }

   println!("{}", style("Registering MCP server...").dim());
   let args = ["mcp", "add", "ggrep", "--", exe.as_str(), "mcp"];
   run_codex_command(&args)?;
   println!("{}", style("✓ Added ggrep MCP server").green());

   println!();
   println!("{}", style("Next steps:").bold());
   println!("  1. Restart Codex if it's running");
   println!("  2. Use ggrep via MCP in Codex sessions");

   Ok(())
}
