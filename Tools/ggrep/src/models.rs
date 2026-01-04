//! Model identifier helpers (Hugging Face model IDs + revisions).

use hf_hub::{Repo, RepoType};

/// Splits a model id of the form "org/name@rev" into ("org/name", Some("rev")).
/// If no revision is present, returns (model_id, None).
pub fn split_model_id(model_id: &str) -> (&str, Option<&str>) {
   if let Some((name, rev)) = model_id.split_once('@') {
      if rev.trim().is_empty() {
         return (model_id, None);
      }
      return (name, Some(rev));
   }
   (model_id, None)
}

/// Builds a Hugging Face repo spec for a model id, preserving any pinned
/// revision.
pub fn repo_for_model(model_id: &str) -> Repo {
   let (model_name, revision) = split_model_id(model_id);
   match revision {
      Some(rev) => Repo::with_revision(model_name.to_string(), RepoType::Model, rev.to_string()),
      None => Repo::new(model_name.to_string(), RepoType::Model),
   }
}
