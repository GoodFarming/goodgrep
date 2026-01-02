//! Stop all servers command.
//!
//! Gracefully shuts down all running ggrep daemon servers.

use std::time::Duration;

use console::style;
use tokio::time;

use crate::{
   Result,
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

/// Executes the stop-all command to shut down all running servers.
pub async fn execute() -> Result<()> {
   const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
   const RPC_TIMEOUT: Duration = Duration::from_millis(2000);

   let servers = usock::list_running_servers();

   if servers.is_empty() {
      println!("{}", style("No servers running").yellow());
      return Ok(());
   }

   let mut stopped = 0;
   let mut failed = 0;

   for store_id in servers {
      let stream = match time::timeout(CONNECT_TIMEOUT, usock::Stream::connect(&store_id)).await {
         Ok(Ok(s)) => Some(s),
         Ok(Err(_)) | Err(_) => None,
      };

      if let Some(mut stream) = stream {
         let mut buffer = ipc::SocketBuffer::new();
         let sent = time::timeout(RPC_TIMEOUT, buffer.send(&mut stream, &Request::Shutdown)).await;
         if !matches!(sent, Ok(Ok(()))) {
            _ = force_kill_if_possible(&store_id);
            usock::remove_socket(&store_id);
            usock::remove_pid(&store_id);
            stopped += 1;
            continue;
         }

         match time::timeout(RPC_TIMEOUT, buffer.recv::<_, Response>(&mut stream)).await {
            Ok(Ok(Response::Shutdown { success: true })) => stopped += 1,
            Ok(Ok(_)) => failed += 1,
            Ok(Err(_)) | Err(_) => {
               _ = force_kill_if_possible(&store_id);
               usock::remove_socket(&store_id);
               usock::remove_pid(&store_id);
               stopped += 1;
            },
         }
      } else {
         _ = force_kill_if_possible(&store_id);
         usock::remove_socket(&store_id);
         usock::remove_pid(&store_id);
         stopped += 1;
      }
   }

   println!("{}", style(format!("Stopped {stopped} servers, {failed} failed")).green());

   Ok(())
}
