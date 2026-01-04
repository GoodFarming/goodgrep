use std::{fs, path::PathBuf};

fn conformance_map_path() -> PathBuf {
   PathBuf::from(env!("CARGO_MANIFEST_DIR"))
      .join("Docs/Spec/GGREP-Conformance-Map-v0.1.md")
}

fn read_rs_files(dir: &PathBuf) -> Vec<(PathBuf, String)> {
   let mut out = Vec::new();
   if let Ok(entries) = fs::read_dir(dir) {
      for entry in entries.flatten() {
         let path = entry.path();
         if path.is_dir() {
            out.extend(read_rs_files(&path));
         } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if let Ok(raw) = fs::read_to_string(&path) {
               out.push((path, raw));
            }
         }
      }
   }
   out
}

fn find_fn_in_sources(name: &str, sources: &[(PathBuf, String)]) -> bool {
   let needle = format!("fn {name}");
   sources.iter().any(|(_, raw)| raw.contains(&needle))
}

#[test]
fn conformance_map_has_no_tbd_and_refs_exist() {
   let path = conformance_map_path();
   let raw = fs::read_to_string(&path).expect("conformance map");

   let test_sources = read_rs_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests"));
   let assert_sources = read_rs_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"));

   for line in raw.lines() {
      let trimmed = line.trim();
      if !trimmed.starts_with('|') || trimmed.contains("---") {
         continue;
      }
      let cols: Vec<String> = trimmed
         .trim_matches('|')
         .split('|')
         .map(|c| c.trim().to_string())
         .collect();
      if cols.len() < 5 || cols[0] == "Requirement ID" {
         continue;
      }
      let test_id = cols[4].trim().trim_matches('`');
      if test_id.is_empty() || test_id.contains("TBD") {
         panic!("conformance map entry missing test/assertion: {}", line);
      }

      if let Some(rest) = test_id.strip_prefix("tests::") {
         let name = rest.rsplit("::").next().unwrap_or(rest);
         if !find_fn_in_sources(name, &test_sources) {
            panic!("missing test function '{name}' referenced in conformance map");
         }
      } else if let Some(rest) = test_id.strip_prefix("assert::") {
         let name = rest.rsplit("::").next().unwrap_or(rest);
         if !find_fn_in_sources(name, &assert_sources) {
            panic!("missing assert function '{name}' referenced in conformance map");
         }
      } else {
         panic!("unknown test/assertion namespace '{test_id}'");
      }
   }
}
