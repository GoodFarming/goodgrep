//! Stop server command.
//!
//! Gracefully shuts down a running daemon server for the current project.

use std::{env, path::PathBuf, time::Duration};

use console::style;
use tokio::time;

use crate::{
   Result, git,
   ipc::{self, Request, Response},
   usock,
};

fn looks_like_ggrep_serve(pid: u32) -> bool {
   let Ok(bytes) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
      return false;
   };
   let cmdline = String::from_utf8_lossy(&bytes);
   cmdline.contains("ggrep") && cmdline.contains("serve")
}

fn force_kill_if_possible(store_id: &str) -> bool {
   let Some(pid) = usock::read_pid(store_id) else {
      return false;
   };
   if !looks_like_ggrep_serve(pid) {
      return false;
   }

   #[cfg(unix)]
   {
      std::process::Command::new("kill")
         .arg("-TERM")
         .arg(pid.to_string())
         .status()
         .map(|s| s.success())
         .unwrap_or(false)
   }

   #[cfg(not(unix))]
   {
      false
   }
}

/// Executes the stop command to shut down a server.
pub async fn execute(path: Option<PathBuf>) -> Result<()> {
   const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
   const RPC_TIMEOUT: Duration = Duration::from_millis(2000);

   let root = env::current_dir()?;
   let target_path = path.unwrap_or(root);

   let store_id = git::resolve_store_id(&target_path)?;

   if !usock::socket_path(&store_id).exists() {
      println!("{}", style("No server running for this project").yellow());
      return Ok(());
   }

   let mut buffer = ipc::SocketBuffer::new();

   let stream = match time::timeout(CONNECT_TIMEOUT, usock::Stream::connect(&store_id)).await {
      Ok(Ok(s)) => Some(s),
      Ok(Err(_)) | Err(_) => None,
   };

   if let Some(mut stream) = stream {
      let sent = time::timeout(RPC_TIMEOUT, buffer.send(&mut stream, &Request::Shutdown)).await;
      if !matches!(sent, Ok(Ok(()))) {
         _ = force_kill_if_possible(&store_id);
         usock::remove_socket(&store_id);
         usock::remove_pid(&store_id);
         println!("{}", style("Server unresponsive; removed socket").yellow());
         return Ok(());
      }

      let recv = time::timeout(RPC_TIMEOUT, buffer.recv::<_, Response>(&mut stream)).await;
      match recv {
         Ok(Ok(Response::Shutdown { success: true })) => {
            println!("{}", style("Server stopped").green());
         },
         Ok(Ok(_)) => {
            println!("{}", style("Unexpected response from server").yellow());
         },
         Ok(Err(_)) | Err(_) => {
            _ = force_kill_if_possible(&store_id);
            usock::remove_socket(&store_id);
            usock::remove_pid(&store_id);
            println!("{}", style("Server unresponsive; removed socket").yellow());
         },
      }
   } else {
      _ = force_kill_if_possible(&store_id);
      usock::remove_socket(&store_id);
      usock::remove_pid(&store_id);
      println!("{}", style("Removed stale socket").yellow());
   }

   Ok(())
}
