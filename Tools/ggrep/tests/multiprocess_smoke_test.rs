use std::{
   env,
   path::PathBuf,
   process::{Command, ExitCode},
   thread,
   time::Duration,
};

use ggrep::{
   chunker::Chunker,
   config,
   embed::{DummyEmbedder, Embedder},
   file::LocalFileSystem,
   identity,
   search::SearchEngine,
   snapshot::SnapshotManager,
   store::LanceStore,
   sync::SyncEngine,
   types::SearchMode,
};
use tempfile::TempDir;
use tokio::runtime::Builder;

const CHILD_ENV: &str = "GGREP_MP_CHILD";
const ROOT_ENV: &str = "GGREP_MP_ROOT";
const HOME_ENV: &str = "GGREP_MP_HOME";

fn main() -> ExitCode {
   if env::var(CHILD_ENV).is_ok() {
      return match run_child() {
         Ok(()) => ExitCode::SUCCESS,
         Err(err) => {
            eprintln!("child error: {err}");
            ExitCode::FAILURE
         },
      };
   }

   match run_parent() {
      Ok(()) => ExitCode::SUCCESS,
      Err(err) => {
         eprintln!("parent error: {err}");
         ExitCode::FAILURE
      },
   }
}

fn run_parent() -> anyhow::Result<()> {
   let temp_home = TempDir::new()?;
   let repo = TempDir::new()?;
   let root = repo.path();

   for idx in 0..200 {
      std::fs::write(root.join(format!("file_{idx}.rs")), "pub fn demo() {}\n")?;
   }

   // Safe in this single-purpose test binary: set before any threads are spawned.
   unsafe {
      env::set_var("HOME", temp_home.path());
   }
   config::init_for_root(root);

   let store_id = "multiprocess-smoke";

   let rt = Builder::new_current_thread().enable_all().build()?;
   rt.block_on(async {
      let store = std::sync::Arc::new(LanceStore::new()?);
      let embedder: std::sync::Arc<dyn Embedder> =
         std::sync::Arc::new(DummyEmbedder::new(config::get().dense_dim));
      let chunker = Chunker::default();
      let sync = SyncEngine::new(LocalFileSystem::new(), chunker, embedder, store);

      sync
         .initial_sync(store_id, root, None, false, &mut ())
         .await?;

      let mut child = spawn_child(root, temp_home.path())?;

      for iter in 0..3 {
         std::fs::write(root.join("main.rs"), format!("pub fn demo() {{ {iter} }}\n"))?;
         sync
            .initial_sync(store_id, root, None, false, &mut ())
            .await?;
         thread::sleep(Duration::from_millis(50));
      }

      let status = child.wait()?;
      if !status.success() {
         return Err(anyhow::anyhow!("child exited with {status}"));
      }
      Ok::<(), anyhow::Error>(())
   })?;

   Ok(())
}

fn spawn_child(
   root: &std::path::Path,
   home: &std::path::Path,
) -> anyhow::Result<std::process::Child> {
   let exe = env::current_exe()?;
   let mut cmd = Command::new(exe);
   cmd.env(CHILD_ENV, "1")
      .env(ROOT_ENV, root)
      .env(HOME_ENV, home);
   Ok(cmd.spawn()?)
}

fn run_child() -> anyhow::Result<()> {
   let root = PathBuf::from(env::var(ROOT_ENV)?);
   let home = PathBuf::from(env::var(HOME_ENV)?);
   // Safe in this single-purpose test binary: set before any threads are spawned.
   unsafe {
      env::set_var("HOME", &home);
   }
   config::init_for_root(&root);

   let store_id = "multiprocess-smoke";

   let rt = Builder::new_current_thread().enable_all().build()?;
   rt.block_on(async {
      let store = std::sync::Arc::new(LanceStore::new()?);
      let embedder: std::sync::Arc<dyn Embedder> =
         std::sync::Arc::new(DummyEmbedder::new(config::get().dense_dim));
      let engine = SearchEngine::new(store.clone(), embedder);
      let fingerprints = identity::compute_fingerprints(&root)?;
      let snapshot_manager = SnapshotManager::new(
         store,
         store_id.to_string(),
         fingerprints.config_fingerprint,
         fingerprints.ignore_fingerprint,
      );
      let include_anchors = config::get().fast_mode;

      for _ in 0..5 {
         let snapshot_view = snapshot_manager.open_snapshot_view().await?;
         engine
            .search_with_mode(
               &snapshot_view,
               store_id,
               "demo",
               5,
               2,
               None,
               false,
               include_anchors,
               SearchMode::Balanced,
            )
            .await?;
         thread::sleep(Duration::from_millis(50));
      }
      Ok::<(), anyhow::Error>(())
   })?;

   Ok(())
}
