use std::path::PathBuf;

use ggrep::file::{FileSystem, LocalFileSystem};

#[test]
fn ignore_conformance_fixture() {
   let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ignore");

   let fs = LocalFileSystem::new();
   let mut actual: Vec<String> = fs
      .get_files(&root)
      .expect("fixture discovery should succeed")
      .map(|f| f.path_key.to_string_lossy().into_owned())
      .collect();
   actual.sort();

   let expected_path = root.join("expected.txt");
   let expected_contents =
      std::fs::read_to_string(expected_path).expect("expected fixture list should be readable");
   let mut expected: Vec<String> = expected_contents
      .lines()
      .filter(|line| !line.trim().is_empty())
      .map(|line| line.trim().to_string())
      .collect();
   expected.sort();

   assert_eq!(actual, expected);
}
