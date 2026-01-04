mod support;

use std::{path::PathBuf, time::Duration};

use ggrep::{
   config,
   file::{FileSystem, FileWatcher, IgnorePatterns, WatchAction},
   meta::MetaStore,
   sync::{ChangeDetector, FileSystemChangeDetector},
};
use support::set_temp_home;
use tempfile::TempDir;

#[tokio::test]
async fn ignore_parity_across_discovery_watcher_manual() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();

   std::fs::write(root.join(".gitignore"), "ignored*.rs\nignored_dir/\n").expect("write gitignore");

   std::fs::write(root.join("ignored.rs"), "ignored\n").expect("seed ignored");
   std::fs::write(root.join("tracked.rs"), "tracked\n").expect("seed tracked");

   config::init_for_root(root);

   let fs = ggrep::file::LocalFileSystem::new();
   let mut discovered: Vec<PathBuf> = fs
      .get_files(root)
      .expect("discover files")
      .map(|p| p.path_key)
      .collect();
   discovered.sort();
   assert!(discovered.contains(&PathBuf::from("tracked.rs")));
   assert!(!discovered.contains(&PathBuf::from("ignored.rs")));

   let meta_store = MetaStore::load("ignore-parity").expect("meta store");
   let detector = FileSystemChangeDetector::new(&fs);
   let changes = detector
      .detect(root, &meta_store)
      .await
      .expect("detect changes");
   let add_paths: Vec<PathBuf> = changes.add.iter().map(|p| p.path_key.clone()).collect();
   assert!(add_paths.contains(&PathBuf::from("tracked.rs")));
   assert!(!add_paths.contains(&PathBuf::from("ignored.rs")));

   let (tx, rx) = std::sync::mpsc::channel::<Vec<(PathBuf, WatchAction)>>();
   let _watcher = FileWatcher::new(root.to_path_buf(), IgnorePatterns::new(root), move |changes| {
      let _ = tx.send(changes);
   })
   .expect("watcher");

   std::fs::write(root.join("ignored2.rs"), "ignored\n").expect("ignored file");
   std::fs::write(root.join("tracked2.rs"), "tracked\n").expect("tracked file");

   tokio::time::sleep(Duration::from_millis(800)).await;

   let mut seen: Vec<PathBuf> = Vec::new();
   while let Ok(batch) = rx.try_recv() {
      for (path, _) in batch {
         if let Some(name) = path.file_name() {
            seen.push(PathBuf::from(name));
         }
      }
   }

   assert!(seen.contains(&PathBuf::from("tracked2.rs")));
   assert!(!seen.contains(&PathBuf::from("ignored2.rs")));
}
