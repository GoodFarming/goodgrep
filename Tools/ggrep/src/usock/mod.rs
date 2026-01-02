//! Unix domain socket and TCP socket abstractions for IPC

use std::{fs, io, path::PathBuf};

/// Errors that can occur during socket operations
#[derive(Debug, thiserror::Error)]
pub enum SocketError {
   #[error("server already running")]
   AlreadyRunning,

   #[error("failed to connect: {0}")]
   Connect(#[source] io::Error),

   #[error("failed to bind: {0}")]
   Bind(#[source] io::Error),

   #[error("accept failed: {0}")]
   Accept(#[source] io::Error),

   #[error("failed to create socket directory: {0}")]
   CreateDir(#[source] io::Error),

   #[error("failed to remove stale socket: {0}")]
   RemoveStale(#[source] io::Error),

   #[error("failed to read port file: {0}")]
   ReadPort(#[source] io::Error),

   #[error("invalid port in port file: {0}")]
   InvalidPort(#[source] io::Error),

   #[error("failed to write port file: {0}")]
   WritePort(#[source] io::Error),
}

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

#[cfg(not(unix))]
mod tcp;
#[cfg(not(unix))]
pub use tcp::*;

pub fn pid_path(store_id: &str) -> PathBuf {
   crate::config::socket_dir().join(format!("{store_id}.pid"))
}

pub fn write_pid(store_id: &str) {
   let path = pid_path(store_id);
   if let Some(parent) = path.parent() {
      let _ = fs::create_dir_all(parent);
   }
   let _ = fs::write(path, format!("{}", std::process::id()));
}

pub fn read_pid(store_id: &str) -> Option<u32> {
   let text = fs::read_to_string(pid_path(store_id)).ok()?;
   text.trim().parse::<u32>().ok()
}

pub fn remove_pid(store_id: &str) {
   let _ = fs::remove_file(pid_path(store_id));
}
