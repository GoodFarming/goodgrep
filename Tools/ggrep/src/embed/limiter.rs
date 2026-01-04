//! Host-wide embed concurrency limiter.
//!
//! Uses lock files under ~/.ggrep/locks to enforce a global maximum across
//! processes. Each permit corresponds to one lock file.

use std::{
   fs::{self, File, OpenOptions},
   io::Write,
   path::{Path, PathBuf},
   time::{Duration, SystemTime, UNIX_EPOCH},
};

use tokio::time;

use crate::{Result, config};

#[derive(Debug)]
pub struct EmbedPermit {
   path: PathBuf,
   #[allow(dead_code)]
   file: Option<File>,
}

impl Drop for EmbedPermit {
   fn drop(&mut self) {
      let _ = fs::remove_file(&self.path);
   }
}

#[derive(Debug, Clone, Copy)]
pub struct EmbedLimiterStatus {
   pub max_concurrent: usize,
   pub in_use:         usize,
   pub stale_lock:     bool,
}

pub async fn acquire() -> Result<Option<EmbedPermit>> {
   let cfg = config::get();
   let max = cfg.max_embed_global;
   if max == 0 {
      return Ok(None);
   }

   let lock_dir = lock_dir();
   fs::create_dir_all(&lock_dir)?;

   let ttl = Duration::from_millis(cfg.embed_lock_ttl_ms);

   loop {
      for slot in 0..max {
         let path = lock_dir.join(format!("embed-{}.lock", slot));
         match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(mut file) => {
               let _ = write_lock_metadata(&mut file);
               let _ = file.sync_all();
               return Ok(Some(EmbedPermit { path, file: Some(file) }));
            },
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
               if is_stale_lock(&path, ttl) {
                  let _ = fs::remove_file(&path);
               }
            },
            Err(e) => return Err(e.into()),
         }
      }

      time::sleep(Duration::from_millis(50)).await;
   }
}

pub fn status() -> Result<EmbedLimiterStatus> {
   let cfg = config::get();
   let max = cfg.max_embed_global;
   if max == 0 {
      return Ok(EmbedLimiterStatus {
         max_concurrent: 0,
         in_use:         0,
         stale_lock:     false,
      });
   }

   let lock_dir = lock_dir();
   let ttl = Duration::from_millis(cfg.embed_lock_ttl_ms);
   let mut in_use = 0usize;
   let mut stale_lock = false;

   for slot in 0..max {
      let path = lock_dir.join(format!("embed-{}.lock", slot));
      if path.exists() {
         in_use += 1;
         if is_stale_lock(&path, ttl) {
            stale_lock = true;
         }
      }
   }

   Ok(EmbedLimiterStatus { max_concurrent: max, in_use, stale_lock })
}

fn lock_dir() -> PathBuf {
   config::base_dir().join("locks")
}

fn write_lock_metadata(file: &mut File) -> std::io::Result<()> {
   let pid = std::process::id();
   let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_millis();
   writeln!(file, "pid={pid}")?;
   writeln!(file, "started_at_ms={now}")?;
   Ok(())
}

fn is_stale_lock(path: &Path, ttl: Duration) -> bool {
   let age = match fs::metadata(path).and_then(|m| m.modified()) {
      Ok(mtime) => SystemTime::now()
         .duration_since(mtime)
         .unwrap_or(Duration::MAX),
      Err(_) => Duration::MAX,
   };

   let pid = read_pid(path);
   if let Some(pid) = pid {
      if pid_is_alive(pid) {
         return false;
      }
   }

   age > ttl
}

fn read_pid(path: &Path) -> Option<u32> {
   let content = fs::read_to_string(path).ok()?;
   for line in content.lines() {
      if let Some(rest) = line.strip_prefix("pid=") {
         if let Ok(pid) = rest.trim().parse::<u32>() {
            return Some(pid);
         }
      }
   }
   None
}

#[cfg(target_os = "linux")]
fn pid_is_alive(pid: u32) -> bool {
   let pid = pid as libc::pid_t;
   let rc = unsafe { libc::kill(pid, 0) };
   rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(target_os = "linux"))]
fn pid_is_alive(_pid: u32) -> bool {
   false
}
