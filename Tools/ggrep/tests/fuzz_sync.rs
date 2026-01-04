mod support;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ggrep::{
   chunker::Chunker,
   config,
   embed::{DummyEmbedder, Embedder},
   file::LocalFileSystem,
   identity,
   search::SearchEngine,
   snapshot::SnapshotManager,
   store::LanceStore,
   sync::{SyncEngine, SyncOptions},
   types::SearchMode,
};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use support::set_temp_home;
use tempfile::TempDir;
use uuid::Uuid;

#[derive(Debug, Clone)]
enum Op {
   Add { name: String, suffix: u16 },
   Modify { name: String, suffix: u16 },
   Delete { name: String },
   Rename { from: String, to: String },
}

fn file_name_strategy() -> impl Strategy<Value = String> {
   (0usize..5).prop_map(|i| format!("file{i}.rs"))
}

fn op_strategy() -> impl Strategy<Value = Op> {
   prop_oneof![
      (file_name_strategy(), any::<u16>())
         .prop_map(|(name, suffix)| Op::Add { name, suffix }),
      (file_name_strategy(), any::<u16>())
         .prop_map(|(name, suffix)| Op::Modify { name, suffix }),
      file_name_strategy().prop_map(|name| Op::Delete { name }),
      (file_name_strategy(), file_name_strategy())
         .prop_filter("rename requires distinct names", |(from, to)| from != to)
         .prop_map(|(from, to)| Op::Rename { from, to }),
   ]
}

fn write_file(root: &Path, name: &str, suffix: u16) {
   let content = format!("fn {name}() {{}}\n// token_{name} {suffix}\n");
   std::fs::write(root.join(name), content).expect("write file");
}

#[test]
fn sync_fuzz_invariants_fixed_seed() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let seed = [42u8; 32];
   let mut runner = TestRunner::new_with_rng(
      Config { cases: 16, max_shrink_iters: 0, ..Config::default() },
      TestRng::from_seed(RngAlgorithm::ChaCha, &seed),
   );

   let strategy = prop::collection::vec(op_strategy(), 1..8);

   runner
      .run(&strategy, |ops| {
         let rt = tokio::runtime::Runtime::new().expect("runtime");
         rt.block_on(async {
            let repo = TempDir::new().expect("repo");
            let root = repo.path();

            let outside = TempDir::new().expect("outside");
            let outside_file = outside.path().join("outside.rs");
            std::fs::write(&outside_file, "token_outside").expect("outside file");
            let _ = std::os::unix::fs::symlink(&outside_file, root.join("outside.rs"));

            config::init_for_root(root);
            let store_id = format!("fuzz-{}", Uuid::new_v4());
            let store = Arc::new(LanceStore::new().expect("store"));
            let embedder: Arc<dyn Embedder> =
               Arc::new(DummyEmbedder::new(config::get().dense_dim));
            let sync_engine = SyncEngine::new(
               LocalFileSystem::new(),
               Chunker::default(),
               embedder.clone(),
               store.clone(),
            );

            let fingerprints = identity::compute_fingerprints(root).expect("fingerprints");
            let snapshot_manager = SnapshotManager::new(
               store.clone(),
               store_id.clone(),
               fingerprints.config_fingerprint,
               fingerprints.ignore_fingerprint,
            );
            let search_engine = SearchEngine::new(store, embedder);
            let include_anchors = config::get().fast_mode;

            let mut state: HashMap<String, u16> = HashMap::new();
            let all_files: Vec<String> = (0usize..5).map(|i| format!("file{i}.rs")).collect();

            for op in ops {
               let prev_state = state.clone();
               let prev_view = snapshot_manager.open_snapshot_view().await.ok();

               match op {
                  Op::Add { name, suffix } | Op::Modify { name, suffix } => {
                     write_file(root, &name, suffix);
                     state.insert(name, suffix);
                  },
                  Op::Delete { name } => {
                     let _ = std::fs::remove_file(root.join(&name));
                     state.remove(&name);
                  },
                  Op::Rename { from, to } => {
                     if root.join(&from).exists() {
                        let _ = std::fs::rename(root.join(&from), root.join(&to));
                        if let Some(value) = state.remove(&from) {
                           state.insert(to, value);
                        }
                     }
                  },
               }

               sync_engine
                  .initial_sync_with_options(
                     &store_id,
                     root,
                     None,
                     false,
                     SyncOptions::default(),
                     &mut (),
                  )
                  .await
                  .expect("sync");

               let view = snapshot_manager.open_snapshot_view().await.expect("view");
               for name in &all_files {
                  let token = format!("token_{name}");
                  let results = search_engine
                     .search_with_mode(
                        &view,
                        &store_id,
                        &token,
                        5,
                        5,
                        None,
                        false,
                        include_anchors,
                        SearchMode::Balanced,
                     )
                     .await
                     .expect("search");
                  let has_path = results.results.iter().any(|r| r.path.ends_with(name));
                  if state.contains_key(name) {
                     assert!(has_path, "{name} should be indexed");
                  } else {
                     assert!(!has_path, "{name} should be absent");
                  }
               }

               let outside_results = search_engine
                  .search_with_mode(
                     &view,
                     &store_id,
                     "token_outside",
                     5,
                     5,
                     None,
                     false,
                     include_anchors,
                     SearchMode::Balanced,
                  )
                  .await
                  .expect("search outside");
               assert!(
                  outside_results
                     .results
                     .iter()
                     .all(|r| !r.path.ends_with("outside.rs")),
                  "out-of-root symlink should be ignored"
               );

               if let Some(prev_view) = prev_view {
                  if let Some((name, _)) = prev_state.iter().next() {
                     let token = format!("token_{name}");
                     let results = search_engine
                        .search_with_mode(
                           &prev_view,
                           &store_id,
                           &token,
                           5,
                           5,
                           None,
                           false,
                           include_anchors,
                           SearchMode::Balanced,
                        )
                        .await
                        .expect("search prev");
                     assert!(
                        results.results.iter().any(|r| r.path.ends_with(name)),
                        "pinned snapshot should still serve {name}"
                     );
                  }
               }
            }
         });

         Ok(())
      })
      .expect("proptest");
}
