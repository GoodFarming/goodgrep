use std::{
   fs::{self, File},
   io::Write,
};

use ggrep::{Result, util::fsync_dir};
use tempfile::tempdir;

#[test]
fn durability_barrier_smoke() -> Result<()> {
   let dir = tempdir()?;
   let base = dir.path();

   let tmp_path = base.join("snapshot.tmp");
   let final_path = base.join("snapshot");

   let mut file = File::create(&tmp_path)?;
   file.write_all(b"probe")?;
   file.sync_all()?;
   drop(file);

   fsync_dir(base)?;

   fs::rename(&tmp_path, &final_path)?;
   fsync_dir(base)?;

   let contents = fs::read(&final_path)?;
   assert_eq!(contents, b"probe");

   Ok(())
}
