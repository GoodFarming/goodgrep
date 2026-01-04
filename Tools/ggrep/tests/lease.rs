use std::path::PathBuf;

use ggrep::lease::WriterLeaseRecord;

#[test]
fn lease_fixture_parses() {
   let path =
      PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schemas/writer_lease.json");
   let data = std::fs::read_to_string(&path).expect("fixture readable");
   let lease: WriterLeaseRecord = serde_json::from_str(&data).expect("valid lease");
   assert_eq!(lease.schema_version, 1);
   assert!(!lease.owner_id.is_empty());
}
