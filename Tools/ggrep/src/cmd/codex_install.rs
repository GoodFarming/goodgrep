//! Codex MCP installation command.
//!
//! Registers ggrep as an MCP server in Codex CLI.

use std::process::{Command, Stdio};

use console::style;

use crate::{Result, error::Error};

fn codex_has_ggrep() -> Result<bool> {
   let output = Command::new("codex")
      .args(["mcp", "get", "ggrep", "--json"])
      .output()
      .map_err(Error::CodexSpawn)?;

   Ok(output.status.success())
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

   if codex_has_ggrep()? {
      println!("{}", style("✓ Codex already has ggrep configured").green());
      return Ok(());
   }

   println!("{}", style("Registering MCP server...").dim());
   run_codex_command(&["mcp", "add", "ggrep", "--", "ggrep", "mcp"])?;
   println!("{}", style("✓ Added ggrep MCP server").green());

   println!();
   println!("{}", style("Next steps:").bold());
   println!("  1. Restart Codex if it's running");
   println!("  2. Use ggrep via MCP in Codex sessions");

   Ok(())
}
