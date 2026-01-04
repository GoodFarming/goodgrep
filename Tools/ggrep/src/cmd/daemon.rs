//! Daemon connection and lifecycle management.
//!
//! Handles connecting to existing daemon processes, spawning new ones when
//! needed, and performing version handshakes to ensure compatibility.

use std::{
   path::Path,
   process::{Command, Stdio},
   time::Duration,
};

use tokio::time;

use crate::{
   Result, config,
   error::Error,
   identity,
   ipc::{self, Request, Response, SocketBuffer},
   usock,
};

/// Timeout when establishing a Unix socket connection.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// Timeout for handshake / control-plane RPCs (hello/health/shutdown).
const RPC_TIMEOUT: Duration = Duration::from_millis(5000);

/// Maximum number of connection retry attempts when waiting for daemon startup.
const RETRY_COUNT: usize = 50;
/// Delay between retry attempts.
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// Connects to a daemon instance matching the current version, spawning one if
/// needed.
///
/// First attempts to connect to an existing daemon. If successful and versions
/// match, returns the connection. Otherwise spawns a new daemon and waits for
/// it to be ready.
pub async fn connect_matching_daemon(path: &Path, store_id: &str) -> Result<usock::Stream> {
   let index_identity = identity::resolve_index_identity(path)?;
   let config_fingerprint = index_identity.config_fingerprint;

   if let Some(stream) = try_connect_existing(store_id, &config_fingerprint).await? {
      return Ok(stream);
   }

   spawn_daemon(path)?;
   wait_for_daemon(store_id, &config_fingerprint).await
}

/// Spawns a new daemon process in the background for the given path.
pub fn spawn_daemon(path: &Path) -> Result<()> {
   let exe = std::env::current_exe()?;
   let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

   Command::new(&exe)
      .arg("serve")
      .arg("--path")
      .arg(path)
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()?;

   Ok(())
}

/// Waits for a newly spawned daemon to become available and respond to
/// handshakes.
async fn wait_for_daemon(store_id: &str, config_fingerprint: &str) -> Result<usock::Stream> {
   for _ in 0..RETRY_COUNT {
      time::sleep(RETRY_DELAY).await;
      if let Some(stream) = try_connect_existing(store_id, config_fingerprint).await? {
         return Ok(stream);
      }
   }

   Err(Error::Server {
      op:     "handshake",
      reason: "daemon did not start with matching version".to_string(),
   })
}

/// Attempts to connect to an existing daemon and verify version compatibility
/// via handshake.
async fn try_connect_existing(
   store_id: &str,
   config_fingerprint: &str,
) -> Result<Option<usock::Stream>> {
   let stream = match time::timeout(CONNECT_TIMEOUT, usock::Stream::connect(store_id)).await {
      Ok(Ok(s)) => s,
      Ok(Err(_)) | Err(_) => return Ok(None),
   };

   let mut stream = stream;

   let outcome = match time::timeout(
      RPC_TIMEOUT,
      client_handshake(&mut stream, store_id, config_fingerprint, "ggrep-cli"),
   )
   .await
   {
      Ok(Ok(v)) => v,
      Ok(Err(e)) => {
         tracing::debug!("handshake failed; treating as incompatible: {}", e);
         HandshakeOutcome::Incompatible
      },
      Err(_) => {
         return Err(
            Error::Server {
               op:     "handshake",
               reason: format!("daemon unresponsive during handshake ({}s)", RPC_TIMEOUT.as_secs()),
            }
            .into(),
         );
      },
   };

   match outcome {
      HandshakeOutcome::Compatible => Ok(Some(stream)),
      HandshakeOutcome::Incompatible | HandshakeOutcome::InvalidRequest => {
         force_shutdown(Some(stream), store_id).await?;
         Ok(None)
      },
   }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum HandshakeOutcome {
   Compatible,
   Incompatible,
   InvalidRequest,
}

/// Performs a version handshake with a daemon to ensure compatibility.
pub(crate) async fn client_handshake(
   stream: &mut usock::Stream,
   store_id: &str,
   config_fingerprint: &str,
   client_role: &str,
) -> Result<HandshakeOutcome> {
   let mut buffer = SocketBuffer::new();
   let request = ipc::client_hello(
      store_id,
      config_fingerprint,
      Some(ipc::default_client_id(client_role)),
      ipc::default_client_capabilities(),
   );
   buffer.send(stream, &request).await?;
   let response: Response = buffer
      .recv_with_limit(stream, config::get().max_response_bytes)
      .await?;

   match response {
      Response::Hello {
         protocol_version,
         protocol_versions,
         store_id: server_store_id,
         config_fingerprint: server_fingerprint,
         ..
      } => {
         let client_versions = ipc::PROTOCOL_VERSIONS;
         let client_supports = client_versions.contains(&protocol_version);
         let server_supports = protocol_versions.contains(&protocol_version);
         if !client_supports || !server_supports {
            return Ok(HandshakeOutcome::Incompatible);
         }
         if server_store_id != store_id || server_fingerprint != config_fingerprint {
            return Ok(HandshakeOutcome::InvalidRequest);
         }
         Ok(HandshakeOutcome::Compatible)
      },
      Response::Error { code, message } => match code.as_str() {
         "incompatible" => Ok(HandshakeOutcome::Incompatible),
         "invalid_request" => Ok(HandshakeOutcome::InvalidRequest),
         _ => Err(Error::Server { op: "handshake", reason: format!("{code}: {message}") }.into()),
      },
      _ => Err(Error::UnexpectedResponse("handshake").into()),
   }
}

/// Forces a daemon to shut down and removes its socket.
pub async fn force_shutdown(existing: Option<usock::Stream>, store_id: &str) -> Result<()> {
   let mut buffer = SocketBuffer::new();

   if let Some(mut stream) = existing {
      let _ = time::timeout(RPC_TIMEOUT, buffer.send(&mut stream, &Request::Shutdown)).await;
      let _ = time::timeout(
         RPC_TIMEOUT,
         buffer.recv_with_limit::<_, Response>(&mut stream, config::get().max_response_bytes),
      )
      .await;
   } else if let Ok(Ok(mut stream)) =
      time::timeout(CONNECT_TIMEOUT, usock::Stream::connect(store_id)).await
   {
      let _ = time::timeout(RPC_TIMEOUT, buffer.send(&mut stream, &Request::Shutdown)).await;
      let _ = time::timeout(
         RPC_TIMEOUT,
         buffer.recv_with_limit::<_, Response>(&mut stream, config::get().max_response_bytes),
      )
      .await;
   }

   // If the daemon can't be shut down cleanly, try to terminate it using the
   // pid file so we don't leave orphaned processes behind.
   #[cfg(unix)]
   {
      if let Some(pid) = usock::read_pid(store_id)
         && looks_like_ggrep_serve(pid)
      {
         let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
      }
   }

   usock::remove_socket(store_id);
   usock::remove_pid(store_id);
   Ok(())
}

fn looks_like_ggrep_serve(pid: u32) -> bool {
   let Ok(bytes) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
      return false;
   };
   let cmdline = String::from_utf8_lossy(&bytes);
   cmdline.contains("ggrep") && cmdline.contains("serve")
}
