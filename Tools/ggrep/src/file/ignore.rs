//! Ignore pattern handling for filtering files during discovery and watching.

use std::{
   collections::BTreeMap,
   path::{Path, PathBuf},
};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use walkdir::WalkDir;

/// Default patterns for files and directories to ignore during file discovery.
const DEFAULT_IGNORE_PATTERNS: &[&str] = &[
   "**/Datasets/**",
   "**/node_modules/**",
   "**/dist/**",
   "**/build/**",
   "**/out/**",
   "**/target/**",
   "**/__pycache__/**",
   "**/.git/**",
   "**/.venv/**",
   "**/venv/**",
   "*.lock",
   "*.bin",
   "*.ipynb",
   "*.pyc",
   "*.onnx",
   "package-lock.json",
   "yarn.lock",
   "pnpm-lock.yaml",
   "bun.lockb",
   "composer.lock",
   "Cargo.lock",
   "Gemfile.lock",
   "*.min.js",
   "*.min.css",
   "*.map",
   "**/coverage/**",
   "**/.nyc_output/**",
   "**/.pytest_cache/**",
];

const IGNORE_FILENAMES: &[&str] = &[".gitignore", ".ggignore", ".smignore"];

const DEFAULT_IGNORE_DIRS: &[&str] = &[
   ".git",
   "Datasets",
   "node_modules",
   "dist",
   "build",
   "out",
   "target",
   "__pycache__",
   ".venv",
   "venv",
   "coverage",
   ".nyc_output",
   ".pytest_cache",
];

/// Manages file and directory ignore patterns from `.gitignore`, `.ggignore`,
/// and legacy `.smignore` files.
pub struct IgnorePatterns {
   root:         PathBuf,
   root_matcher: Option<Gitignore>,
   dir_matchers: BTreeMap<PathBuf, Gitignore>,
}

impl IgnorePatterns {
   /// Creates ignore patterns by loading default patterns, `.gitignore`,
   /// `.ggignore`, and legacy `.smignore`.
   pub fn new(root: &Path) -> Self {
      let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
      let mut ignore_files = collect_ignore_files(&root);
      ignore_files.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));

      let mut per_dir: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
      for ignore_file in ignore_files {
         let Some(parent) = ignore_file.parent() else {
            continue;
         };
         per_dir
            .entry(parent.to_path_buf())
            .or_default()
            .push(ignore_file);
      }

      let mut root_builder = GitignoreBuilder::new(&root);
      for pattern in DEFAULT_IGNORE_PATTERNS {
         let _ = root_builder.add_line(None, pattern);
      }
      if let Some(root_files) = per_dir.remove(&root) {
         for ignore_file in root_files {
            let _ = root_builder.add(ignore_file);
         }
      }

      let root_matcher = root_builder.build().ok();
      let mut dir_matchers = BTreeMap::new();
      for (dir, mut files) in per_dir {
         files.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
         let mut builder = GitignoreBuilder::new(&dir);
         for ignore_file in files {
            let _ = builder.add(ignore_file);
         }
         if let Ok(gi) = builder.build() {
            dir_matchers.insert(dir, gi);
         }
      }

      Self { root, root_matcher, dir_matchers }
   }

   /// Checks whether a path matches any ignore patterns.
   pub fn is_ignored(&self, path: &Path) -> bool {
      let Ok(relative) = path.strip_prefix(&self.root) else {
         return true;
      };

      let mut active: Vec<ScopedMatcher<'_>> = Vec::new();
      if let Some(ref matcher) = self.root_matcher {
         active.push(ScopedMatcher { root: self.root.clone(), matcher });
      }

      let mut blocked = false;
      if let Some(parent) = relative.parent() {
         let mut abs_dir = self.root.clone();
         for comp in parent.components() {
            abs_dir.push(comp);

            let dir_state = apply_matchers(&active, &abs_dir, true);
            blocked = match dir_state {
               MatchState::Ignore => true,
               MatchState::Whitelist => false,
               MatchState::None => blocked,
            };

            if !blocked {
               if let Some(gi) = self.dir_matchers.get(&abs_dir) {
                  active.push(ScopedMatcher { root: abs_dir.clone(), matcher: gi });
               }
            }
         }
      }

      if blocked {
         return true;
      }

      match apply_matchers(&active, path, path.is_dir()) {
         MatchState::Ignore => true,
         MatchState::Whitelist => false,
         MatchState::None => false,
      }
   }
}

pub(crate) fn collect_ignore_files(root: &Path) -> Vec<PathBuf> {
   let mut files = Vec::new();
   let walker = WalkDir::new(root)
      .follow_links(false)
      .into_iter()
      .filter_entry(|entry| {
         if !entry.file_type().is_dir() {
            return true;
         }
         let name = entry.file_name().to_string_lossy();
         if DEFAULT_IGNORE_DIRS.iter().any(|d| *d == name) {
            return false;
         }
         true
      });

   for entry in walker.filter_map(|e| e.ok()) {
      if !entry.file_type().is_file() {
         continue;
      }
      if let Some(name) = entry.file_name().to_str()
         && IGNORE_FILENAMES.iter().any(|f| *f == name)
      {
         files.push(entry.path().to_path_buf());
      }
   }

   files
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MatchState {
   None,
   Ignore,
   Whitelist,
}

struct ScopedMatcher<'a> {
   root:    PathBuf,
   matcher: &'a Gitignore,
}

fn apply_matchers(matchers: &[ScopedMatcher<'_>], path: &Path, is_dir: bool) -> MatchState {
   let mut state = MatchState::None;
   for scoped in matchers {
      let Ok(relative) = path.strip_prefix(&scoped.root) else {
         continue;
      };
      let m = scoped.matcher.matched(relative, is_dir);
      if m.is_ignore() {
         state = MatchState::Ignore;
      } else if m.is_whitelist() {
         state = MatchState::Whitelist;
      }
   }
   state
}

#[cfg(test)]
mod tests {
   use std::fs;

   use tempfile::TempDir;

   use super::*;

   #[test]
   fn default_patterns_loaded() {
      let tmp = TempDir::new().unwrap();
      let ignore = IgnorePatterns::new(tmp.path());

      fs::create_dir_all(tmp.path().join("node_modules")).unwrap();
      fs::create_dir_all(tmp.path().join("dist")).unwrap();
      fs::create_dir_all(tmp.path().join("src")).unwrap();

      let node_modules = tmp.path().join("node_modules").join("package");
      let dist = tmp.path().join("dist").join("main.js");
      let src = tmp.path().join("src").join("main.rs");

      fs::write(&node_modules, "").unwrap();
      fs::write(&dist, "").unwrap();
      fs::write(&src, "").unwrap();

      assert!(ignore.is_ignored(&node_modules));
      assert!(ignore.is_ignored(&dist));
      assert!(!ignore.is_ignored(&src));
   }

   #[test]
   fn glob_patterns_work() {
      let tmp = TempDir::new().unwrap();
      let ignore = IgnorePatterns::new(tmp.path());
      let min_js = tmp.path().join("test.min.js");
      let min_css = tmp.path().join("bundle.min.css");
      let map = tmp.path().join("app.js.map");
      let normal_js = tmp.path().join("app.js");
      assert!(ignore.is_ignored(&min_js));
      assert!(ignore.is_ignored(&min_css));
      assert!(ignore.is_ignored(&map));
      assert!(!ignore.is_ignored(&normal_js));
   }

   #[test]
   fn negation_patterns_work() {
      let tmp = TempDir::new().unwrap();

      let ignore_file = tmp.path().join(".ggignore");
      fs::write(&ignore_file, "*.log\n!important.log\n").unwrap();

      let ignore = IgnorePatterns::new(tmp.path());

      let test_log = tmp.path().join("test.log");
      let important_log = tmp.path().join("important.log");
      fs::write(&test_log, "").unwrap();
      fs::write(&important_log, "").unwrap();

      assert!(ignore.is_ignored(&test_log));
      assert!(!ignore.is_ignored(&important_log));
   }

   #[test]
   fn comment_patterns_ignored() {
      let tmp = TempDir::new().unwrap();

      let ignore_file = tmp.path().join(".ggignore");
      fs::write(&ignore_file, "# This is a comment\n*.tmp\n# Another comment\n").unwrap();

      let ignore = IgnorePatterns::new(tmp.path());

      let tmp_file = tmp.path().join("test.tmp");
      fs::write(&tmp_file, "").unwrap();

      assert!(ignore.is_ignored(&tmp_file));
   }

   #[test]
   fn anchored_patterns_work() {
      let tmp = TempDir::new().unwrap();

      let ignore_file = tmp.path().join(".ggignore");
      fs::write(&ignore_file, "/root.config\n").unwrap();

      let ignore = IgnorePatterns::new(tmp.path());

      let root_file = tmp.path().join("root.config");
      let nested_file = tmp.path().join("nested").join("root.config");
      fs::write(&root_file, "").unwrap();
      fs::create_dir(tmp.path().join("nested")).unwrap();
      fs::write(&nested_file, "").unwrap();

      assert!(ignore.is_ignored(&root_file));
      assert!(!ignore.is_ignored(&nested_file));
   }

   #[test]
   fn double_star_patterns_work() {
      let tmp = TempDir::new().unwrap();

      let ignore_file = tmp.path().join(".ggignore");
      fs::write(&ignore_file, "**/generated/**\n").unwrap();

      let ignore = IgnorePatterns::new(tmp.path());

      let generated_dir = tmp.path().join("src").join("generated");
      fs::create_dir_all(&generated_dir).unwrap();
      let generated_file = generated_dir.join("code.ts");
      fs::write(&generated_file, "").unwrap();

      assert!(ignore.is_ignored(&generated_file));
   }

   #[test]
   fn respects_gitignore() {
      let tmp = TempDir::new().unwrap();

      let gitignore_file = tmp.path().join(".gitignore");
      fs::write(&gitignore_file, "*.secret\n").unwrap();

      let ignore = IgnorePatterns::new(tmp.path());

      let secret_file = tmp.path().join("passwords.secret");
      fs::write(&secret_file, "").unwrap();

      assert!(ignore.is_ignored(&secret_file));
   }

   #[test]
   fn nested_gitignore_patterns_apply() {
      let tmp = TempDir::new().unwrap();

      let nested = tmp.path().join("src");
      fs::create_dir_all(&nested).unwrap();
      fs::write(nested.join(".gitignore"), "*.gen\n").unwrap();

      let generated = nested.join("auto.gen");
      let regular = nested.join("main.rs");
      fs::write(&generated, "").unwrap();
      fs::write(&regular, "").unwrap();

      let ignore = IgnorePatterns::new(tmp.path());
      assert!(ignore.is_ignored(&generated));
      assert!(!ignore.is_ignored(&regular));
   }

   #[test]
   fn nested_gitignore_negation_overrides_parent() {
      let tmp = TempDir::new().unwrap();

      fs::write(tmp.path().join(".gitignore"), "*.log\n").unwrap();

      let logs_dir = tmp.path().join("logs");
      fs::create_dir_all(&logs_dir).unwrap();
      fs::write(logs_dir.join(".gitignore"), "!keep.log\n").unwrap();

      let keep = logs_dir.join("keep.log");
      let drop = logs_dir.join("drop.log");
      fs::write(&keep, "").unwrap();
      fs::write(&drop, "").unwrap();

      let ignore = IgnorePatterns::new(tmp.path());
      assert!(!ignore.is_ignored(&keep));
      assert!(ignore.is_ignored(&drop));
   }

   #[test]
   fn nested_ggignore_scopes_to_directory() {
      let tmp = TempDir::new().unwrap();

      let gen_dir = tmp.path().join("gen");
      fs::create_dir_all(&gen_dir).unwrap();
      fs::write(gen_dir.join(".ggignore"), "*.tmp\n").unwrap();

      let nested_tmp = gen_dir.join("skip.tmp");
      let root_tmp = tmp.path().join("skip.tmp");
      fs::write(&nested_tmp, "").unwrap();
      fs::write(&root_tmp, "").unwrap();

      let ignore = IgnorePatterns::new(tmp.path());
      assert!(ignore.is_ignored(&nested_tmp));
      assert!(!ignore.is_ignored(&root_tmp));
   }
}
