//! Unix domain socket and TCP socket abstractions for IPC

use std::{fs, io, path::PathBuf};

use sha2::{Digest, Sha256};

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

const MAX_SOCKET_PATH_LEN: usize = 100;
const SOCKET_HASH_LEN: usize = 12;

fn socket_location(store_id: &str) -> (PathBuf, String) {
   let base_dir = crate::config::socket_dir().clone();
   let stem = store_id.to_string();
   let candidate = base_dir.join(format!("{stem}.sock"));
   if candidate.to_string_lossy().len() <= MAX_SOCKET_PATH_LEN {
      return (base_dir, stem);
   }

   let hash = short_hash(store_id);
   let short_stem = format!("ggrep-{hash}");
   let tmp_dir = temp_socket_dir();
   let tmp_candidate = tmp_dir.join(format!("{short_stem}.sock"));
   if tmp_candidate.to_string_lossy().len() <= MAX_SOCKET_PATH_LEN {
      return (tmp_dir, short_stem);
   }

   (base_dir, short_stem)
}

fn temp_socket_dir() -> PathBuf {
   #[cfg(unix)]
   {
      let uid = unsafe { libc::geteuid() };
      PathBuf::from(format!("/tmp/ggrep-{uid}"))
   }

   #[cfg(not(unix))]
   {
      crate::config::socket_dir().clone()
   }
}

pub fn socket_dirs() -> Vec<PathBuf> {
   let mut dirs = vec![crate::config::socket_dir().clone()];
   let tmp = temp_socket_dir();
   if tmp != dirs[0] {
      dirs.push(tmp);
   }
   dirs
}

fn short_hash(input: &str) -> String {
   let mut hasher = Sha256::new();
   hasher.update(input.as_bytes());
   let digest = hex::encode(hasher.finalize());
   digest[..SOCKET_HASH_LEN.min(digest.len())].to_string()
}

pub fn socket_path_for(store_id: &str, ext: &str) -> PathBuf {
   let (dir, stem) = socket_location(store_id);
   dir.join(format!("{stem}.{ext}"))
}

pub fn pid_path(store_id: &str) -> PathBuf {
   socket_path_for(store_id, "pid")
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

pub fn socket_id_path(store_id: &str) -> PathBuf {
   socket_path_for(store_id, "id")
}

pub fn write_socket_id(store_id: &str) {
   let path = socket_id_path(store_id);
   if let Some(parent) = path.parent() {
      let _ = fs::create_dir_all(parent);
   }
   let _ = fs::write(path, store_id);
}

pub fn read_socket_id(path: &PathBuf) -> Option<String> {
   fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

pub fn remove_socket_id(store_id: &str) {
   let _ = fs::remove_file(socket_id_path(store_id));
}
