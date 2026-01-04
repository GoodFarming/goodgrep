use std::{
   fs,
   path::{Path, PathBuf},
};

use walkdir::WalkDir;

struct Rule {
   needle:           &'static str,
   allowed_prefixes: &'static [&'static str],
}

fn is_allowed(path: &Path, allowed_prefixes: &[&str]) -> bool {
   let path_str = path.to_string_lossy().replace('\\', "/");
   allowed_prefixes
      .iter()
      .any(|prefix| path_str.ends_with(prefix) || path_str.contains(prefix))
}

#[test]
fn module_boundary_lint() {
   let rules = [
      Rule {
         needle:           "crate::store::lance",
         allowed_prefixes: &["/src/store/", "src/store/", "src/error.rs"],
      },
      Rule {
         needle:           "crate::embed::candle",
         allowed_prefixes: &["/src/embed/", "src/embed/", "src/error.rs"],
      },
   ];

   let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
   let mut violations = Vec::new();

   for entry in WalkDir::new(&root).into_iter().filter_map(Result::ok) {
      if !entry.file_type().is_file() {
         continue;
      }
      if entry.path().extension().and_then(|e| e.to_str()) != Some("rs") {
         continue;
      }

      let Ok(contents) = fs::read_to_string(entry.path()) else {
         continue;
      };

      for rule in &rules {
         if contents.contains(rule.needle) && !is_allowed(entry.path(), rule.allowed_prefixes) {
            violations.push(format!(
               "{}: forbidden import '{}'; allowed in {:?}",
               entry.path().display(),
               rule.needle,
               rule.allowed_prefixes
            ));
         }
      }
   }

   if !violations.is_empty() {
      panic!("module boundary violations:\n{}", violations.join("\n"));
   }
}
