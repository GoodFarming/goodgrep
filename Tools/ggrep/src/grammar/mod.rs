//! Tree-sitter grammar management and loading

use std::{
   path::{Path, PathBuf},
   time::Duration,
};

use sha2::{Digest, Sha256};
use tokio::fs;
use tree_sitter::{Language, Parser, WasmStore, wasmtime};

use crate::{
   config,
   error::{ChunkerError, ConfigError, Error, Result},
   util::ArtifactLock,
};

/// Language name and URL pair for grammar downloads
pub type GrammarPair = (&'static str, &'static str);

/// URLs for downloading tree-sitter WASM grammars
pub const GRAMMAR_URLS: &[GrammarPair] = &[
    // tree-sitter organization (official)
    ("typescript", "https://github.com/tree-sitter/tree-sitter-typescript/releases/download/v0.23.2/tree-sitter-typescript.wasm"),
    ("tsx",        "https://github.com/tree-sitter/tree-sitter-typescript/releases/download/v0.23.2/tree-sitter-tsx.wasm"),
    ("python",     "https://github.com/tree-sitter/tree-sitter-python/releases/download/v0.25.0/tree-sitter-python.wasm"),
    ("go",         "https://github.com/tree-sitter/tree-sitter-go/releases/download/v0.25.0/tree-sitter-go.wasm"),
    ("rust",       "https://github.com/tree-sitter/tree-sitter-rust/releases/download/v0.24.0/tree-sitter-rust.wasm"),
    ("javascript", "https://github.com/tree-sitter/tree-sitter-javascript/releases/download/v0.25.0/tree-sitter-javascript.wasm"),
    ("c",          "https://github.com/tree-sitter/tree-sitter-c/releases/download/v0.24.1/tree-sitter-c.wasm"),
    ("cpp",        "https://github.com/tree-sitter/tree-sitter-cpp/releases/download/v0.23.4/tree-sitter-cpp.wasm"),
    ("java",       "https://github.com/tree-sitter/tree-sitter-java/releases/download/v0.23.5/tree-sitter-java.wasm"),
    ("ruby",       "https://github.com/tree-sitter/tree-sitter-ruby/releases/download/v0.23.1/tree-sitter-ruby.wasm"),
    ("php",        "https://github.com/tree-sitter/tree-sitter-php/releases/download/v0.24.2/tree-sitter-php.wasm"),
    ("html",       "https://github.com/tree-sitter/tree-sitter-html/releases/download/v0.23.2/tree-sitter-html.wasm"),
    ("css",        "https://github.com/tree-sitter/tree-sitter-css/releases/download/v0.25.0/tree-sitter-css.wasm"),
    ("bash",       "https://github.com/tree-sitter/tree-sitter-bash/releases/download/v0.25.1/tree-sitter-bash.wasm"),
    ("json",       "https://github.com/tree-sitter/tree-sitter-json/releases/download/v0.24.8/tree-sitter-json.wasm"),
    ("c_sharp",    "https://github.com/tree-sitter/tree-sitter-c-sharp/releases/download/v0.23.1/tree-sitter-c_sharp.wasm"),
    ("scala",      "https://github.com/tree-sitter/tree-sitter-scala/releases/download/v0.24.0/tree-sitter-scala.wasm"),
    ("haskell",    "https://github.com/tree-sitter/tree-sitter-haskell/releases/download/v0.23.1/tree-sitter-haskell.wasm"),
    ("ocaml",      "https://github.com/tree-sitter/tree-sitter-ocaml/releases/download/v0.24.2/tree-sitter-ocaml.wasm"),
    ("regex",      "https://github.com/tree-sitter/tree-sitter-regex/releases/download/v0.25.0/tree-sitter-regex.wasm"),
    ("julia",      "https://github.com/tree-sitter/tree-sitter-julia/releases/download/v0.25.0/tree-sitter-julia.wasm"),
    ("verilog",    "https://github.com/tree-sitter/tree-sitter-verilog/releases/download/v1.0.3/tree-sitter-verilog.wasm"),
    // tree-sitter-grammars organization
    ("zig",        "https://github.com/tree-sitter-grammars/tree-sitter-zig/releases/download/v1.1.2/tree-sitter-zig.wasm"),
    ("lua",        "https://github.com/tree-sitter-grammars/tree-sitter-lua/releases/download/v0.4.1/tree-sitter-lua.wasm"),
    ("yaml",       "https://github.com/tree-sitter-grammars/tree-sitter-yaml/releases/download/v0.7.2/tree-sitter-yaml.wasm"),
    ("toml",       "https://github.com/tree-sitter-grammars/tree-sitter-toml/releases/download/v0.7.0/tree-sitter-toml.wasm"),
    ("markdown",   "https://github.com/tree-sitter-grammars/tree-sitter-markdown/releases/download/v0.5.1/tree-sitter-markdown.wasm"),
    ("kotlin",     "https://github.com/tree-sitter-grammars/tree-sitter-kotlin/releases/download/v1.1.0/tree-sitter-kotlin.wasm"),
    ("make",       "https://github.com/tree-sitter-grammars/tree-sitter-make/releases/download/v1.1.1/tree-sitter-make.wasm"),
    ("objc",       "https://github.com/tree-sitter-grammars/tree-sitter-objc/releases/download/v3.0.2/tree-sitter-objc.wasm"),
    ("diff",       "https://github.com/tree-sitter-grammars/tree-sitter-diff/releases/download/v0.1.0/tree-sitter-diff.wasm"),
    ("xml",        "https://github.com/tree-sitter-grammars/tree-sitter-xml/releases/download/v0.7.0/tree-sitter-xml.wasm"),
    ("starlark",   "https://github.com/tree-sitter-grammars/tree-sitter-starlark/releases/download/v1.3.0/tree-sitter-starlark.wasm"),
    ("hcl",        "https://github.com/tree-sitter-grammars/tree-sitter-hcl/releases/download/v1.2.0/tree-sitter-hcl.wasm"),
    ("terraform",  "https://github.com/tree-sitter-grammars/tree-sitter-hcl/releases/download/v1.2.0/tree-sitter-terraform.wasm"),
    ("odin",       "https://github.com/tree-sitter-grammars/tree-sitter-odin/releases/download/v1.3.0/tree-sitter-odin.wasm"),
    // elixir-lang organization
    ("elixir",     "https://github.com/elixir-lang/tree-sitter-elixir/releases/download/v0.3.4/tree-sitter-elixir.wasm"),
];

/// Maps file extensions to language names
pub static EXTENSION_MAP: &[(&str, &str)] = &[
   ("js", "javascript"),
   ("mjs", "javascript"),
   ("cjs", "javascript"),
   ("ts", "typescript"),
   ("mts", "typescript"),
   ("cts", "typescript"),
   ("jsx", "tsx"),
   ("tsx", "tsx"),
   ("py", "python"),
   ("pyi", "python"),
   ("go", "go"),
   ("rs", "rust"),
   ("c", "c"),
   ("h", "c"),
   ("cpp", "cpp"),
   ("cc", "cpp"),
   ("cxx", "cpp"),
   ("c++", "cpp"),
   ("hpp", "cpp"),
   ("hxx", "cpp"),
   ("h++", "cpp"),
   ("java", "java"),
   ("rb", "ruby"),
   ("php", "php"),
   ("html", "html"),
   ("htm", "html"),
   ("css", "css"),
   ("sh", "bash"),
   ("bash", "bash"),
   ("json", "json"),
   ("cs", "c_sharp"),
   ("scala", "scala"),
   ("sc", "scala"),
   ("hs", "haskell"),
   ("lhs", "haskell"),
   ("ml", "ocaml"),
   ("mli", "ocaml"),
   ("zig", "zig"),
   ("lua", "lua"),
   ("yaml", "yaml"),
   ("yml", "yaml"),
   ("toml", "toml"),
   ("md", "markdown"),
   ("markdown", "markdown"),
   ("ex", "elixir"),
   ("exs", "elixir"),
   ("jl", "julia"),
   ("v", "verilog"),
   ("sv", "verilog"),
   ("svh", "verilog"),
   ("kt", "kotlin"),
   ("kts", "kotlin"),
   ("makefile", "make"),
   ("mk", "make"),
   ("m", "objc"),
   ("mm", "objc"),
   ("diff", "diff"),
   ("patch", "diff"),
   ("xml", "xml"),
   ("xsl", "xml"),
   ("xslt", "xml"),
   ("xsd", "xml"),
   ("bzl", "starlark"),
   ("star", "starlark"),
   ("hcl", "hcl"),
   ("tf", "terraform"),
   ("tfvars", "terraform"),
   ("odin", "odin"),
];

/// Manages downloading, caching, and loading tree-sitter grammars
pub struct GrammarManager {
   grammar_dir:   PathBuf,
   engine:        wasmtime::Engine,
   languages:     moka::future::Cache<&'static str, Language>,
   failed:        moka::future::Cache<&'static str, ()>,
   auto_download: bool,
}

impl std::fmt::Debug for GrammarManager {
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
      f.debug_struct("GrammarManager")
         .field("languages", &self.languages)
         .field("grammars_dir", &self.grammar_dir)
         .field("auto_download", &self.auto_download)
         .finish()
   }
}

impl GrammarManager {
   pub fn new() -> Result<Self> {
      let auto_download = !config::get().offline;
      Self::with_auto_download(auto_download)
   }

   pub fn with_auto_download(auto_download: bool) -> Result<Self> {
      let grammar_dir = config::grammar_dir();
      std::fs::create_dir_all(grammar_dir).map_err(ConfigError::CreateGrammarsDir)?;

      let engine = wasmtime::Engine::default();

      Ok(Self {
         grammar_dir: grammar_dir.clone(),
         engine,
         languages: moka::future::Cache::builder().max_capacity(32).build(),
         failed: moka::future::Cache::builder().max_capacity(128).build(),
         auto_download,
      })
   }

   /// Returns the directory where grammars are stored
   pub fn grammar_dir(&self) -> &Path {
      &self.grammar_dir
   }

   /// Converts a file extension to a language name
   pub fn extension_to_language(ext: &str) -> Option<&'static str> {
      EXTENSION_MAP
         .iter()
         .find(|(e, _)| e.eq_ignore_ascii_case(ext))
         .map(|(_, lang)| *lang)
   }

   /// Returns the download URL for a grammar by language name
   pub fn grammar_url(lang: &str) -> Option<&'static str> {
      GRAMMAR_URLS
         .iter()
         .find(|(l, _)| l.eq_ignore_ascii_case(lang))
         .map(|(_, url)| *url)
   }

   /// Returns the filesystem path for a grammar WASM file
   pub fn grammar_path(&self, lang: &str) -> PathBuf {
      self.grammar_dir.join(format!("tree-sitter-{lang}.wasm"))
   }

   /// Checks if a grammar is available locally
   pub fn is_available(&self, lang: &str) -> bool {
      self.grammar_path(lang).exists()
   }

   /// Returns an iterator of languages available locally
   pub fn available_languages(&self) -> impl Iterator<Item = &'static str> + Clone {
      GRAMMAR_URLS
         .iter()
         .filter(|(lang, _)| self.is_available(lang))
         .map(|(lang, _)| *lang)
   }

   /// Returns an iterator of languages not available locally
   pub fn missing_languages(&self) -> impl Iterator<Item = &'static str> + Clone {
      GRAMMAR_URLS
         .iter()
         .filter(|(lang, _)| !self.is_available(lang))
         .map(|(lang, _)| *lang)
   }

   fn load_language(&self, lang: &str, bytes: &[u8]) -> Result<Language> {
      let mut store = WasmStore::new(&self.engine).map_err(ChunkerError::CreateWasmStore)?;
      store
         .load_language(lang, bytes)
         .map_err(|e| ChunkerError::LoadLanguage { lang: lang.to_string(), reason: e }.into())
   }

   /// Downloads and loads a grammar, using cached version if available
   pub async fn download_grammar(&self, pair: GrammarPair) -> Result<Language> {
      let (lang, url) = pair;
      let dest = self.grammar_path(lang);
      if dest.exists() {
         let bytes = match fs::read(&dest).await {
            Ok(b) => b,
            Err(err) => {
               tracing::warn!("failed to read cached grammar {}: {}", dest.display(), err);
               Vec::new()
            },
         };
         if !bytes.is_empty() {
            if let Some(expected) = read_checksum(&dest).await {
               let actual = hex::encode(Sha256::digest(&bytes));
               if expected != actual {
                  tracing::warn!(
                     "grammar checksum mismatch for {} (expected {}, got {})",
                     dest.display(),
                     expected,
                     actual
                  );
                  let _ = fs::remove_file(&dest).await;
                  let _ = fs::remove_file(dest.with_extension("sha256")).await;
               } else if let Ok(language) = self.load_language(lang, &bytes) {
                  return Ok(language);
               }
            } else if let Ok(language) = self.load_language(lang, &bytes) {
               let _ = write_checksum(&dest, &bytes).await;
               return Ok(language);
            }
         }
      }

      if !self.auto_download {
         return Err(ConfigError::DownloadsDisabled { artifact: format!("grammar:{lang}") }.into());
      }

      if url.contains("/latest/") {
         tracing::warn!(
            "grammar '{}' uses a latest URL; consider pinning to a versioned release",
            lang
         );
      }

      let lock_path = dest.with_extension("lock");
      let _lock = ArtifactLock::acquire(&lock_path, Duration::from_secs(60)).await?;

      if dest.exists() {
         let language = fs::read(&dest)
            .await
            .map_err(Error::from)
            .and_then(|bytes| self.load_language(lang, &bytes));
         if let Ok(language) = language {
            return Ok(language);
         }
      }

      tracing::info!("downloading grammar for {} from {}", lang, url);

      let client = reqwest::Client::new();
      let response = client
         .get(url)
         .timeout(Duration::from_secs(20))
         .send()
         .await
         .map_err(|e| Error::Config(ConfigError::DownloadFailed { lang, reason: e }))?;

      if !response.status().is_success() {
         return Err(Error::Config(ConfigError::DownloadHttpStatus {
            lang,
            status: response.status().as_u16(),
         }));
      }

      let bytes = response.bytes().await.map_err(ConfigError::ReadResponse)?;

      if bytes.is_empty() {
         return Err(
            Error::Server {
               op:     "download",
               reason: format!("empty grammar payload for {lang}"),
            }
            .into(),
         );
      }

      tracing::info!("downloaded grammar for {}", lang);

      let language = self.load_language(lang, &bytes)?;

      let tmp_name = format!(
         "{}.tmp-{}",
         dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("grammar.wasm"),
         std::process::id()
      );
      let tmp_path = dest.with_file_name(tmp_name);

      fs::write(&tmp_path, &bytes)
         .await
         .map_err(ConfigError::WriteWasmFile)?;
      if let Err(err) = fs::rename(&tmp_path, &dest).await {
         if !dest.exists() {
            return Err(ConfigError::RenameWasmFile(err).into());
         }
         let _ = fs::remove_file(&tmp_path).await;
      }
      let _ = write_checksum(&dest, &bytes).await;

      Ok(language)
   }

   /// Gets a language by name, downloading if necessary
   pub async fn get_language(&self, lang: &str) -> Result<Option<Language>> {
      let pair = GRAMMAR_URLS
         .iter()
         .find(|(l, _)| l.eq_ignore_ascii_case(lang));
      let Some(pair) = pair else {
         return Ok(None);
      };

      if let Some(cached) = self.languages.get(&pair.0).await {
         return Ok(Some(cached));
      }

      if self.failed.get(&pair.0).await.is_some() {
         return Ok(None);
      }

      let language = match self.download_grammar(*pair).await {
         Ok(lang) => lang,
         Err(e) => {
            // Treat on-demand grammar downloads as best-effort: chunking will
            // fall back to simple chunking if a grammar cannot be fetched.
            tracing::warn!("failed to download grammar for {}: {}", pair.0, e);
            self.failed.insert(pair.0, ()).await;
            return Ok(None);
         },
      };

      self.languages.insert(pair.0, language.clone()).await;
      Ok(Some(language))
   }

   /// Gets a language for a file path based on its extension
   pub async fn get_language_for_path(&self, path: &Path) -> Result<Option<Language>> {
      let lang = path
         .extension()
         .and_then(|e| e.to_str())
         .and_then(Self::extension_to_language);
      let Some(lang) = lang else {
         return Ok(None);
      };
      self.get_language(lang).await
   }

   /// Creates a new parser and WASM store for parsing
   pub fn create_parser_with_store(&self) -> Result<(Parser, WasmStore)> {
      let parser = Parser::new();
      let store = WasmStore::new(&self.engine).map_err(ChunkerError::CreateWasmStore)?;
      Ok((parser, store))
   }
}

impl Default for GrammarManager {
   fn default() -> Self {
      Self::new().expect("failed to create grammar manager")
   }
}

async fn read_checksum(path: &Path) -> Option<String> {
   let checksum_path = path.with_extension("sha256");
   let Ok(content) = fs::read_to_string(checksum_path).await else {
      return None;
   };
   let trimmed = content.trim();
   if trimmed.is_empty() {
      None
   } else {
      Some(trimmed.to_string())
   }
}

async fn write_checksum(path: &Path, bytes: &[u8]) -> Result<()> {
   let checksum_path = path.with_extension("sha256");
   let digest = hex::encode(Sha256::digest(bytes));
   fs::write(checksum_path, digest)
      .await
      .map_err(ConfigError::WriteWasmFile)?;
   Ok(())
}
