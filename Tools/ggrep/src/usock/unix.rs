//! Unix domain socket implementation for Unix platforms

use std::{
   fs, io,
   path::PathBuf,
   pin::Pin,
   task::{self, Poll},
};

use tokio::{
   io::ReadBuf,
   net::{UnixListener as TokioUnixListener, UnixStream as TokioUnixStream},
};

use super::{
   SocketError, read_socket_id, remove_socket_id, socket_dirs, socket_path_for, write_socket_id,
};
use crate::Result;

/// Returns the socket file path for a store ID
pub fn socket_path(store_id: &str) -> PathBuf {
   socket_path_for(store_id, "sock")
}

/// Lists all running servers by checking for socket files
pub fn list_running_servers() -> Vec<String> {
   let mut servers = Vec::new();

   for dir in socket_dirs() {
      if !dir.exists() {
         continue;
      }
      let entries = fs::read_dir(dir)
         .into_iter()
         .flatten()
         .filter_map(|e| e.ok())
         .filter(|e| e.path().extension().is_some_and(|ext| ext == "sock"))
         .filter_map(|e| {
            let path = e.path();
            if let Some(id) = read_socket_id(&path.with_extension("id")) {
               return Some(id);
            }
            path.file_stem().and_then(|s| s.to_str()).map(String::from)
         });
      servers.extend(entries);
   }

   servers.sort();
   servers.dedup();
   servers
}

/// Removes the socket file for a store ID
pub fn remove_socket(store_id: &str) {
   let _ = fs::remove_file(socket_path(store_id));
   remove_socket_id(store_id);
}

/// Unix domain socket listener
pub struct Listener {
   inner: TokioUnixListener,
   path:  PathBuf,
}

impl Listener {
   /// Binds to a Unix domain socket path
   pub async fn bind(store_id: &str) -> Result<Self> {
      let path = socket_path(store_id);

      if let Some(parent) = path.parent() {
         fs::create_dir_all(parent).map_err(SocketError::CreateDir)?;
         #[cfg(unix)]
         {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
         }
      }

      if path.exists() {
         // If a daemon is listening, we must not unlink the socket file — doing
         // so can "orphan" the running daemon and cause multiple servers to
         // pile up. Treat a successful connect as "already running" and only
         // remove the socket if the connect fails (stale file, no listener).
         if Stream::connect(store_id).await.is_ok() {
            return Err(SocketError::AlreadyRunning.into());
         }
         fs::remove_file(&path).map_err(SocketError::RemoveStale)?;
      }

      let inner = TokioUnixListener::bind(&path).map_err(SocketError::Bind)?;
      #[cfg(unix)]
      {
         use std::os::unix::fs::PermissionsExt;
         fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(SocketError::Bind)?;
      }
      write_socket_id(store_id);
      Ok(Self { inner, path })
   }

   /// Accepts an incoming connection
   pub async fn accept(&self) -> Result<Stream> {
      let (stream, _) = self.inner.accept().await.map_err(SocketError::Accept)?;
      Ok(Stream { inner: stream })
   }

   /// Returns the socket path as a string
   pub fn local_addr(&self) -> String {
      self.path.display().to_string()
   }
}

impl Drop for Listener {
   fn drop(&mut self) {
      let _ = fs::remove_file(&self.path);
      let id_path = self.path.with_extension("id");
      let _ = fs::remove_file(id_path);
   }
}

/// Unix domain socket stream implementing async I/O
#[repr(transparent)]
pub struct Stream {
   inner: TokioUnixStream,
}

impl Stream {
   /// Connects to a Unix domain socket
   pub async fn connect(store_id: &str) -> Result<Self> {
      let path = socket_path(store_id);
      let inner = TokioUnixStream::connect(&path)
         .await
         .map_err(SocketError::Connect)?;
      Ok(Self { inner })
   }
}

impl tokio::io::AsyncRead for Stream {
   fn poll_read(
      mut self: Pin<&mut Self>,
      cx: &mut task::Context<'_>,
      buf: &mut ReadBuf<'_>,
   ) -> Poll<io::Result<()>> {
      Pin::new(&mut self.inner).poll_read(cx, buf)
   }
}

impl tokio::io::AsyncWrite for Stream {
   fn poll_write(
      mut self: Pin<&mut Self>,
      cx: &mut task::Context<'_>,
      buf: &[u8],
   ) -> Poll<io::Result<usize>> {
      Pin::new(&mut self.inner).poll_write(cx, buf)
   }

   fn poll_flush(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<io::Result<()>> {
      Pin::new(&mut self.inner).poll_flush(cx)
   }

   fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<io::Result<()>> {
      Pin::new(&mut self.inner).poll_shutdown(cx)
   }
}
