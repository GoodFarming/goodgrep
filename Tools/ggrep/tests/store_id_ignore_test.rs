mod support;

use ggrep::{identity, config};
use support::set_temp_home;
use tempfile::TempDir;

#[test]
fn ignore_changes_do_not_change_store_id() {
   let temp_home = TempDir::new().expect("temp home");
   set_temp_home(&temp_home);

   let repo = TempDir::new().expect("temp repo");
   let root = repo.path();

   std::fs::write(root.join("a.rs"), "fn a() {}\n").expect("seed file");
   std::fs::write(root.join(".ggignore"), "ignored.txt\n").expect("ignore file");

   config::init_for_root(root);
   let identity_a = identity::resolve_index_identity(root).expect("identity a");

   std::fs::write(root.join(".ggignore"), "ignored.txt\nextra.log\n")
      .expect("update ignore");
   let identity_b = identity::resolve_index_identity(root).expect("identity b");

   assert_eq!(identity_a.store_id, identity_b.store_id);
   assert_ne!(identity_a.ignore_fingerprint, identity_b.ignore_fingerprint);
}
