//! Model Context Protocol (MCP) server implementation.
//!
//! Implements the MCP JSON-RPC protocol to expose ggrep's semantic search
//! capabilities as a tool that can be used by Claude and other MCP clients.

use std::{
   io::Write,
   path::PathBuf,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

use crate::{
   Result,
   cmd::{daemon, health, search, status},
   config,
   error::Error,
   file::{normalize_path, normalize_relative},
   identity,
   types::SearchMode,
};

/// Incoming JSON-RPC 2.0 request from an MCP client.
#[derive(Deserialize)]
struct JsonRpcRequest {
   #[allow(dead_code, reason = "jsonrpc field is required by JSON-RPC spec but not used in code")]
   jsonrpc: String,
   id:      Option<Value>,
   method:  String,
   #[serde(default)]
   params:  Value,
}

/// Outgoing JSON-RPC 2.0 response to an MCP client.
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
   jsonrpc: &'static str,
   id:      Value,
   #[serde(skip_serializing_if = "Option::is_none")]
   result:  Option<Value>,
   #[serde(skip_serializing_if = "Option::is_none")]
   error:   Option<JsonRpcError>,
}

/// JSON-RPC error object.
#[derive(Debug, Serialize)]
struct JsonRpcError {
   code:    i32,
   message: String,
}

impl JsonRpcResponse {
   const fn success(id: Value, result: Value) -> Self {
      Self { jsonrpc: "2.0", id, result: Some(result), error: None }
   }

   const fn error(id: Value, code: i32, message: String) -> Self {
      Self { jsonrpc: "2.0", id, result: None, error: Some(JsonRpcError { code, message }) }
   }
}

struct McpState {
   startup_cwd: PathBuf,
   workspace_root: Option<PathBuf>,
}

/// Executes the MCP server, reading JSON-RPC requests from stdin and writing
/// responses to stdout.
pub async fn execute() -> Result<()> {
   let stdin = BufReader::new(tokio::io::stdin());
   let mut lines = stdin.lines();

   let startup_cwd = std::env::current_dir()?;
   let mut state = McpState { startup_cwd, workspace_root: None };

   while let Some(line) = lines.next_line().await? {
      if line.is_empty() {
         continue;
      }

      let request: JsonRpcRequest = match serde_json::from_str(&line) {
         Ok(r) => r,
         Err(e) => {
            let response = JsonRpcResponse::error(Value::Null, -32700, format!("Parse error: {e}"));
            write_response(&response)?;
            continue;
         },
      };

      let id = request.id.clone();
      let response = handle_request(request, &mut state).await;
      // JSON-RPC notifications (no `id`) must not receive a response. Some MCP
      // clients will treat responses-to-notifications as a protocol error and
      // close the transport.
      if let Some(id) = id {
         let response = match response {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(e) => JsonRpcResponse::error(id, -32603, e.to_string()),
         };
         write_response(&response)?;
      } else if let Err(e) = response {
         tracing::debug!("MCP notification error: {}", e);
      }
   }

   Ok(())
}

/// Writes a JSON-RPC response to stdout.
fn write_response(response: &JsonRpcResponse) -> Result<()> {
   let stdout = std::io::stdout();
   let mut stdout = stdout.lock();
   serde_json::to_writer(&mut stdout, response)?;
   stdout.write_all(b"\n")?;
   stdout.flush()?;
   Ok(())
}

/// Handles an incoming JSON-RPC request and returns the result value.
async fn handle_request(
   request: JsonRpcRequest,
   state: &mut McpState,
) -> Result<Value> {
   match request.method.as_str() {
      "initialize" => {
         if let Some(root) = parse_initialize_root(&request.params) {
            state.workspace_root = root.canonicalize().ok().or(Some(root));
         }

         Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
               "tools": {},
               "resources": {}
            },
            "serverInfo": {
               "name": "ggrep",
               "version": env!("CARGO_PKG_VERSION")
            }
         }))
      },

      "notifications/initialized" | "initialized" => Ok(Value::Null),

      "tools/list" => {
         let search_input_schema = json!({
            "type": "object",
            "properties": {
               "query": {
                  "type": "string",
                  "description": "Natural language query describing what you're looking for"
               },
               "limit": {
                  "type": "integer",
                  "description": "Maximum number of results (default: 10)",
                  "default": 10
               },
               "per_file": {
                  "type": "integer",
                  "description": "Maximum results per file (default: 2)",
                  "default": 2
               },
               "mode": {
                  "type": "string",
                  "description": "Search mode: balanced|discovery|implementation|planning|debug (default: discovery)",
                  "default": "discovery"
               },
               "path": {
                  "type": "string",
                  "description": "Optional directory scope (relative to repo root or absolute). Example: \"CRM\".",
                  "default": ""
               },
               "repo_root": {
                  "type": "string",
                  "description": "Optional repo root (absolute, or relative to workspace). Defaults to MCP workspace root (or server startup cwd).",
                  "default": ""
               },
               "explain": {
                  "type": "boolean",
                  "description": "Include explainability metadata (candidate mix).",
                  "default": false
               },
               "rerank": {
                  "type": "boolean",
                  "description": "Enable ColBERT reranking (default: true).",
                  "default": true
               }
            },
            "required": ["query"]
         });
         let search_input_schema_clone = search_input_schema.clone();

         Ok(json!({
            "tools": [{
               "name": "search",
               "description": "Semantic code search. Returns the same JSON schema as `ggrep search --json` (query_success/query_error).",
               "inputSchema": search_input_schema_clone
            }, {
               "name": "good_search",
               "description": "Deprecated alias of `search` (kept for backwards compatibility). Returns the same JSON schema as `ggrep search --json` (query_success/query_error).",
               "inputSchema": search_input_schema
            }, {
               "name": "ggrep_status",
               "description": "Returns `ggrep status --json` for the selected repo (or MCP workspace root).",
               "inputSchema": {
                  "type": "object",
                  "properties": {
                     "repo_root": {
                        "type": "string",
                        "description": "Optional repo root (absolute, or relative to workspace). Defaults to MCP workspace root (or server startup cwd).",
                        "default": ""
                     }
                  }
               }
            }, {
               "name": "ggrep_health",
               "description": "Returns `ggrep health --json` for the selected repo (or MCP workspace root).",
               "inputSchema": {
                  "type": "object",
                  "properties": {
                     "repo_root": {
                        "type": "string",
                        "description": "Optional repo root (absolute, or relative to workspace). Defaults to MCP workspace root (or server startup cwd).",
                        "default": ""
                     }
                  }
               }
            }]
         }))
      },

      "tools/call" => {
         let name = request
            .params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
         let args = request
            .params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

         match name {
            "search" | "good_search" | "sem_search" => {
               Ok(tool_good_search(state, &args).await)
            },
            "ggrep_status" => Ok(tool_status(state, &args).await),
            "ggrep_health" => Ok(tool_health(state, &args).await),
            _ => Err(Error::McpUnknownTool(name.to_string())),
         }
      },

      "resources/list" => Ok(json!({
         "resources": [{
            "uri": "ggrep://status",
            "name": "ggrep status",
            "description": "Repository status (same schema as `ggrep status --json`).",
            "mimeType": "application/json"
         }, {
            "uri": "ggrep://health",
            "name": "ggrep health",
            "description": "Repository health (same schema as `ggrep health --json`).",
            "mimeType": "application/json"
         }]
      })),

      "resources/read" => {
         let uri = request.params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
         match uri {
            "ggrep://status" => {
               let base = default_repo_root(state);
               let text = status::collect_status_json(&base, false).await?;
               Ok(json!({
                  "contents": [{
                     "uri": uri,
                     "mimeType": "application/json",
                     "text": text
                  }]
               }))
            },
            "ggrep://health" => {
               let base = default_repo_root(state);
               let text = health::collect_health_json(&base, false).await?;
               Ok(json!({
                  "contents": [{
                     "uri": uri,
                     "mimeType": "application/json",
                     "text": text
                  }]
               }))
            },
            _ => Err(Error::McpUnknownMethod(format!("resources/read:{uri}"))),
         }
      },

      _ => Err(Error::McpUnknownMethod(request.method)),
   }
}

fn tool_ok(text: String) -> Value {
   json!({
      "content": [{
         "type": "text",
         "text": text
      }]
   })
}

fn tool_err(text: String) -> Value {
   json!({
      "content": [{
         "type": "text",
         "text": text
      }],
      "isError": true
   })
}

async fn tool_good_search(state: &McpState, args: &Value) -> Value {
   let request_id = Uuid::new_v4().to_string();
   match try_tool_good_search(state, args, &request_id).await {
      Ok(text) => tool_ok(text),
      Err(err) => {
         let payload = search::build_json_error(&err, request_id.as_str());
         let text = serde_json::to_string(&payload)
            .unwrap_or_else(|_| r#"{"error":{"code":"internal","message":"serialization failed"}}"#.to_string());
         tool_err(text)
      },
   }
}

async fn try_tool_good_search(state: &McpState, args: &Value, request_id: &str) -> Result<String> {
   let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").trim();
   if query.is_empty() {
      return Err(Error::Server { op: "search", reason: "invalid_request: query is required".to_string() });
   }

   let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
   let per_file = args.get("per_file").and_then(|v| v.as_u64()).unwrap_or(2) as usize;
   let mode = args
      .get("mode")
      .and_then(|v| v.as_str())
      .and_then(|m| parse_mode(m).ok())
      .unwrap_or(SearchMode::Discovery);
   let rerank = args.get("rerank").and_then(|v| v.as_bool()).unwrap_or(true);

   let repo_root_arg = args.get("repo_root").and_then(|v| v.as_str()).map(str::trim);
   let base = resolve_repo_root(state, repo_root_arg)?;

   let scope_arg = args.get("path").and_then(|v| v.as_str()).map(str::trim);
   let filter_path = match scope_arg {
      Some(scope) if !scope.is_empty() => {
         let scope_path = PathBuf::from(scope);
         if scope_path.is_absolute() {
            scope_path
         } else {
            base.join(scope_path)
         }
      },
      _ => base.clone(),
   };

   let filter_path = filter_path.canonicalize().map_err(|e| Error::Server {
      op: "search",
      reason: format!("invalid_request: invalid path {}: {e}", filter_path.display()),
   })?;

   let index_identity = identity::resolve_index_identity(&filter_path)?;
   let index_root = index_identity.canonical_root.clone();
   let store_id = index_identity.store_id.clone();

   let scope_rel = if filter_path != index_root {
      let rel = filter_path
         .strip_prefix(&index_root)
         .ok()
         .and_then(normalize_relative)
         .unwrap_or_else(|| PathBuf::from(normalize_path(&filter_path)));
      Some(rel)
   } else {
      None
   };

   let cfg = config::get();
   let capped_limit = limit.min(cfg.max_query_results).max(1);
   let capped_per_file = per_file.min(cfg.max_query_per_file).max(1);

   let stream = daemon::connect_matching_daemon(&index_root, &store_id).await?;
   let outcome = search::send_search_request(
      stream,
      query,
      capped_limit,
      capped_per_file,
      mode,
      rerank,
      scope_rel.as_deref(),
      &index_root,
   )
   .await?;

   let meta = search::build_meta(
      query,
      &index_identity,
      &store_id,
      scope_rel.as_deref(),
      search::SnippetMode::Default,
      capped_limit,
      capped_per_file,
      rerank,
      mode,
      request_id,
      &outcome,
   )?;

   let explain = args.get("explain").and_then(|v| v.as_bool()).unwrap_or(false);
   let explain = explain.then(|| search::build_explain(&meta, &outcome));

   let payload = search::build_json_output(meta, outcome, explain);
   Ok(serde_json::to_string(&payload)?)
}

async fn tool_status(state: &McpState, args: &Value) -> Value {
   let repo_root_arg = args.get("repo_root").and_then(|v| v.as_str()).map(str::trim);
   let base = match resolve_repo_root(state, repo_root_arg) {
      Ok(p) => p,
      Err(e) => return tool_err(e.to_string()),
   };
   match status::collect_status_json(&base, false).await {
      Ok(text) => tool_ok(text),
      Err(e) => tool_err(e.to_string()),
   }
}

async fn tool_health(state: &McpState, args: &Value) -> Value {
   let repo_root_arg = args.get("repo_root").and_then(|v| v.as_str()).map(str::trim);
   let base = match resolve_repo_root(state, repo_root_arg) {
      Ok(p) => p,
      Err(e) => return tool_err(e.to_string()),
   };
   match health::collect_health_json(&base, false).await {
      Ok(text) => tool_ok(text),
      Err(e) => tool_err(e.to_string()),
   }
}

fn default_repo_root(state: &McpState) -> PathBuf {
   state
      .workspace_root
      .clone()
      .unwrap_or_else(|| state.startup_cwd.clone())
}

fn resolve_repo_root(state: &McpState, repo_root: Option<&str>) -> Result<PathBuf> {
   let base = default_repo_root(state);
   let Some(repo_root) = repo_root else {
      return Ok(base);
   };
   let repo_root = repo_root.trim();
   if repo_root.is_empty() {
      return Ok(base);
   }
   let p = PathBuf::from(repo_root);
   Ok(if p.is_absolute() { p } else { base.join(p) })
}

fn parse_initialize_root(params: &Value) -> Option<PathBuf> {
   if let Some(uri) = params.get("rootUri").and_then(|v| v.as_str()) {
      if let Some(p) = decode_file_uri(uri) {
         return Some(p);
      }
   }
   let folders = params.get("workspaceFolders").and_then(|v| v.as_array())?;
   let first = folders.first()?;
   let uri = first.get("uri").and_then(|v| v.as_str())?;
   decode_file_uri(uri)
}

fn decode_file_uri(uri: &str) -> Option<PathBuf> {
   let rest = uri.strip_prefix("file://")?;
   if rest.is_empty() {
      return None;
   }
   Some(PathBuf::from(percent_decode(rest)))
}

fn percent_decode(input: &str) -> String {
   let bytes = input.as_bytes();
   let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
   let mut i = 0;
   while i < bytes.len() {
      if bytes[i] == b'%' && i + 2 < bytes.len() {
         let hi = from_hex(bytes[i + 1]);
         let lo = from_hex(bytes[i + 2]);
         if let (Some(hi), Some(lo)) = (hi, lo) {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
         }
      }
      out.push(bytes[i]);
      i += 1;
   }
   String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(byte: u8) -> Option<u8> {
   match byte {
      b'0'..=b'9' => Some(byte - b'0'),
      b'a'..=b'f' => Some(10 + (byte - b'a')),
      b'A'..=b'F' => Some(10 + (byte - b'A')),
      _ => None,
   }
}

fn parse_mode(mode: &str) -> std::result::Result<SearchMode, String> {
   match mode.trim().to_ascii_lowercase().as_str() {
      "balanced" => Ok(SearchMode::Balanced),
      "discovery" => Ok(SearchMode::Discovery),
      "implementation" | "impl" => Ok(SearchMode::Implementation),
      "planning" | "plan" => Ok(SearchMode::Planning),
      "debug" => Ok(SearchMode::Debug),
      other => Err(format!(
         "invalid mode '{other}' (expected: balanced|discovery|implementation|planning|debug)"
      )),
   }
}
