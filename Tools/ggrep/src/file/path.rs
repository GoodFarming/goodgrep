//! Path normalization and safety helpers for indexing.

use std::{
   collections::VecDeque,
   ffi::OsString,
   fs, io,
   path::{Component, Path, PathBuf},
};

use crate::Result;

const MAX_SYMLINK_DEPTH: usize = 32;

#[derive(Debug, Clone)]
pub struct ResolvedPath {
   pub real_path:   PathBuf,
   pub path_key:    PathBuf,
   pub path_key_ci: String,
}

pub fn canonical_root(root: &Path) -> PathBuf {
   resolve_with_symlink_limit(root).unwrap_or_else(|_| root.to_path_buf())
}

pub fn resolve_candidate(root: &Path, candidate: &Path) -> Result<Option<ResolvedPath>> {
   let root = canonical_root(root);
   let candidate = if candidate.is_absolute() {
      candidate.to_path_buf()
   } else {
      root.join(candidate)
   };

   let real_path = match resolve_with_symlink_limit(&candidate) {
      Ok(path) => path,
      Err(e) => {
         if is_symlink_loop(&e) {
            tracing::warn!(
               "skipping path due to symlink loop or depth limit: {}",
               candidate.display()
            );
            return Ok(None);
         }
         tracing::warn!("skipping path due to resolution failure: {}", candidate.display());
         return Ok(None);
      },
   };

   if !real_path.starts_with(&root) {
      tracing::warn!(
         "skipping out-of-root path (resolved to {}): {}",
         real_path.display(),
         candidate.display()
      );
      return Ok(None);
   }

   let path_key = match path_key_from_real(&root, &real_path) {
      Some(key) => key,
      None => {
         tracing::warn!("skipping non-utf8 or invalid path key: {}", real_path.display());
         return Ok(None);
      },
   };

   let path_key_ci = casefold_path_key(&path_key).unwrap_or_default();

   Ok(Some(ResolvedPath { real_path, path_key, path_key_ci }))
}

pub fn path_key_from_real(root: &Path, real_path: &Path) -> Option<PathBuf> {
   let relative = real_path.strip_prefix(root).ok()?;
   normalize_relative(relative)
}

pub fn casefold_path_key(path_key: &Path) -> Option<String> {
   let key = path_key.to_str()?;
   Some(key.to_lowercase())
}

pub fn normalize_relative(path: &Path) -> Option<PathBuf> {
   let raw = path.to_str()?;
   let mut normalized = raw.replace('\\', "/");
   while normalized.starts_with("./") {
      normalized = normalized[2..].to_string();
   }

   if normalized.is_empty() {
      return None;
   }

   let normalized_path = Path::new(&normalized);
   for component in normalized_path.components() {
      if matches!(component, Component::ParentDir | Component::CurDir) {
         return None;
      }
   }

   Some(PathBuf::from(normalized))
}

fn resolve_with_symlink_limit(path: &Path) -> io::Result<PathBuf> {
   let mut remaining = VecDeque::new();
   let mut current = PathBuf::new();

   for component in path.components() {
      match component {
         Component::Prefix(prefix) => {
            current = PathBuf::from(prefix.as_os_str());
         },
         Component::RootDir => {
            current = PathBuf::from("/");
         },
         Component::CurDir => {},
         Component::ParentDir => {
            remaining.push_back(OsString::from(".."));
         },
         Component::Normal(name) => {
            remaining.push_back(name.to_os_string());
         },
      }
   }

   if current.as_os_str().is_empty() {
      current = std::env::current_dir()?;
   }

   let mut symlink_depth = 0usize;

   while let Some(part) = remaining.pop_front() {
      if part == "." {
         continue;
      }
      if part == ".." {
         current.pop();
         continue;
      }

      let candidate = current.join(&part);
      let meta = fs::symlink_metadata(&candidate)?;
      if meta.file_type().is_symlink() {
         symlink_depth += 1;
         if symlink_depth > MAX_SYMLINK_DEPTH {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "symlink depth limit exceeded"));
         }

         let target = fs::read_link(&candidate)?;
         let (base, target_components) = if target.is_absolute() {
            let comps = components_to_vec(&target);
            (PathBuf::from("/"), comps)
         } else {
            (current.clone(), components_to_vec(&target))
         };

         current = base;
         for comp in target_components.into_iter().rev() {
            remaining.push_front(comp);
         }
         continue;
      }

      current = candidate;
   }

   Ok(current)
}

fn components_to_vec(path: &Path) -> Vec<OsString> {
   path
      .components()
      .filter_map(|component| match component {
         Component::Normal(name) => Some(name.to_os_string()),
         Component::CurDir => Some(OsString::from(".")),
         Component::ParentDir => Some(OsString::from("..")),
         Component::RootDir | Component::Prefix(_) => None,
      })
      .collect()
}

fn is_symlink_loop(err: &io::Error) -> bool {
   match err.raw_os_error() {
      Some(code) => code == libc::ELOOP,
      None => err.kind() == io::ErrorKind::InvalidData,
   }
}

#[cfg(test)]
mod tests {
   use std::fs;

   use tempfile::TempDir;

   use super::*;

   #[test]
   fn normalize_relative_rejects_parent_segments() {
      assert!(normalize_relative(Path::new("../secret.txt")).is_none());
   }

   #[test]
   fn normalize_relative_strips_dot_prefix() {
      let normalized = normalize_relative(Path::new("./src/lib.rs")).unwrap();
      assert_eq!(normalized, PathBuf::from("src/lib.rs"));
   }

   #[test]
   fn resolve_candidate_skips_out_of_root_symlink() {
      let tmp = TempDir::new().unwrap();
      let root = tmp.path().join("root");
      let external = tmp.path().join("external.txt");
      fs::create_dir_all(&root).unwrap();
      fs::write(&external, "hello").unwrap();

      let link = root.join("link.txt");
      std::os::unix::fs::symlink(&external, &link).unwrap();

      let resolved = resolve_candidate(&root, &link).unwrap();
      assert!(resolved.is_none());
   }

   #[test]
   fn resolve_candidate_accepts_in_root_symlink() {
      let tmp = TempDir::new().unwrap();
      let root = tmp.path().join("root");
      fs::create_dir_all(&root).unwrap();

      let target = root.join("real.txt");
      fs::write(&target, "hello").unwrap();

      let link = root.join("alias.txt");
      std::os::unix::fs::symlink(&target, &link).unwrap();

      let resolved = resolve_candidate(&root, &link).unwrap().unwrap();
      assert_eq!(resolved.path_key, PathBuf::from("real.txt"));
   }

   #[test]
   fn resolve_candidate_rejects_symlink_loop() {
      let tmp = TempDir::new().unwrap();
      let root = tmp.path().join("root");
      fs::create_dir_all(&root).unwrap();

      let a = root.join("a");
      let b = root.join("b");
      std::os::unix::fs::symlink(&b, &a).unwrap();
      std::os::unix::fs::symlink(&a, &b).unwrap();

      let resolved = resolve_candidate(&root, &a).unwrap();
      assert!(resolved.is_none());
   }
}
