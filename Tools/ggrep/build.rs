use std::{env, fs::File, path::Path, process::Command};

fn main() {
   build_git_metadata();
   build_plugin_bundle();
}

fn build_git_metadata() {
   println!("cargo:rerun-if-changed=.git/HEAD");
   println!("cargo:rerun-if-changed=.git/refs/");

   let git_hash = Command::new("git")
      .args(["rev-parse", "--short", "HEAD"])
      .output()
      .ok()
      .filter(|o| o.status.success())
      .and_then(|o| String::from_utf8(o.stdout).ok())
      .map_or_else(|| "unknown".to_string(), |s| s.trim().to_string());

   let git_tag = Command::new("git")
      .args(["describe", "--tags", "--abbrev=0"])
      .output()
      .ok()
      .filter(|o| o.status.success())
      .and_then(|o| String::from_utf8(o.stdout).ok())
      .map(|s| s.trim().to_string())
      .unwrap_or_default();

   let git_dirty = Command::new("git")
      .args(["status", "--porcelain"])
      .output()
      .ok()
      .filter(|o| o.status.success())
      .is_some_and(|o| !o.stdout.is_empty());

   println!("cargo:rustc-env=GIT_HASH={git_hash}");
   println!("cargo:rustc-env=GIT_TAG={git_tag}");
   println!("cargo:rustc-env=GIT_DIRTY={}", if git_dirty { "true" } else { "false" });
}

fn build_plugin_bundle() {
   println!("cargo:rerun-if-changed=.claude-plugin");
   println!("cargo:rerun-if-changed=plugins");

   let out_dir = env::var("OUT_DIR").unwrap();
   let dest_path = Path::new(&out_dir).join("plugin-bundle.tar");

   let file = File::create(&dest_path).expect("Failed to create plugin-bundle.tar");
   let mut builder = tar::Builder::new(file);

   let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
   let root = Path::new(&manifest_dir);

   builder
      .append_dir_all(".claude-plugin", root.join(".claude-plugin"))
      .expect("Failed to add .claude-plugin");
   builder
      .append_dir_all("plugins/ggrep", root.join("plugins/ggrep"))
      .expect("Failed to add plugins/ggrep");
   builder.finish().expect("Failed to finish tar");
}
