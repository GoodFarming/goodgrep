//! Utility functions for filesystem operations

use std::{
   fs::{self, File, OpenOptions},
   io::Write,
   path::{Path, PathBuf},
   time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use fs4::FileExt;
use tokio::time;

use crate::{Result, error::Error};

/// Converts raw relevance scores into a stable, relative 0–99 "match %" scale.
///
/// The goal is a simple, human/agent-friendly indicator that:
/// - is monotonic with the raw score,
/// - is comparable *within a single result set* (not globally calibrated),
/// - does not always force the top hit to 100%.
pub fn compute_match_pcts(scores: &[f32]) -> Vec<Option<u8>> {
   if scores.is_empty() {
      return Vec::new();
   }

   let finite: Vec<f64> = scores
      .iter()
      .copied()
      .filter(|s| s.is_finite())
      .map(|s| s as f64)
      .collect();

   // Fallback for degenerate cases (0–1 finite scores): show a bounded absolute
   // scale so the UI can still surface something.
   if finite.len() < 2 {
      return scores
         .iter()
         .map(|&s| {
            if !s.is_finite() {
               return None;
            }
            let pct = (s as f64 * 100.0).round().clamp(0.0, 99.0) as u8;
            Some(pct)
         })
         .collect();
   }

   let mean = finite.iter().sum::<f64>() / finite.len() as f64;
   let var = finite
      .iter()
      .map(|v| {
         let d = v - mean;
         d * d
      })
      .sum::<f64>()
      / finite.len() as f64;
   let std = var.sqrt();

   if !std.is_finite() || std <= 1e-9 {
      return scores
         .iter()
         .map(|&s| {
            if !s.is_finite() {
               return None;
            }
            let pct = (s as f64 * 100.0).round().clamp(0.0, 99.0) as u8;
            Some(pct)
         })
         .collect();
   }

   scores
      .iter()
      .map(|&s| {
         if !s.is_finite() {
            return None;
         }
         let z = (s as f64 - mean) / std;
         let sigmoid = 1.0 / (1.0 + (-z).exp());
         let pct = (sigmoid * 100.0).round().clamp(0.0, 99.0) as u8;
         Some(pct)
      })
      .collect()
}

/// Recursively calculates the total size of a directory in bytes
pub fn get_dir_size(path: &Path) -> Result<u64> {
   let mut total = 0u64;

   if path.is_dir() {
      for entry in fs::read_dir(path)? {
         let entry = entry?;
         let metadata = entry.metadata()?;

         if metadata.is_dir() {
            total += get_dir_size(&entry.path())?;
         } else {
            total += metadata.len();
         }
      }
   }

   Ok(total)
}

/// Formats a byte count as a human-readable size string
pub fn format_size(bytes: u64) -> String {
   const KB: u64 = 1024;
   const MB: u64 = KB * 1024;
   const GB: u64 = MB * 1024;

   if bytes < KB {
      format!("{bytes} B")
   } else if bytes < MB {
      format!("{:.1} KB", bytes as f64 / KB as f64)
   } else if bytes < GB {
      format!("{:.1} MB", bytes as f64 / MB as f64)
   } else {
      format!("{:.1} GB", bytes as f64 / GB as f64)
   }
}

#[cfg(feature = "failpoints")]
pub fn fail_point(name: &str) -> Result<()> {
   fail::fail_point!(name, |_| {
      Err(Error::Server { op: "failpoint", reason: name.to_string() }.into())
   });
   Ok(())
}

#[cfg(not(feature = "failpoints"))]
pub fn fail_point(_name: &str) -> Result<()> {
   Ok(())
}

/// Strips control characters and ANSI escape sequences from output strings.
pub fn sanitize_output(input: &str) -> String {
   let mut out = String::with_capacity(input.len());
   let mut chars = input.chars().peekable();
   while let Some(ch) = chars.next() {
      if ch == '\u{1b}' {
         if matches!(chars.peek(), Some('[')) {
            let _ = chars.next();
            while let Some(c) = chars.next() {
               if ('@'..='~').contains(&c) {
                  break;
               }
            }
         }
         continue;
      }
      if ch.is_control() && ch != '\n' && ch != '\t' {
         continue;
      }
      out.push(ch);
   }
   out
}

pub struct ArtifactLock {
   file: File,
}

pub fn probe_store_path(path: &Path) -> Result<()> {
   ensure_local_filesystem(path)?;

   fs::create_dir_all(path)?;
   let probe_dir = probe_dir(path);
   fs::create_dir_all(&probe_dir)?;

   let exclusive_path = probe_dir.join("exclusive.lock");
   let mut file = OpenOptions::new()
      .create_new(true)
      .write(true)
      .open(&exclusive_path)?;
   file.write_all(b"probe")?;
   file.sync_all()?;

   let tmp_path = probe_dir.join("rename.tmp");
   let final_path = probe_dir.join("rename.final");
   fs::write(&tmp_path, b"probe")?;
   fs::rename(&tmp_path, &final_path)?;
   let contents = fs::read(&final_path)?;
   if contents != b"probe" {
      let _ = fs::remove_dir_all(&probe_dir);
      return Err(
         Error::Server { op: "fs_probe", reason: "rename/read-after-write failed".to_string() }
            .into(),
      );
   }

   let _ = fs::remove_dir_all(&probe_dir);
   Ok(())
}

pub fn fsync_dir(path: &Path) -> Result<()> {
   let dir = File::open(path)?;
   dir.sync_all()?;
   Ok(())
}

fn ensure_local_filesystem(path: &Path) -> Result<()> {
   #[cfg(target_os = "linux")]
   {
      let fs_type = statfs_type(path)?;
      if is_network_fs(fs_type) {
         return Err(
            Error::Server {
               op:     "fs_probe",
               reason: format!("refusing network filesystem type {:#x}", fs_type),
            }
            .into(),
         );
      }
   }
   Ok(())
}

#[cfg(target_os = "linux")]
fn statfs_type(path: &Path) -> Result<i64> {
   use std::ffi::CString;
   let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| Error::Server {
      op:     "fs_probe",
      reason: "invalid path for statfs".to_string(),
   })?;
   let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
   let rc = unsafe { libc::statfs(c_path.as_ptr(), &mut stat) };
   if rc != 0 {
      return Err(std::io::Error::last_os_error().into());
   }
   Ok(stat.f_type as i64)
}

#[cfg(target_os = "linux")]
fn is_network_fs(fs_type: i64) -> bool {
   const NFS_SUPER_MAGIC: i64 = 0x6969;
   const CIFS_SUPER_MAGIC: i64 = 0xff534d42u32 as i64;
   const SMB_SUPER_MAGIC: i64 = 0x517bu32 as i64;
   const SMB2_SUPER_MAGIC: i64 = 0xfe534d42u32 as i64;
   matches!(fs_type, NFS_SUPER_MAGIC | CIFS_SUPER_MAGIC | SMB_SUPER_MAGIC | SMB2_SUPER_MAGIC)
}

fn probe_dir(base: &Path) -> PathBuf {
   let ts = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_nanos())
      .unwrap_or(0);
   base.join(format!(".ggrep-probe-{}-{}", std::process::id(), ts))
}

impl ArtifactLock {
   pub async fn acquire(path: &Path, timeout: Duration) -> Result<Self> {
      if let Some(parent) = path.parent() {
         fs::create_dir_all(parent)?;
      }

      let file = OpenOptions::new()
         .create(true)
         .read(true)
         .write(true)
         .open(path)?;

      let start = Instant::now();
      loop {
         match file.try_lock_exclusive() {
            Ok(()) => return Ok(Self { file }),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
               if start.elapsed() >= timeout {
                  return Err(
                     std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("timed out waiting for {}", path.display()),
                     )
                     .into(),
                  );
               }
               time::sleep(Duration::from_millis(100)).await;
            },
            Err(e) => return Err(e.into()),
         }
      }
   }
}

impl Drop for ArtifactLock {
   fn drop(&mut self) {
      let _ = self.file.unlock();
   }
}
