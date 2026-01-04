//! Writer lease management (single writer + heartbeat).

use std::{
   fs::{self, File, OpenOptions},
   io::Write,
   path::{Path, PathBuf},
   time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::{task::JoinHandle, time};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{Result, config, error::Error, util::fsync_dir};

pub const LEASE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterLeaseRecord {
   pub schema_version:    u32,
   pub owner_id:          String,
   pub pid:               u32,
   pub hostname:          String,
   pub started_at:        String,
   pub last_heartbeat_at: String,
   pub lease_epoch:       u64,
   pub lease_ttl_ms:      u64,
   pub staging_txn_id:    Option<String>,
}

impl WriterLeaseRecord {
   fn new(owner_id: String, lease_epoch: u64, lease_ttl_ms: u64) -> Self {
      let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
      Self {
         schema_version: LEASE_SCHEMA_VERSION,
         owner_id,
         pid: std::process::id(),
         hostname: hostname(),
         started_at: now.clone(),
         last_heartbeat_at: now,
         lease_epoch,
         lease_ttl_ms,
         staging_txn_id: None,
      }
   }

   fn is_stale(&self, now: DateTime<Utc>) -> bool {
      let Ok(last) = DateTime::parse_from_rfc3339(&self.last_heartbeat_at) else {
         return true;
      };
      let last = last.with_timezone(&Utc);
      let elapsed = now.signed_duration_since(last).num_milliseconds();
      elapsed > self.lease_ttl_ms as i64
   }
}

pub struct WriterLease {
   store_id:    String,
   owner_id:    String,
   lease_epoch: u64,
   token:       CancellationToken,
   heartbeat:   Option<JoinHandle<()>>,
}

impl WriterLease {
   pub async fn acquire(store_id: &str) -> Result<Self> {
      let cfg = config::get();
      let guard = LeaseGuard::acquire(store_id, Duration::from_secs(5)).await?;
      let path = lease_path(store_id);
      let now = Utc::now();

      let existing = read_lease(&path).ok();
      if let Some(ref lease) = existing {
         if !lease.is_stale(now) {
            return Err(
               Error::Server { op: "lease", reason: "writer lease already held".to_string() }
                  .into(),
            );
         }
      }

      let epoch = existing.map(|lease| lease.lease_epoch + 1).unwrap_or(1);
      let owner_id = Uuid::new_v4().to_string();
      let record = WriterLeaseRecord::new(owner_id.clone(), epoch, cfg.lease_ttl_ms);
      write_lease_atomic(&path, &record)?;
      drop(guard);

      let token = CancellationToken::new();
      let token_clone = token.clone();
      let store_id_owned = store_id.to_string();
      let owner_id_clone = owner_id.clone();
      let heartbeat_interval =
         Duration::from_millis(cfg.lease_ttl_ms / 3).max(Duration::from_millis(250));
      let heartbeat = tokio::spawn(async move {
         loop {
            tokio::select! {
               _ = token_clone.cancelled() => break,
               _ = time::sleep(heartbeat_interval) => {
                  if let Err(err) = heartbeat_once(&store_id_owned, &owner_id_clone, epoch).await {
                     tracing::warn!("lease heartbeat failed: {err}");
                     break;
                  }
               }
            }
         }
      });

      Ok(Self {
         store_id: store_id.to_string(),
         owner_id,
         lease_epoch: epoch,
         token,
         heartbeat: Some(heartbeat),
      })
   }

   pub fn lease_epoch(&self) -> u64 {
      self.lease_epoch
   }

   pub fn owner_id(&self) -> &str {
      &self.owner_id
   }

   pub async fn set_staging_txn_id(&self, staging_txn_id: Option<String>) -> Result<()> {
      let _guard = LeaseGuard::acquire(&self.store_id, Duration::from_secs(5)).await?;
      let path = lease_path(&self.store_id);
      let mut lease = read_lease(&path)?;
      if lease.owner_id != self.owner_id || lease.lease_epoch != self.lease_epoch {
         return Err(Error::Server { op: "lease", reason: "lease ownership lost".to_string() }.into());
      }
      lease.staging_txn_id = staging_txn_id;
      write_lease_atomic(&path, &lease)?;
      Ok(())
   }
}

pub(crate) fn read_lease_record(store_id: &str) -> Result<WriterLeaseRecord> {
   read_lease(&lease_path(store_id))
}

pub(crate) fn verify_lease_owner(
   store_id: &str,
   owner_id: &str,
   lease_epoch: u64,
) -> Result<()> {
   let lease = read_lease_record(store_id)?;
   if lease.owner_id != owner_id || lease.lease_epoch != lease_epoch {
      return Err(Error::Server { op: "lease", reason: "lease ownership lost".to_string() }.into());
   }
   Ok(())
}

impl Drop for WriterLease {
   fn drop(&mut self) {
      self.token.cancel();
      if let Some(handle) = self.heartbeat.take() {
         handle.abort();
      }
      let _ = release_lease(&self.store_id, &self.owner_id, self.lease_epoch);
   }
}

async fn heartbeat_once(store_id: &str, owner_id: &str, lease_epoch: u64) -> Result<()> {
   let _guard = LeaseGuard::acquire(store_id, Duration::from_secs(5)).await?;
   let path = lease_path(store_id);
   let mut lease = read_lease(&path)?;
   if lease.owner_id != owner_id || lease.lease_epoch != lease_epoch {
      return Err(Error::Server { op: "lease", reason: "lease ownership lost".to_string() }.into());
   }
   lease.last_heartbeat_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
   write_lease_atomic(&path, &lease)?;
   Ok(())
}

fn lease_path(store_id: &str) -> PathBuf {
   config::data_dir()
      .join(store_id)
      .join("locks")
      .join("writer_lease.json")
}

fn guard_path(store_id: &str) -> PathBuf {
   config::data_dir()
      .join(store_id)
      .join("locks")
      .join("lease_guard.lock")
}

fn read_lease(path: &Path) -> Result<WriterLeaseRecord> {
   let raw = fs::read_to_string(path)?;
   let lease: WriterLeaseRecord = serde_json::from_str(&raw)?;
   Ok(lease)
}

fn write_lease_atomic(path: &Path, lease: &WriterLeaseRecord) -> Result<()> {
   if let Some(parent) = path.parent() {
      fs::create_dir_all(parent)?;
   }
   let tmp_path = temp_path(path)?;
   let data = serde_json::to_string_pretty(lease)?;
   fs::write(&tmp_path, data)?;
   fs::rename(&tmp_path, path)?;
   if let Some(parent) = path.parent() {
      fsync_dir(parent)?;
   }
   Ok(())
}

fn release_lease(store_id: &str, owner_id: &str, lease_epoch: u64) -> Result<()> {
   let path = lease_path(store_id);
   let Ok(lease) = read_lease(&path) else {
      return Ok(());
   };
   if lease.owner_id == owner_id && lease.lease_epoch == lease_epoch {
      let _ = fs::remove_file(&path);
   }
   Ok(())
}

fn temp_path(path: &Path) -> Result<PathBuf> {
   let name = path
      .file_name()
      .and_then(|n| n.to_str())
      .ok_or_else(|| Error::Server { op: "lease", reason: "invalid lease path".to_string() })?;
   Ok(path.with_file_name(format!("{name}.tmp")))
}

struct LeaseGuard {
   path: PathBuf,
}

impl LeaseGuard {
   async fn acquire(store_id: &str, timeout: Duration) -> Result<Self> {
      let path = guard_path(store_id);
      if let Some(parent) = path.parent() {
         fs::create_dir_all(parent)?;
      }

      let start = std::time::Instant::now();
      let ttl = Duration::from_secs(10);
      loop {
         match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(mut file) => {
               let _ = write_guard_metadata(&mut file);
               let _ = file.sync_all();
               return Ok(Self { path });
            },
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
               if is_guard_stale(&path, ttl) {
                  let _ = fs::remove_file(&path);
                  continue;
               }
               if start.elapsed() >= timeout {
                  return Err(
                     Error::Server { op: "lease", reason: "lease guard timeout".to_string() }
                        .into(),
                  );
               }
               time::sleep(Duration::from_millis(25)).await;
            },
            Err(e) => return Err(e.into()),
         }
      }
   }
}

impl Drop for LeaseGuard {
   fn drop(&mut self) {
      let _ = fs::remove_file(&self.path);
   }
}

fn write_guard_metadata(file: &mut File) -> std::io::Result<()> {
   let pid = std::process::id();
   let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_millis();
   writeln!(file, "pid={pid}")?;
   writeln!(file, "started_at_ms={now}")?;
   Ok(())
}

fn is_guard_stale(path: &Path, ttl: Duration) -> bool {
   let age = match fs::metadata(path).and_then(|m| m.modified()) {
      Ok(mtime) => SystemTime::now()
         .duration_since(mtime)
         .unwrap_or(Duration::MAX),
      Err(_) => Duration::MAX,
   };

   let pid = read_guard_pid(path);
   if let Some(pid) = pid {
      if pid_is_alive(pid) {
         return false;
      }
   }

   age > ttl
}

fn read_guard_pid(path: &Path) -> Option<u32> {
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

fn hostname() -> String {
   std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string())
}
