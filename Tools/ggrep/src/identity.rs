//! Repo identity and fingerprinting utilities.

use std::{
   fs,
   path::{Path, PathBuf},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
   Result, chunker,
   config::{self, Config},
   file::{canonical_root, ignore::collect_ignore_files, path_key_from_real},
   git, grammar, meta,
   types::SearchMode,
};

const CONFIG_FINGERPRINT_VERSION: &str = "config-fingerprint-v1";
const QUERY_FINGERPRINT_VERSION: &str = "query-fingerprint-v1";
const EMBED_CONFIG_FINGERPRINT_VERSION: &str = "embed-config-fingerprint-v1";
const STORE_ID_HASH_LEN: usize = 12;

#[derive(Debug, Clone)]
pub struct IndexFingerprints {
   pub config_fingerprint: String,
   pub ignore_fingerprint: String,
   pub repo_config_hash:   Option<String>,
}

#[derive(Debug, Clone)]
pub struct IndexIdentity {
   pub canonical_root:     PathBuf,
   pub store_id:           String,
   pub config_fingerprint: String,
   pub ignore_fingerprint: String,
   pub repo_config_hash:   Option<String>,
}

/// Resolves the canonical root plus fingerprints and store id for a path.
pub fn resolve_index_identity(path: &Path) -> Result<IndexIdentity> {
   let repo_root = git::get_repo_root(path).unwrap_or_else(|| path.to_path_buf());
   let canonical_root = canonical_root(&repo_root);

   config::init_for_root(&canonical_root);
   let fingerprints = compute_fingerprints(&canonical_root)?;
   let store_id = build_store_id(&canonical_root, &fingerprints.config_fingerprint)?;

   Ok(IndexIdentity {
      canonical_root,
      store_id,
      config_fingerprint: fingerprints.config_fingerprint,
      ignore_fingerprint: fingerprints.ignore_fingerprint,
      repo_config_hash: fingerprints.repo_config_hash,
   })
}

/// Computes config + ignore fingerprints for a repo root.
pub fn compute_fingerprints(root: &Path) -> Result<IndexFingerprints> {
   config::init_for_root(root);
   config::validate_repo_config(config::get())?;
   let repo_config_hash = compute_repo_config_hash(root)?;
   let config_fingerprint =
      compute_config_fingerprint_with_config(config::get(), repo_config_hash.as_deref())?;
   let ignore_fingerprint = compute_ignore_fingerprint(root)?;

   Ok(IndexFingerprints { config_fingerprint, ignore_fingerprint, repo_config_hash })
}

/// Computes the content hash of `.ggrep.toml` if present.
pub fn compute_repo_config_hash(root: &Path) -> Result<Option<String>> {
   let path = config::repo_config_path(root);
   if !path.exists() {
      return Ok(None);
   }
   let bytes = fs::read(path)?;
   Ok(Some(hex::encode(Sha256::digest(bytes))))
}

/// Computes the config fingerprint using effective config values.
pub fn compute_config_fingerprint_with_config(
   cfg: &Config,
   repo_config_hash: Option<&str>,
) -> Result<String> {
   let chunker = ChunkerFingerprint {
      max_lines:     chunker::MAX_LINES,
      max_chars:     chunker::MAX_CHARS,
      overlap_lines: chunker::OVERLAP_LINES,
      overlap_chars: chunker::OVERLAP_CHARS,
   };

   let grammar_urls_hash = hash_grammar_urls();

   let input = ConfigFingerprintInput {
      version: CONFIG_FINGERPRINT_VERSION,
      index_version: meta::INDEX_VERSION,
      chunker,
      embeddings: EmbeddingFingerprint {
         dense_model:        cfg.dense_model.as_str(),
         colbert_model:      cfg.colbert_model.as_str(),
         dense_dim:          cfg.dense_dim,
         colbert_dim:        cfg.colbert_dim,
         doc_prefix:         cfg.doc_prefix.as_str(),
         dense_max_length:   cfg.dense_max_length,
         colbert_max_length: cfg.colbert_max_length,
      },
      limits: LimitsFingerprint {
         max_file_size_bytes: cfg.effective_max_file_size_bytes(),
         max_chunks_per_file: cfg.effective_max_chunks_per_file(),
         max_bytes_per_sync:  cfg.effective_max_bytes_per_sync(),
      },
      repo_config_hash,
      grammar_urls_hash,
   };

   let payload = serde_json::to_vec(&input)?;
   Ok(hex::encode(Sha256::digest(payload)))
}

/// Computes the ignore fingerprint from `.gitignore`/`.ggignore` inputs.
pub fn compute_ignore_fingerprint(root: &Path) -> Result<String> {
   let root = canonical_root(root);
   let mut entries: Vec<(PathBuf, PathBuf)> = Vec::new();

   for path in collect_ignore_files(&root) {
      let Some(path_key) = path_key_from_real(&root, &path) else {
         tracing::warn!("skipping ignore file with invalid path key: {}", path.display());
         continue;
      };
      entries.push((path_key, path));
   }

   entries.sort_by(|a, b| a.0.as_os_str().cmp(b.0.as_os_str()));

   let mut hasher = Sha256::new();
   for (path_key, path) in entries {
      let bytes = fs::read(&path)?;
      hasher.update(path_key.to_string_lossy().as_bytes());
      hasher.update([0u8]);
      hasher.update(bytes);
   }

   Ok(hex::encode(hasher.finalize()))
}

pub struct QueryFingerprintOptions<'a> {
   pub mode:        SearchMode,
   pub max_results: usize,
   pub per_file:    usize,
   pub rerank:      bool,
   pub scope:       Option<&'a Path>,
   pub snippet:     &'a str,
}

pub fn compute_query_fingerprint(query: &str, opts: QueryFingerprintOptions<'_>) -> Result<String> {
   let scope = opts.scope.map(|p| p.to_string_lossy().to_string());
   let input = QueryFingerprintInput {
      version: QUERY_FINGERPRINT_VERSION,
      query,
      mode: opts.mode,
      max_results: opts.max_results,
      per_file: opts.per_file,
      rerank: opts.rerank,
      scope,
      snippet: opts.snippet,
   };
   let payload = serde_json::to_vec(&input)?;
   Ok(hex::encode(Sha256::digest(payload)))
}

pub fn compute_embed_config_fingerprint(cfg: &Config) -> Result<String> {
   let input = EmbedConfigFingerprintInput {
      version:            EMBED_CONFIG_FINGERPRINT_VERSION,
      dense_model:        cfg.dense_model.as_str(),
      colbert_model:      cfg.colbert_model.as_str(),
      dense_dim:          cfg.dense_dim,
      colbert_dim:        cfg.colbert_dim,
      query_prefix:       cfg.query_prefix.as_str(),
      doc_prefix:         cfg.doc_prefix.as_str(),
      dense_max_length:   cfg.dense_max_length,
      colbert_max_length: cfg.colbert_max_length,
   };
   let payload = serde_json::to_vec(&input)?;
   Ok(hex::encode(Sha256::digest(payload)))
}

fn build_store_id(root: &Path, config_fingerprint: &str) -> Result<String> {
   let base = git::resolve_repo_slug(root)?.unwrap_or_else(|| {
      root
         .file_name()
         .and_then(|n| n.to_str())
         .unwrap_or("unknown")
         .to_string()
   });
   let root_hash = hash_path(root);
   let root_hash = truncate_hash(&root_hash, STORE_ID_HASH_LEN);
   let cfg_hash = truncate_hash(config_fingerprint, STORE_ID_HASH_LEN);

   Ok(format!("{base}-{root_hash}-{cfg_hash}"))
}

fn hash_path(path: &Path) -> String {
   let mut hasher = Sha256::new();
   hasher.update(path.to_string_lossy().as_bytes());
   hex::encode(hasher.finalize())
}

fn truncate_hash(hash: &str, len: usize) -> String {
   if hash.len() <= len {
      hash.to_string()
   } else {
      hash[..len].to_string()
   }
}

fn hash_grammar_urls() -> String {
   let mut hasher = Sha256::new();
   for (lang, url) in grammar::GRAMMAR_URLS {
      hasher.update(lang.as_bytes());
      hasher.update([0u8]);
      hasher.update(url.as_bytes());
      hasher.update([0u8]);
   }
   hex::encode(hasher.finalize())
}

#[derive(Serialize)]
struct ConfigFingerprintInput<'a> {
   version:           &'static str,
   index_version:     &'static str,
   chunker:           ChunkerFingerprint,
   embeddings:        EmbeddingFingerprint<'a>,
   limits:            LimitsFingerprint,
   repo_config_hash:  Option<&'a str>,
   grammar_urls_hash: String,
}

#[derive(Serialize)]
struct ChunkerFingerprint {
   max_lines:     usize,
   max_chars:     usize,
   overlap_lines: usize,
   overlap_chars: usize,
}

#[derive(Serialize)]
struct EmbeddingFingerprint<'a> {
   dense_model:        &'a str,
   colbert_model:      &'a str,
   dense_dim:          usize,
   colbert_dim:        usize,
   doc_prefix:         &'a str,
   dense_max_length:   usize,
   colbert_max_length: usize,
}

#[derive(Serialize)]
struct LimitsFingerprint {
   max_file_size_bytes: u64,
   max_chunks_per_file: usize,
   max_bytes_per_sync:  u64,
}

#[derive(Serialize)]
struct QueryFingerprintInput<'a> {
   version:     &'static str,
   query:       &'a str,
   mode:        SearchMode,
   max_results: usize,
   per_file:    usize,
   rerank:      bool,
   scope:       Option<String>,
   snippet:     &'a str,
}

#[derive(Serialize)]
struct EmbedConfigFingerprintInput<'a> {
   version:            &'static str,
   dense_model:        &'a str,
   colbert_model:      &'a str,
   dense_dim:          usize,
   colbert_dim:        usize,
   query_prefix:       &'a str,
   doc_prefix:         &'a str,
   dense_max_length:   usize,
   colbert_max_length: usize,
}

#[cfg(test)]
mod tests {
   use tempfile::TempDir;

   use super::*;

   #[test]
   fn config_fingerprint_changes_with_repo_hash() {
      let cfg = Config::default();
      let fp1 = compute_config_fingerprint_with_config(&cfg, Some("abc")).unwrap();
      let fp2 = compute_config_fingerprint_with_config(&cfg, Some("def")).unwrap();
      assert_ne!(fp1, fp2);
   }

   #[test]
   fn ignore_fingerprint_changes_with_ignore_content() {
      let tmp = TempDir::new().unwrap();
      let root = tmp.path();

      fs::write(root.join(".gitignore"), "*.log\n").unwrap();
      let fp1 = compute_ignore_fingerprint(root).unwrap();

      fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();
      let fp2 = compute_ignore_fingerprint(root).unwrap();

      assert_ne!(fp1, fp2);
   }
}
