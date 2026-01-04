//! IPC protocol for client-server communication over sockets

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
   Result,
   error::IpcError,
   types::{SearchMode, SearchResponse},
};

pub const PROTOCOL_VERSIONS: &[u32] = &[2];
const SCHEMA_VERSION_QUERY_SUCCESS: u32 = 1;
const SCHEMA_VERSION_QUERY_ERROR: u32 = 1;
const SCHEMA_VERSION_STATUS: u32 = 1;
const SCHEMA_VERSION_HEALTH: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupportedSchemaVersions {
   pub query_success: Vec<u32>,
   pub query_error:   Vec<u32>,
   pub status:        Vec<u32>,
   pub health:        Vec<u32>,
}

impl SupportedSchemaVersions {
   pub fn current() -> Self {
      Self {
         query_success: vec![SCHEMA_VERSION_QUERY_SUCCESS],
         query_error:   vec![SCHEMA_VERSION_QUERY_ERROR],
         status:        vec![SCHEMA_VERSION_STATUS],
         health:        vec![SCHEMA_VERSION_HEALTH],
      }
   }
}

pub fn negotiate_protocol(client_versions: &[u32]) -> Option<u32> {
   let mut best: Option<u32> = None;
   for &version in client_versions {
      if PROTOCOL_VERSIONS.contains(&version) {
         best = Some(best.map_or(version, |current| current.max(version)));
      }
   }
   best
}

pub fn default_client_id(role: &str) -> String {
   std::env::var("GGREP_CLIENT_ID").unwrap_or_else(|_| format!("{role}-{}", std::process::id()))
}

pub fn default_client_capabilities() -> Vec<String> {
   vec!["json".to_string(), "explain".to_string()]
}

pub fn client_hello(
   store_id: &str,
   config_fingerprint: &str,
   client_id: Option<String>,
   client_capabilities: Vec<String>,
) -> Request {
   Request::Hello {
      protocol_versions: PROTOCOL_VERSIONS.to_vec(),
      store_id: store_id.to_string(),
      config_fingerprint: config_fingerprint.to_string(),
      client_id,
      client_capabilities,
   }
}

/// Client request messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
   Hello {
      protocol_versions:   Vec<u32>,
      store_id:            String,
      config_fingerprint:  String,
      client_id:           Option<String>,
      client_capabilities: Vec<String>,
   },
   Search {
      query:    String,
      limit:    usize,
      per_file: usize,
      mode:     SearchMode,
      path:     Option<PathBuf>,
      rerank:   bool,
   },
   Health,
   Gc {
      dry_run: bool,
   },
   Shutdown,
}

/// Server response messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
   Hello {
      protocol_version:          u32,
      protocol_versions:         Vec<u32>,
      binary_version:            String,
      supported_schema_versions: SupportedSchemaVersions,
      store_id:                  String,
      config_fingerprint:        String,
   },
   Search(SearchResponse),
   Health {
      status: ServerStatus,
   },
   Gc {
      report: crate::snapshot::GcReport,
   },
   Shutdown {
      success: bool,
   },
   Error {
      code:    String,
      message: String,
   },
}

/// Server health status information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStatus {
   pub indexing:          bool,
   pub progress:          u8,
   pub files:             usize,
   pub queries_in_flight: usize,
   pub queries_queued:    usize,
   pub busy_total:        u64,
   pub timeouts_total:    u64,
   pub slow_total:        u64,
   pub query_latency_p50_ms: u64,
   pub query_latency_p95_ms: u64,
   pub segments_touched_max: u64,
   pub segments_open:     u64,
   pub segments_budget:   u64,
}

/// Stack-allocated buffer for socket I/O operations
pub struct SocketBuffer {
   buf: SmallVec<[u8; 2048]>,
}

const DEFAULT_MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

impl Extend<u8> for &mut SocketBuffer {
   fn extend<I: IntoIterator<Item = u8>>(&mut self, iter: I) {
      self.buf.extend(iter);
   }
}

impl Default for SocketBuffer {
   fn default() -> Self {
      Self::new()
   }
}

impl SocketBuffer {
   pub fn new() -> Self {
      Self { buf: SmallVec::new() }
   }

   #[allow(
      clippy::future_not_send,
      reason = "Generic async function with references - Send bound would be too restrictive for \
                trait"
   )]
   /// Serializes and sends a message with length prefix
   pub async fn send<W, T>(&mut self, writer: &mut W, msg: &T) -> Result<()>
   where
      W: AsyncWrite + Unpin,
      T: Serialize,
   {
      self.buf.clear();
      self.buf.resize(4, 0u8);
      _ = postcard::to_extend(msg, &mut *self).map_err(IpcError::Serialize)?;
      let payload_len = (self.buf.len() - 4) as u32;
      *self.buf.first_chunk_mut().unwrap() = payload_len.to_le_bytes();
      writer.write_all(&self.buf).await.map_err(IpcError::Write)?;
      writer.flush().await.map_err(IpcError::Write)?;
      Ok(())
   }

   /// Receives and deserializes a message with length prefix
   pub async fn recv<'de, R, T>(&'de mut self, reader: &mut R) -> Result<T>
   where
      R: AsyncRead + Unpin,
      T: Deserialize<'de>,
   {
      self
         .recv_with_limit(reader, DEFAULT_MAX_MESSAGE_BYTES)
         .await
   }

   pub async fn recv_with_limit<'de, R, T>(
      &'de mut self,
      reader: &mut R,
      max_len: usize,
   ) -> Result<T>
   where
      R: AsyncRead + Unpin,
      T: Deserialize<'de>,
   {
      let mut len_buf = [0u8; 4];
      reader
         .read_exact(&mut len_buf)
         .await
         .map_err(IpcError::Read)?;
      let len = u32::from_le_bytes(len_buf) as usize;

      if len > max_len {
         return Err(IpcError::MessageTooLarge(len).into());
      }

      self.buf.resize(len, 0u8);
      reader
         .read_exact(self.buf.as_mut_slice())
         .await
         .map_err(IpcError::Read)?;
      postcard::from_bytes(&self.buf).map_err(|e| IpcError::Deserialize(e).into())
   }
}
