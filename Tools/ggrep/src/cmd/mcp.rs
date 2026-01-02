//! Model Context Protocol (MCP) server implementation.
//!
//! Implements the MCP JSON-RPC protocol to expose ggrep's semantic search
//! capabilities as a tool that can be used by Claude and other MCP clients.

use std::{
   io::Write,
   path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
   io::{AsyncBufReadExt, BufReader},
   time,
};

use crate::{
   Result,
   cmd::daemon,
   error::Error,
   git,
   ipc::{Request, Response, SocketBuffer},
   types::{SearchMode, SearchStatus},
   usock,
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

/// Connection to a ggrep daemon for executing searches.
struct DaemonConn {
   stream: usock::Stream,
   buffer: SocketBuffer,
}

impl DaemonConn {
   async fn connect(cwd: PathBuf) -> Result<Self> {
      let cwd = cwd.canonicalize()?;
      let root = git::get_repo_root(&cwd).unwrap_or_else(|| cwd.clone());
      let store_id = git::resolve_store_id(&root)?;
      let stream = daemon::connect_matching_daemon(&root, &store_id).await?;

      Ok(Self { stream, buffer: SocketBuffer::new() })
   }

   async fn search(
      &mut self,
      query: &str,
      limit: usize,
      per_file: usize,
      mode: SearchMode,
      path: Option<PathBuf>,
   ) -> Result<String> {
      let request =
         Request::Search { query: query.to_string(), limit, per_file, mode, path, rerank: true };

      self.buffer.send(&mut self.stream, &request).await?;
      let response: Response = self.buffer.recv(&mut self.stream).await?;

      match response {
         Response::Search(search_response) => {
            let scores: Vec<f32> = search_response.results.iter().map(|r| r.score).collect();
            let pcts = crate::util::compute_match_pcts(&scores);

            let mut output = String::new();
            if search_response.status == SearchStatus::Indexing {
               let p = search_response
                  .progress
                  .map_or_else(|| "?".to_string(), |v| v.to_string());
               use std::fmt::Write;
               writeln!(output, "Status: indexing {p}% (results may be incomplete)").unwrap();
               output.push('\n');
            }
            for (r, match_pct) in search_response.results.into_iter().zip(pcts.into_iter()) {
               use std::fmt::Write;

               if r.score.is_finite() {
                  writeln!(
                     output,
                     "{}:{} (match: {}%, score: {:.3})",
                     r.path.display(),
                     r.start_line,
                     match_pct.unwrap_or(0),
                     r.score
                  )
                  .unwrap();
               } else {
                  writeln!(output, "{}:{}", r.path.display(), r.start_line).unwrap();
               }
               for line in r.content.lines().take(10) {
                  writeln!(output, "  {line}").unwrap();
               }
               output.push('\n');
            }
            if output.is_empty() {
               output = format!("No results found for '{query}'");
               if search_response.status == SearchStatus::Indexing {
                  let p = search_response
                     .progress
                     .map_or_else(|| "?".to_string(), |v| v.to_string());
                  output
                     .push_str(&format!("\n\nStatus: indexing {p}% (results may be incomplete)"));
               }
            }
            Ok(output)
         },
         Response::Error { message } => Err(Error::Server { op: "search", reason: message }),
         _ => Err(Error::UnexpectedResponse("search")),
      }
   }

   async fn health(&mut self) -> Result<crate::ipc::ServerStatus> {
      self.buffer.send(&mut self.stream, &Request::Health).await?;
      let response: Response = self.buffer.recv(&mut self.stream).await?;
      match response {
         Response::Health { status } => Ok(status),
         Response::Error { message } => Err(Error::Server { op: "health", reason: message }),
         _ => Err(Error::UnexpectedResponse("health")),
      }
   }
}

/// Executes the MCP server, reading JSON-RPC requests from stdin and writing
/// responses to stdout.
pub async fn execute() -> Result<()> {
   let stdin = BufReader::new(tokio::io::stdin());
   let mut lines = stdin.lines();

   let cwd = std::env::current_dir()?;
   let mut conn: Option<DaemonConn> = None;

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
      let response = handle_request(request, &cwd, &mut conn).await;
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
   cwd: &Path,
   conn: &mut Option<DaemonConn>,
) -> Result<Value> {
   match request.method.as_str() {
      "initialize" => Ok(json!({
         "protocolVersion": "2024-11-05",
         "capabilities": {
            "tools": {}
         },
         "serverInfo": {
            "name": "ggrep",
            "version": env!("CARGO_PKG_VERSION")
         }
      })),

      "notifications/initialized" | "initialized" => Ok(Value::Null),

      "tools/list" => Ok(json!({
         "tools": [{
            "name": "good_search",
            "description": "Semantic code search. Finds code by meaning, not just text matching. Prefer this over literal search when you don't know the exact identifier or file.",
            "inputSchema": {
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
                  }
               },
               "required": ["query"]
            }
         }]
      })),

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
            "good_search" | "sem_search" => {
               let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
               let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
               let per_file = args.get("per_file").and_then(|v| v.as_u64()).unwrap_or(2) as usize;
               let mode = args
                  .get("mode")
                  .and_then(|v| v.as_str())
                  .and_then(|m| parse_mode(m).ok())
                  .unwrap_or(SearchMode::Discovery);
               let path = args
                  .get("path")
                  .and_then(|v| v.as_str())
                  .map(str::trim)
                  .filter(|s| !s.is_empty())
                  .map(PathBuf::from);

               let result =
                  do_search_with_retry(cwd.to_path_buf(), conn, query, limit, per_file, mode, path)
                     .await?;
               Ok(json!({
                  "content": [{
                     "type": "text",
                     "text": result
                  }]
               }))
            },
            _ => Err(Error::McpUnknownTool(name.to_string())),
         }
      },

      _ => Err(Error::McpUnknownMethod(request.method)),
   }
}

/// Executes a search with automatic retry on connection failure.
async fn do_search_with_retry(
   cwd: PathBuf,
   conn: &mut Option<DaemonConn>,
   query: &str,
   limit: usize,
   per_file: usize,
   mode: SearchMode,
   path: Option<PathBuf>,
) -> Result<String> {
   let timeout = std::time::Duration::from_millis(crate::config::get().worker_timeout_ms)
      .min(std::time::Duration::from_secs(55));

   let cwd = cwd.canonicalize()?;
   let root = git::get_repo_root(&cwd).unwrap_or_else(|| cwd.clone());
   let scope_path = path.map(|p| if p.is_absolute() { p } else { root.join(p) });

   let result = time::timeout(timeout, async {
      let conn_ref = ensure_conn(&cwd, conn).await?;
      conn_ref
         .search(query, limit, per_file, mode, scope_path.clone())
         .await
   })
   .await;

   match result {
      Ok(Ok(res)) => Ok(res),
      Err(_) => {
         // Drop the connection so a late daemon response can't desync the
         // request/response framing on the next tool call.
         *conn = None;

         let mut status_hint = String::new();
         if let Ok(Ok(status)) = time::timeout(std::time::Duration::from_secs(2), async {
            let mut tmp = DaemonConn::connect(cwd.clone()).await?;
            tmp.health().await
         })
         .await
         {
            status_hint = if status.indexing {
               format!(" (daemon: indexing {}%, files: {})", status.progress, status.files)
            } else {
               format!(" (daemon: ready, files: {})", status.files)
            };
         }
         Ok(format!(
            "ggrep search timed out after {}s{status_hint}; daemon may be indexing or overloaded. \
             Try again, or run `ggrep status`.",
            timeout.as_secs(),
         ))
      },
      Ok(Err(_)) => {
         // Connection failure (socket closed / broken framing / daemon died).
         // Reconnect once and retry.
         *conn = Some(DaemonConn::connect(cwd.clone()).await?);
         let conn_ref = ensure_conn(&cwd, conn).await?;
         match time::timeout(timeout, conn_ref.search(query, limit, per_file, mode, scope_path))
            .await
         {
            Ok(res) => res,
            Err(_) => {
               *conn = None;
               let mut status_hint = String::new();
               if let Ok(Ok(status)) = time::timeout(std::time::Duration::from_secs(2), async {
                  let mut tmp = DaemonConn::connect(cwd.clone()).await?;
                  tmp.health().await
               })
               .await
               {
                  status_hint = if status.indexing {
                     format!(" (daemon: indexing {}%, files: {})", status.progress, status.files)
                  } else {
                     format!(" (daemon: ready, files: {})", status.files)
                  };
               }
               Ok(format!(
                  "ggrep search timed out after {}s{status_hint}; daemon may be indexing or \
                   overloaded. Try again, or run `ggrep status`.",
                  timeout.as_secs(),
               ))
            },
         }
      },
   }
}

/// Ensures a daemon connection exists, creating one if necessary.
async fn ensure_conn<'a>(
   cwd: &Path,
   conn: &'a mut Option<DaemonConn>,
) -> Result<&'a mut DaemonConn> {
   if conn.is_none() {
      *conn = Some(DaemonConn::connect(cwd.to_path_buf()).await?);
   }
   Ok(conn.as_mut().expect("connection initialized"))
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
