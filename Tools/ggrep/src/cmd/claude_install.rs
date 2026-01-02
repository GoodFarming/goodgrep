//! Claude Code plugin installation command.
//!
//! Installs the ggrep plugin for Claude Code by extracting the bundled plugin
//! files and registering them with the Claude CLI.

use std::{
   fs,
   io::Cursor,
   path::PathBuf,
   process::{Command, Stdio},
};

use console::style;
use serde::Deserialize;

use crate::{Result, config, error::Error};

/// Embedded plugin bundle containing all plugin files.
const PLUGIN_BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/plugin-bundle.tar"));

#[derive(Debug, Deserialize)]
struct MarketplaceManifest {
   name: String,
}

/// Extracts the embedded plugin bundle to the specified destination directory.
fn extract_plugin_bundle(dest: &PathBuf) -> Result<()> {
   fs::create_dir_all(dest)?;
   let mut archive = tar::Archive::new(Cursor::new(PLUGIN_BUNDLE));
   archive.unpack(dest)?;
   Ok(())
}

fn marketplace_manifest_path(dest: &PathBuf) -> PathBuf {
   dest.join(".claude-plugin").join("marketplace.json")
}

fn load_marketplace_name(dest: &PathBuf) -> Option<String> {
   let path = marketplace_manifest_path(dest);
   let contents = fs::read_to_string(path).ok()?;
   let parsed: MarketplaceManifest = serde_json::from_str(&contents).ok()?;
   Some(parsed.name)
}

fn claude_marketplace_installed(name: &str) -> Result<bool> {
   let output = Command::new("claude")
      .args(["plugin", "marketplace", "list"])
      .output()
      .map_err(Error::ClaudeSpawn)?;

   if !output.status.success() {
      return Err(Error::ClaudeCommand(output.status.code().unwrap_or(-1)));
   }

   let stdout = String::from_utf8_lossy(&output.stdout);
   Ok(stdout.contains(&format!("❯ {name}")))
}

/// Runs a Claude CLI command with the given arguments.
fn run_claude_command(args: &[&str]) -> Result<()> {
   let status = Command::new("claude")
      .args(args)
      .stdin(Stdio::inherit())
      .stdout(Stdio::inherit())
      .stderr(Stdio::inherit())
      .status()
      .map_err(Error::ClaudeSpawn)?;

   if !status.success() {
      return Err(Error::ClaudeCommand(status.code().unwrap_or(-1)));
   }

   Ok(())
}

/// Executes the Claude Code plugin installation command.
pub fn execute() -> Result<()> {
   println!(
      "{}",
      style("Installing ggrep plugin for Claude Code...")
         .cyan()
         .bold()
   );

   let marketplace_dir = config::marketplace_dir();
   println!("Marketplace: {}", style(marketplace_dir.display()).dim());

   println!("{}", style("Extracting plugin files...").dim());
   extract_plugin_bundle(marketplace_dir)?;
   println!("{}", style("✓ Plugin files extracted").green());

   let marketplace_path = marketplace_dir.to_string_lossy();
   let marketplace_name =
      load_marketplace_name(marketplace_dir).unwrap_or_else(|| "ggrep".to_string());

   if claude_marketplace_installed(&marketplace_name)? {
      println!("{}", style("✓ Marketplace already installed").green());
   } else {
      println!("{}", style("Adding marketplace...").dim());
      match run_claude_command(&["plugin", "marketplace", "add", &marketplace_path]) {
         Ok(()) => println!("{}", style("✓ Added ggrep marketplace").green()),
         Err(e) => {
            eprintln!("{}", style(format!("✗ Failed to add marketplace: {e}")).red());
            print_troubleshooting(&marketplace_name);
            return Err(e);
         },
      }
   }

   println!("{}", style("Installing plugin...").dim());
   match run_claude_command(&["plugin", "install", "ggrep@ggrep"]) {
      Ok(()) => println!("{}", style("✓ Installed ggrep plugin").green()),
      Err(e) => {
         eprintln!("{}", style(format!("✗ Failed to install plugin: {e}")).red());
         print_troubleshooting(&marketplace_name);
         return Err(e);
      },
   }

   println!();
   println!("{}", style("Next steps:").bold());
   println!("  1. Restart Claude Code if it's running");
   println!("  2. The plugin will automatically index your project when you open it");
   println!("  3. Claude will use ggrep for semantic code search automatically");

   Ok(())
}

/// Prints troubleshooting information when installation fails.
fn print_troubleshooting(marketplace_name: &str) {
   eprintln!();
   eprintln!("{}", style("Troubleshooting:").yellow().bold());
   eprintln!("  • Ensure you have Claude Code installed");
   eprintln!("  • Try running: claude --version");
   eprintln!(
      "  • If the marketplace is stuck, try: claude plugin marketplace remove {marketplace_name}"
   );
}
