use std::{
   collections::HashMap,
   fs,
   path::{Path, PathBuf},
};

use jsonschema::JSONSchema;
use serde_json::Value;

fn load_schemas(root: &Path) -> HashMap<String, JSONSchema> {
   let schema_dir = root.join("Docs/Spec/Schemas");
   let mut schemas = HashMap::new();
   for entry in fs::read_dir(schema_dir).expect("schema dir") {
      let entry = entry.expect("schema entry");
      let path = entry.path();
      if !path.is_file() {
         continue;
      }
      let name = path
         .file_name()
         .and_then(|n| n.to_str())
         .and_then(|n| n.strip_suffix(".schema.json"))
         .expect("schema file name");
      let raw = fs::read_to_string(&path).expect("schema read");
      let json: Value = serde_json::from_str(&raw).expect("schema json");
      let schema = JSONSchema::compile(&json).expect("schema compile");
      schemas.insert(name.to_string(), schema);
   }
   schemas
}

fn parse_schema_marker(line: &str) -> Option<String> {
   let trimmed = line.trim();
   let prefix = "<!-- schema:";
   if !trimmed.starts_with(prefix) {
      return None;
   }
   let rest = trimmed.strip_prefix(prefix)?;
   let rest = rest.trim();
   let rest = rest.strip_suffix("-->")?;
   Some(rest.trim().to_string())
}

fn extract_annotated_json(doc: &str) -> Vec<(String, String)> {
   let lines: Vec<&str> = doc.lines().collect();
   let mut out = Vec::new();
   let mut i = 0;
   while i < lines.len() {
      if let Some(schema) = parse_schema_marker(lines[i]) {
         i += 1;
         while i < lines.len() && !lines[i].trim_start().starts_with("```json") {
            i += 1;
         }
         if i >= lines.len() {
            break;
         }
         i += 1;
         let mut json_lines = Vec::new();
         while i < lines.len() && !lines[i].trim_start().starts_with("```") {
            json_lines.push(lines[i]);
            i += 1;
         }
         let json = json_lines.join("\n");
         out.push((schema, json));
      } else {
         i += 1;
      }
   }
   out
}

fn validate_instance(schema: &JSONSchema, instance: &Value, label: &str) {
   if let Err(errors) = schema.validate(instance) {
      let messages: Vec<String> = errors.map(|e| e.to_string()).collect();
      panic!("schema validation failed for {label}: {}", messages.join("; "));
   }
}

fn fixture_paths(root: &Path, subdir: &str) -> Vec<PathBuf> {
   let dir = root.join(subdir);
   if !dir.exists() {
      return Vec::new();
   }
   let mut out = Vec::new();
   for entry in fs::read_dir(dir).expect("fixture dir") {
      let entry = entry.expect("fixture entry");
      let path = entry.path();
      if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("json") {
         out.push(path);
      }
   }
   out
}

#[test]
fn doc_examples_validate_against_schemas() {
   let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
   let schemas = load_schemas(&root);
   let docs = [
      root.join("Docs/Spec/GGREP-Query-Daemon-Contracts-v0.1.md"),
      root.join("Docs/Spec/GGREP-Snapshot-Index-Contracts-v0.1.md"),
   ];

   for doc_path in docs {
      let raw = fs::read_to_string(&doc_path).expect("doc read");
      for (schema_name, json) in extract_annotated_json(&raw) {
         let schema = schemas
            .get(&schema_name)
            .unwrap_or_else(|| panic!("schema not found: {schema_name}"));
         let instance: Value = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("json parse failed for {schema_name}: {e}"));
         validate_instance(schema, &instance, &format!("{schema_name} ({})", doc_path.display()));
      }
   }
}

#[test]
fn fixtures_validate_against_schemas() {
   let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
   let schemas = load_schemas(&root);
   let fixtures = fixture_paths(&root, "tests/fixtures/schemas");

   for path in fixtures {
      if path.parent().and_then(|p| p.file_name()) == Some(std::ffi::OsStr::new("forward")) {
         continue;
      }
      let name = path
         .file_stem()
         .and_then(|n| n.to_str())
         .expect("fixture name");
      let schema = schemas
         .get(name)
         .unwrap_or_else(|| panic!("schema not found for fixture: {name}"));
      let raw = fs::read_to_string(&path).expect("fixture read");
      let instance: Value = serde_json::from_str(&raw)
         .unwrap_or_else(|e| panic!("fixture json parse failed for {name}: {e}"));
      validate_instance(schema, &instance, &format!("fixture {name} ({})", path.display()));
   }
}

#[test]
fn forward_compat_fixtures_fail_validation() {
   let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
   let schemas = load_schemas(&root);
   let fixtures = fixture_paths(&root, "tests/fixtures/schemas/forward");

   for path in fixtures {
      let name = path
         .file_stem()
         .and_then(|n| n.to_str())
         .expect("fixture name");
      let schema_name = name.trim_end_matches("_v99").to_string();
      let schema = schemas
         .get(&schema_name)
         .unwrap_or_else(|| panic!("schema not found for fixture: {schema_name}"));
      let raw = fs::read_to_string(&path).expect("fixture read");
      let instance: Value = serde_json::from_str(&raw)
         .unwrap_or_else(|e| panic!("fixture json parse failed for {name}: {e}"));
      if schema.is_valid(&instance) {
         panic!("forward compat fixture unexpectedly validated: {}", path.display());
      }
   }
}
