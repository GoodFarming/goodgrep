use std::path::PathBuf;

use ggrep::snapshot::SnapshotManifest;

#[test]
fn manifest_fixture_parses() {
   let path =
      PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schemas/manifest.json");
   let data = std::fs::read_to_string(&path).expect("fixture readable");
   let manifest: SnapshotManifest = serde_json::from_str(&data).expect("valid manifest");
   assert_eq!(manifest.schema_version, 1);
   assert_eq!(manifest.chunk_row_schema_version, 1);
   assert!(!manifest.snapshot_id.is_empty());
}
