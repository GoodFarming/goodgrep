//! Version information from build metadata

/// Package version from Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Git commit hash at build time
pub const GIT_HASH: &str = env!("GIT_HASH");

/// Git tag at build time
pub const GIT_TAG: &str = env!("GIT_TAG");

/// Whether the working tree was dirty at build time
pub const GIT_DIRTY: &str = env!("GIT_DIRTY");

/// Returns a formatted version string with git information
pub fn version_string() -> String {
   let dirty = if GIT_DIRTY == "true" { "-dirty" } else { "" };
   if GIT_TAG.is_empty() {
      format!("{VERSION} ({GIT_HASH}{dirty})")
   } else {
      format!("{VERSION} ({GIT_TAG}, {GIT_HASH}{dirty})")
   }
}
