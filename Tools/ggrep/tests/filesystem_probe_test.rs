use ggrep::util::probe_store_path;
use tempfile::TempDir;

#[test]
fn probe_store_path_accepts_local_fs() {
   let dir = TempDir::new().expect("temp dir");
   probe_store_path(dir.path()).expect("probe store path");
}
