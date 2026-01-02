//! Server status command.
//!
//! Displays the status of all running ggrep daemon servers.

use std::time::Duration;

use console::style;
use tokio::time;

use crate::{
   Result,
   ipc::{self, Request, Response},
   usock,
};

/// Executes the status command to show running servers.
pub async fn execute() -> Result<()> {
   const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
   const RPC_TIMEOUT: Duration = Duration::from_millis(2000);

   let servers = usock::list_running_servers();

   if servers.is_empty() {
      println!("{}", style("No servers running").dim());
      return Ok(());
   }

   println!("{}", style("Running servers:").bold());
   println!();

   let mut buffer = ipc::SocketBuffer::new();
   for store_id in servers {
      let stream = match time::timeout(CONNECT_TIMEOUT, usock::Stream::connect(&store_id)).await {
         Ok(Ok(s)) => s,
         Ok(Err(_)) | Err(_) => {
            println!("  {} {} {}", style("●").red(), store_id, style("(stale)").dim());
            continue;
         },
      };

      let mut stream = stream;

      let sent = time::timeout(RPC_TIMEOUT, buffer.send(&mut stream, &Request::Health)).await;
      if !matches!(sent, Ok(Ok(()))) {
         println!("  {} {} {}", style("●").yellow(), store_id, style("(unresponsive)").dim());
         continue;
      }

      let recv = time::timeout(RPC_TIMEOUT, buffer.recv::<_, Response>(&mut stream)).await;
      match recv {
         Ok(Ok(Response::Health { status })) => {
            let state = if status.indexing {
               format!("indexing {}%", status.progress)
            } else {
               "ready".to_string()
            };
            println!(
               "  {} {} {}",
               style("●").green(),
               store_id,
               style(format!("({state}, files: {})", status.files)).dim()
            );
         },
         Ok(Ok(_)) => {
            println!("  {} {} {}", style("●").yellow(), store_id, style("(unknown)").dim());
         },
         Ok(Err(_)) | Err(_) => {
            println!("  {} {} {}", style("●").yellow(), store_id, style("(unresponsive)").dim());
         },
      }
   }

   Ok(())
}
