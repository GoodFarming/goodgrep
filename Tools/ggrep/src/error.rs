use std::{io, path::PathBuf, sync::Arc};

use regex;
use thiserror::Error;
use tree_sitter::{LanguageError, WasmError};

use crate::{embed::candle::EmbeddingError, store::lance::StoreError, usock::SocketError};
/// Main error type for the ggrep application.
///
/// This enum represents all possible errors that can occur throughout the
/// application, including I/O operations, store operations, embedding
/// operations, chunking, git operations, configuration, HTTP requests, and
/// various other domain-specific errors.
#[derive(Debug, Error)]
pub enum Error {
   /// Shared error (moka).
   #[error(transparent)]
   Shared(#[from] Arc<Self>),

   /// I/O error occurred during file or network operations.
   #[error("io error: {0}")]
   Io(#[from] io::Error),

   /// Error occurred in the store layer (e.g., database operations).
   #[error("store error: {0}")]
   Store(#[from] StoreError),

   /// Error occurred during embedding generation or processing.
   #[error("embedding error: {0}")]
   Embedding(#[from] EmbeddingError),

   /// Error occurred during code chunking operations.
   #[error("chunker error: {0}")]
   Chunker(#[from] ChunkerError),

   /// Git operation failed.
   #[error("git error: {0}")]
   Git(#[from] git2::Error),

   /// Configuration-related error occurred.
   #[error("config error: {0}")]
   Config(#[from] ConfigError),

   /// HTTP request or response error occurred.
   #[error("http error: {0}")]
   Http(#[from] HttpError),

   /// JSON serialization or deserialization error occurred.
   #[error("json error: {0}")]
   Json(#[from] serde_json::Error),

   /// TOML serialization or deserialization error occurred.
   #[error("toml error: {0}")]
   Toml(#[from] toml::de::Error),

   /// Postcard serialization or deserialization error occurred.
   #[error("postcard error: {0}")]
   Postcard(#[from] postcard::Error),

   /// `LanceDB` database operation error occurred.
   #[error("lancedb error: {0}")]
   LanceDb(#[from] lancedb::error::Error),

   /// Inter-process communication error occurred.
   #[error("ipc error: {0}")]
   Ipc(#[from] IpcError),

   /// Socket communication error occurred.
   #[error("socket error: {0}")]
   Socket(#[from] SocketError),

   /// Server error occurred during a specific operation.
   #[error("server error during {op}: {reason}")]
   Server { op: &'static str, reason: String },

   /// Unexpected response received from the server during an operation.
   #[error("unexpected response from server during {0}")]
   UnexpectedResponse(&'static str),

   /// Failed to locate the ggrep root directory.
   #[error("failed to find ggrep root directory: {0}")]
   FindRoot(#[source] io::Error),

   /// Failed to spawn the daemon process.
   #[error("failed to spawn daemon: {0}")]
   DaemonSpawn(#[source] io::Error),

   /// Failed to execute a Claude command.
   #[error("failed to run claude command: {0}")]
   ClaudeSpawn(#[source] io::Error),

   /// Claude command exited with a non-zero exit code.
   #[error("claude command exited with code {0}")]
   ClaudeCommand(i32),

   /// Failed to execute a Codex command.
   #[error("failed to run codex command: {0}")]
   CodexSpawn(#[source] io::Error),

   /// Codex command exited with a non-zero exit code.
   #[error("codex command exited with code {0}")]
   CodexCommand(i32),

   /// Failed to execute a Gemini command.
   #[error("failed to run gemini command: {0}")]
   GeminiSpawn(#[source] io::Error),

   /// Gemini command exited with a non-zero exit code.
   #[error("gemini command exited with code {0}")]
   GeminiCommand(i32),

   /// Unknown MCP (Model Context Protocol) method was requested.
   #[error("mcp unknown method: {0}")]
   McpUnknownMethod(String),

   /// Unknown MCP (Model Context Protocol) tool was requested.
   #[error("mcp unknown tool: {0}")]
   McpUnknownTool(String),

   /// Hugging Face Hub API error occurred.
   #[error("hf_hub error: {0}")]
   HfHub(#[from] hf_hub::api::tokio::ApiError),

   /// Apache Arrow schema or data error occurred.
   #[error("arrow error: {0}")]
   Arrow(#[from] arrow_schema::ArrowError),

   /// Regex compilation error occurred.
   #[error("regex error: {0}")]
   Regex(#[from] regex::Error),

   /// Failed to read the git index.
   #[error("failed to read index: {0}")]
   ReadIndex(#[source] git2::Error),

   /// Failed to get the working directory of a git repository.
   #[error("failed to get working directory: {path}", path = _0.display())]
   NoWorkingDirectory(PathBuf),

   /// Failed to open a git repository.
   #[error("failed to open repository: {0}")]
   OpenRepository(#[source] git2::Error),

   /// Path collision detected after casefold normalization.
   #[error("path collision detected (case-insensitive): {paths:?}")]
   PathCollision { paths: Vec<String> },

   /// Error already reported to the user (e.g., JSON output emitted).
   #[error("{message}")]
   Reported { message: String, exit_code: i32 },
}

impl Error {
   pub fn exit_code(&self) -> i32 {
      if let Error::Reported { exit_code, .. } = self {
         return *exit_code;
      }

      let reason = match self {
         Error::Server { reason, .. } => reason.to_lowercase(),
         Error::UnexpectedResponse(op) => op.to_string().to_lowercase(),
         other => other.to_string().to_lowercase(),
      };

      if reason.contains("busy") {
         10
      } else if reason.contains("timeout") {
         11
      } else if reason.contains("cancel") {
         12
      } else if reason.contains("incompatible") {
         13
      } else {
         1
      }
   }
}

/// Errors that can occur during inter-process communication (IPC).
///
/// These errors are related to message serialization, deserialization, and I/O
/// operations when communicating between processes.
#[derive(Debug, Error)]
pub enum IpcError {
   /// The message size exceeds the maximum allowed size.
   #[error("message too large: {0} bytes")]
   MessageTooLarge(usize),

   /// Failed to serialize a message for IPC transmission.
   #[error("failed to serialize: {0}")]
   Serialize(#[source] postcard::Error),

   /// Failed to deserialize a message received via IPC.
   #[error("failed to deserialize: {0}")]
   Deserialize(#[source] postcard::Error),

   /// Failed to read data from the IPC channel.
   #[error("failed to read: {0}")]
   Read(#[source] io::Error),

   /// Failed to write data to the IPC channel.
   #[error("failed to write: {0}")]
   Write(#[source] io::Error),
}

/// Errors that can occur during code chunking operations.
///
/// These errors are related to tree-sitter parsing, WASM store management,
/// and language grammar loading.
#[derive(Debug, Error)]
pub enum ChunkerError {
   /// Failed to create a WASM store for tree-sitter parsing.
   #[error("failed to create WASM store: {0}")]
   CreateWasmStore(#[source] WasmError),

   /// Failed to set the language for the parser.
   #[error("set language error: {0}")]
   SetLanguage(#[source] LanguageError),

   /// Failed to set the WASM store for the parser.
   #[error("set WASM store error: {0}")]
   SetWasmStore(#[source] LanguageError),

   /// Failed to load a language grammar from WASM.
   #[error("failed to load language {lang}: {reason}")]
   LoadLanguage {
      lang:   String,
      #[source]
      reason: WasmError,
   },

   /// Failed to parse the source file into an AST.
   #[error("failed to parse file")]
   ParseFailed,
}

/// Errors that can occur during configuration and grammar management.
///
/// These errors are related to user directory access, grammar downloading,
/// WASM file management, and runtime creation.
#[derive(Debug, Error)]
pub enum ConfigError {
   /// Failed to retrieve user directories (e.g., home directory, config
   /// directory).
   #[error("failed to get user directories")]
   GetUserDirectories,

   /// Repo config is invalid or exceeds safety caps.
   #[error("invalid repo config: {0}")]
   InvalidRepoConfig(String),

   /// Failed to create the grammars directory for storing language grammars.
   #[error("failed to create grammars directory: {0}")]
   CreateGrammarsDir(#[source] io::Error),

   /// Downloads are disabled (offline mode).
   #[error(
      "downloads disabled for {artifact}; run 'ggrep setup' to pre-seed caches or unset \
       GGREP_OFFLINE"
   )]
   DownloadsDisabled { artifact: String },

   /// Failed to download a language grammar from the remote source.
   #[error("failed to download {lang}: {reason}")]
   DownloadFailed {
      lang:   &'static str,
      #[source]
      reason: reqwest::Error,
   },

   /// Grammar download failed with a non-success HTTP status code.
   #[error("failed to download {lang}: HTTP {status}")]
   DownloadHttpStatus { lang: &'static str, status: u16 },

   /// Failed to read the HTTP response body during grammar download.
   #[error("failed to read response: {0}")]
   ReadResponse(#[source] reqwest::Error),

   /// Failed to create a WASM file in the grammars directory.
   #[error("failed to create WASM file: {0}")]
   CreateWasmFile(#[source] io::Error),

   /// Failed to write the downloaded WASM grammar file to disk.
   #[error("failed to write WASM file: {0}")]
   WriteWasmFile(#[source] io::Error),

   /// Failed to rename a WASM grammar file (e.g., during atomic write
   /// operations).
   #[error("failed to rename WASM file: {0}")]
   RenameWasmFile(#[source] io::Error),

   /// Failed to create the WASM runtime for executing grammar parsers.
   #[error("failed to create runtime: {0}")]
   CreateRuntime(#[source] io::Error),
}

/// Errors that can occur during HTTP operations.
///
/// These errors are related to HTTP requests, responses, and status codes.
#[derive(Debug, Error)]
pub enum HttpError {
   /// HTTP request failed (network error, timeout, etc.).
   #[error("request failed: {0}")]
   Request(#[from] reqwest::Error),

   /// Received an invalid or unexpected HTTP status code.
   #[error("invalid status code: {0}")]
   StatusCode(u16),
}

impl From<notify::Error> for Error {
   fn from(e: notify::Error) -> Self {
      Self::Io(io::Error::other(e))
   }
}

/// Standard result type using [`enum@Error`] as the default error type
pub type Result<T, E = Error> = std::result::Result<T, E>;
