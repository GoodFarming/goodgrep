//! Git repository utilities for store identification and file tracking

use std::{
   ffi::OsStr,
   path::{Path, PathBuf},
};

use git2::{Repository, Status, StatusOptions};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Checks if a path is a git repository
pub fn is_git_repo(path: &Path) -> bool {
   Repository::open(path).is_ok()
}

/// Returns the repository root directory
pub fn get_repo_root(path: &Path) -> Option<PathBuf> {
   let repo = Repository::discover(path).ok()?;

   let repo_git_dir = repo.path().to_path_buf();
   let repo_git_dir = repo_git_dir.canonicalize().unwrap_or(repo_git_dir);
   let repo_root_from_git_dir = repo_git_dir
      .parent()
      .map(PathBuf::from)
      .unwrap_or_else(|| repo_git_dir.clone());

   match repo.workdir() {
      Some(workdir) => {
         let workdir = workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.to_path_buf());
         if workdir.file_name() == Some(OsStr::new(".git")) {
            Some(repo_root_from_git_dir)
         } else {
            Some(workdir)
         }
      },
      None => Some(repo_root_from_git_dir),
   }
}

/// Returns the current HEAD commit SHA for a repo, if available.
pub fn get_head_sha(path: &Path) -> Option<String> {
   let repo = Repository::discover(path).ok()?;
   let head = repo.head().ok()?;
   let oid = head.target()?;
   Some(oid.to_string())
}

/// Returns whether the working tree is dirty (has any changes).
pub fn is_dirty(path: &Path) -> Option<bool> {
   let repo = Repository::discover(path).ok()?;
   let mut opts = StatusOptions::new();
   opts
      .include_untracked(true)
      .recurse_untracked_dirs(true)
      .include_ignored(false);
   let statuses = repo.statuses(Some(&mut opts)).ok()?;
   let dirty = statuses.iter().any(|e| e.status() != Status::CURRENT);
   Some(dirty)
}

/// Returns paths for untracked files (excluding ignored files).
pub fn untracked_paths(path: &Path) -> Option<Vec<PathBuf>> {
   let repo = Repository::discover(path).ok()?;
   let workdir = repo.workdir()?.to_path_buf();
   let mut opts = StatusOptions::new();
   opts
      .include_untracked(true)
      .recurse_untracked_dirs(true)
      .include_ignored(false)
      .include_unmodified(false);
   let statuses = repo.statuses(Some(&mut opts)).ok()?;
   let mut out = Vec::new();
   for entry in statuses.iter() {
      let status = entry.status();
      if status.is_wt_new() {
         if let Some(rel) = entry.path() {
            out.push(workdir.join(rel));
         }
      }
   }
   Some(out)
}

/// Returns the URL of the origin remote
pub fn get_remote_url(repo: &Repository) -> Option<String> {
   repo
      .find_remote("origin")
      .ok()
      .and_then(|remote| remote.url().map(|s| s.to_string()))
}

/// Returns the repository slug from the origin remote, if available.
pub fn resolve_repo_slug(path: &Path) -> Result<Option<String>> {
   let abs_path = path.canonicalize()?;

   if let Ok(repo) = Repository::discover(&abs_path)
      && let Some(remote_url) = get_remote_url(&repo)
      && let Some(store_id) = extract_owner_repo(&remote_url)
   {
      return Ok(Some(store_id));
   }

   Ok(None)
}

/// Returns all tracked files in the repository index
pub fn get_tracked_files(repo: &Repository) -> Result<Vec<PathBuf>> {
   let mut files = Vec::new();
   let index = repo.index().map_err(Error::ReadIndex)?;

   let workdir = repo
      .workdir()
      .ok_or_else(|| Error::NoWorkingDirectory(repo.path().to_path_buf()))?;

   for entry in index.iter() {
      let path_bytes = entry.path.as_slice();
      if let Ok(path_str) = std::str::from_utf8(path_bytes) {
         let file_path = workdir.join(path_str);
         if file_path.exists() {
            files.push(file_path);
         }
      }
   }

   Ok(files)
}

/// Resolves a store ID from a path, using git remote if available or directory
/// name and hash
pub fn resolve_store_id(path: &Path) -> Result<String> {
   let abs_path = path.canonicalize()?;

   if let Ok(repo) = Repository::discover(&abs_path)
      && let Some(remote_url) = get_remote_url(&repo)
      && let Some(store_id) = extract_owner_repo(&remote_url)
   {
      return Ok(store_id);
   }

   let dir_name = abs_path
      .file_name()
      .and_then(|n| n.to_str())
      .unwrap_or("unknown");

   let path_hash = compute_path_hash(&abs_path);

   Ok(format!("{}-{}", dir_name, &path_hash[..8]))
}

pub(crate) fn extract_owner_repo(url: &str) -> Option<String> {
   let url = url.trim_end_matches(".git");

   if let Some(path_part) = url.strip_prefix("https://github.com/") {
      let parts: Vec<&str> = path_part.split('/').collect();
      if parts.len() >= 2 {
         return Some(format!("{}-{}", parts[0], parts[1]));
      }
   }

   if let Some(path_part) = url.strip_prefix("git@github.com:") {
      let parts: Vec<&str> = path_part.split('/').collect();
      if parts.len() >= 2 {
         return Some(format!("{}-{}", parts[0], parts[1]));
      }
   }

   if url.contains("://")
      && let Some(path_start) = url.rfind('/')
      && let Some(path_before) = url[..path_start].rfind('/')
   {
      let owner = &url[path_before + 1..path_start];
      let repo = &url[path_start + 1..];
      if !owner.is_empty() && !repo.is_empty() {
         return Some(format!("{owner}-{repo}"));
      }
   }

   None
}

fn compute_path_hash(path: &Path) -> String {
   let mut hasher = Sha256::new();
   hasher.update(path.to_string_lossy().as_bytes());
   hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
   use super::*;

	   #[test]
	   fn extract_owner_repo_https() {
	      let url = "https://github.com/GoodFarming/goodgrep";
	      assert_eq!(extract_owner_repo(url), Some("GoodFarming-goodgrep".to_string()));
	   }

	   #[test]
	   fn extract_owner_repo_https_with_git() {
	      let url = "https://github.com/GoodFarming/goodgrep.git";
	      assert_eq!(extract_owner_repo(url), Some("GoodFarming-goodgrep".to_string()));
	   }

	   #[test]
	   fn extract_owner_repo_ssh() {
	      let url = "git@github.com:GoodFarming/goodgrep.git";
	      assert_eq!(extract_owner_repo(url), Some("GoodFarming-goodgrep".to_string()));
	   }

   #[test]
   fn path_hash_computed() {
      let path = Path::new("/tmp/test");
      let hash = compute_path_hash(path);
      assert_eq!(hash.len(), 64);
   }
}
