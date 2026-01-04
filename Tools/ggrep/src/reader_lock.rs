use std::{
   fs::{self, File, OpenOptions},
   path::PathBuf,
};

use fs4::FileExt;

use crate::{Result, config};

pub struct ReaderLock {
   file: File,
}

impl ReaderLock {
   fn lock_path(store_id: &str) -> PathBuf {
      config::data_dir()
         .join(store_id)
         .join("locks")
         .join("readers.lock")
   }

   pub fn acquire_shared(store_id: &str) -> Result<Self> {
      let lock_path = Self::lock_path(store_id);
      if let Some(parent) = lock_path.parent() {
         fs::create_dir_all(parent)?;
      }
      let file = OpenOptions::new()
         .create(true)
         .read(true)
         .write(true)
         .open(&lock_path)?;
      file.lock_shared()?;
      Ok(Self { file })
   }

   pub fn acquire_exclusive(store_id: &str) -> Result<Self> {
      let lock_path = Self::lock_path(store_id);
      if let Some(parent) = lock_path.parent() {
         fs::create_dir_all(parent)?;
      }
      let file = OpenOptions::new()
         .create(true)
         .read(true)
         .write(true)
         .open(&lock_path)?;
      file.lock_exclusive()?;
      Ok(Self { file })
   }
}

impl Drop for ReaderLock {
   fn drop(&mut self) {
      let _ = self.file.unlock();
   }
}
