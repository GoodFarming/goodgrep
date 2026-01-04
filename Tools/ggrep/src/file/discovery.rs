//! File discovery for local file systems and git repositories.

use std::{
   fs,
   path::{Path, PathBuf},
   process::Command,
};

use git2::Repository;

use crate::{
   config,
   error::{Error, Result},
   file::{IgnorePatterns, ResolvedPath, resolve_candidate},
   grammar::EXTENSION_MAP,
};

/// Additional extensions for text-based files without tree-sitter grammar
/// support. Extensions with grammar support are derived from [`EXTENSION_MAP`].
const ADDITIONAL_EXTENSIONS: &[&str] = &[
   "swift",
   "vue",
   "svelte",
   "txt",
   "sql",
   "zsh",
   "dockerfile",
   "el",
   "clj",
   "cljs",
   "cljc",
   "edn",
   "dart",
   "f90",
   "f95",
   "f03",
   "f08",
   "env",
   "gitignore",
   "gradle",
   "cmake",
   "proto",
   "graphql",
   "gql",
   "r",
   "R",
   "nim",
   "cr",
   "mmd",
   "mermaid",
];

/// Abstraction for file system operations to discover source files.
pub trait FileSystem {
   /// Returns an iterator of all discoverable files under the given root path.
   fn get_files(&self, root: &Path) -> Result<Box<dyn Iterator<Item = ResolvedPath>>>;
}

/// Local file system implementation that discovers files via git or directory
/// traversal.
pub struct LocalFileSystem;

impl LocalFileSystem {
   pub const fn new() -> Self {
      Self
   }

   fn is_supported_extension(path: &Path) -> bool {
      let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
      let filename = path.file_name().and_then(|f| f.to_str()).unwrap_or("");

      // Check grammar-supported extensions first
      EXTENSION_MAP.iter().any(|(e, _)| ext.eq_ignore_ascii_case(e))
         // Then additional text-based extensions
         || ADDITIONAL_EXTENSIONS.iter().any(|&e| ext.eq_ignore_ascii_case(e))
         // Special filename patterns
         || filename.eq_ignore_ascii_case("dockerfile")
         || filename.eq_ignore_ascii_case("makefile")
   }

   fn should_include_file(path: &Path, metadata: Option<&fs::Metadata>) -> bool {
      let max_file_size = config::get().effective_max_file_size_bytes();
      if !Self::is_supported_extension(path) {
         return false;
      }

      if let Some(filename) = path.file_name().and_then(|f| f.to_str())
         && filename.starts_with('.')
      {
         return false;
      }

      // Check file size if metadata provided, otherwise check via fs
      match metadata {
         Some(m) => m.len() <= max_file_size,
         None => fs::metadata(path)
            .map(|m| m.len() <= max_file_size)
            .unwrap_or(true),
      }
   }

   fn get_git_files(root: &Path) -> Result<Vec<PathBuf>> {
      let repo = Repository::discover(root).map_err(Error::OpenRepository)?;

      let mut files = Vec::new();

      let index = repo.index().map_err(Error::ReadIndex)?;

      let root_abs = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
      let repo_git_dir = repo.path().to_path_buf();
      let repo_git_dir = repo_git_dir.canonicalize().unwrap_or(repo_git_dir);
      let repo_root_from_git_dir = repo_git_dir
         .parent()
         .map(PathBuf::from)
         .unwrap_or_else(|| repo_git_dir.clone());

      let repo_root_abs = match repo.workdir() {
         Some(workdir) => {
            let workdir = workdir
               .canonicalize()
               .unwrap_or_else(|_| workdir.to_path_buf());
            if workdir.file_name() == Some(std::ffi::OsStr::new(".git")) {
               repo_root_from_git_dir
            } else {
               workdir
            }
         },
         None => repo_root_from_git_dir,
      };

      for entry in index.iter() {
         let path_bytes = entry.path.as_slice();
         if let Ok(path_str) = std::str::from_utf8(path_bytes) {
            let file_path = repo_root_abs.join(path_str);
            if file_path.exists()
               && file_path.starts_with(&root_abs)
               && Self::should_include_file(&file_path, None)
            {
               files.push(file_path);
            }
         }
      }

      if let Ok(output) = Command::new("git")
         .args(["ls-files", "--others", "--exclude-per-directory=.gitignore"])
         .current_dir(&repo_root_abs)
         .output()
         && output.status.success()
      {
         for line in String::from_utf8_lossy(&output.stdout).lines() {
            let file_path = repo_root_abs.join(line);
            if file_path.exists()
               && file_path.starts_with(&root_abs)
               && Self::should_include_file(&file_path, None)
            {
               files.push(file_path);
            }
         }
      }

      Ok(files)
   }

   fn is_git_repository(path: &Path) -> bool {
      path.join(".git").exists()
   }

   fn get_walkdir_files(root: &Path) -> Vec<PathBuf> {
      Self::get_walkdir_files_recursive(root, root)
   }

   fn get_walkdir_files_recursive(dir: &Path, root: &Path) -> Vec<PathBuf> {
      let mut files = Vec::new();

      let Ok(entries) = fs::read_dir(dir) else {
         return files;
      };

      for entry in entries.filter_map(|e| e.ok()) {
         let path = entry.path();

         if let Some(filename) = path.file_name().and_then(|f| f.to_str())
            && filename.starts_with('.')
         {
            continue;
         }

         let Ok(file_type) = entry.file_type() else {
            continue;
         };

         if file_type.is_dir() {
            if path != root && Self::is_git_repository(&path) {
               if let Ok(git_files) = Self::get_git_files(&path) {
                  files.extend(git_files);
               } else {
                  files.extend(Self::get_walkdir_files_recursive(&path, &path));
               }
            } else {
               files.extend(Self::get_walkdir_files_recursive(&path, root));
            }
         } else if (file_type.is_file() || file_type.is_symlink())
            && let Ok(metadata) = entry.metadata()
            && Self::should_include_file(&path, Some(&metadata))
         {
            files.push(path);
         }
      }

      files
   }
}

impl FileSystem for LocalFileSystem {
   fn get_files(&self, root: &Path) -> Result<Box<dyn Iterator<Item = ResolvedPath>>> {
      let files = if Repository::discover(root).is_ok() {
         Self::get_git_files(root)?
      } else {
         Self::get_walkdir_files(root)
      };

      let ignore_patterns = IgnorePatterns::new(root);
      let filtered: Vec<PathBuf> = files
         .into_iter()
         .filter(|p| !ignore_patterns.is_ignored(p))
         .collect();

      let resolved: Vec<ResolvedPath> = filtered
         .into_iter()
         .filter_map(|path| resolve_candidate(root, &path).ok().flatten())
         .collect();

      Ok(Box::new(resolved.into_iter()))
   }
}

impl Default for LocalFileSystem {
   fn default() -> Self {
      Self::new()
   }
}

#[cfg(test)]
mod tests {
   use super::*;

   #[test]
   fn supported_extension_recognized() {
      assert!(LocalFileSystem::is_supported_extension(Path::new("test.rs")));
      assert!(LocalFileSystem::is_supported_extension(Path::new("test.ts")));
      assert!(LocalFileSystem::is_supported_extension(Path::new("test.py")));
      assert!(!LocalFileSystem::is_supported_extension(Path::new("test.bin")));
   }

   #[test]
   fn hidden_files_filtered() {
      assert!(!LocalFileSystem::should_include_file(Path::new(".hidden.rs"), None));
      assert!(LocalFileSystem::should_include_file(Path::new("visible.rs"), None));
   }
}
