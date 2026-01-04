//! CLI command implementations for ggrep.
//!
//! This module contains all subcommand implementations for the ggrep CLI tool.
//! Each module corresponds to a specific command available to users.

pub mod claude_install;
pub mod audit;
pub mod clean;
pub mod clone_store;
pub mod compact;
pub mod codex_install;
pub mod gc;
pub mod daemon;
pub mod doctor;
pub mod eval;
pub mod gemini_install;
pub mod health;
pub mod index;
pub mod list;
pub mod mcp;
pub mod opencode_install;
pub mod promote_eval;
pub mod repair;
pub mod search;
pub mod serve;
pub mod setup;
pub mod status;
pub mod stop;
pub mod stop_all;
pub mod upgrade_store;
