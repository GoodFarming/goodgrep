//! Gemini MCP installation command.
//!
//! Registers ggrep as an MCP server in Gemini CLI.

use std::process::{Command, Stdio};

use console::style;

use crate::{Result, error::Error};

fn gemini_has_ggrep() -> Result<bool> {
   let output = Command::new("gemini")
      .args(["mcp", "list"])
      .output()
      .map_err(Error::GeminiSpawn)?;

   if !output.status.success() {
      return Err(Error::GeminiCommand(output.status.code().unwrap_or(-1)));
   }

   let stdout = String::from_utf8_lossy(&output.stdout);
   Ok(stdout
      .lines()
      .any(|line| line.trim_start().starts_with("ggrep")))
}

fn run_gemini_command(args: &[&str]) -> Result<()> {
   let status = Command::new("gemini")
      .args(args)
      .stdin(Stdio::inherit())
      .stdout(Stdio::inherit())
      .stderr(Stdio::inherit())
      .status()
      .map_err(Error::GeminiSpawn)?;

   if !status.success() {
      return Err(Error::GeminiCommand(status.code().unwrap_or(-1)));
   }

   Ok(())
}

/// Executes the Gemini MCP installation command.
pub fn execute() -> Result<()> {
   println!(
      "{}",
      style("Installing ggrep MCP server for Gemini...")
         .cyan()
         .bold()
   );

   if gemini_has_ggrep()? {
      println!("{}", style("✓ Gemini already has ggrep configured").green());
      return Ok(());
   }

   println!("{}", style("Registering MCP server...").dim());
   run_gemini_command(&["mcp", "add", "ggrep", "ggrep", "mcp"])?;
   println!("{}", style("✓ Added ggrep MCP server").green());

   println!();
   println!("{}", style("Next steps:").bold());
   println!("  1. Restart Gemini CLI if it's running");
   println!("  2. Use ggrep via MCP in Gemini sessions");

   Ok(())
}
