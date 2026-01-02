<div align="center">
  <a href="https://github.com/GoodFarmingAI/goodgrep">
    <img src="assets/logo.png" alt="ggrep" width="128" height="128" />
  </a>
  <h1>ggrep</h1>
  <p><em>Semantic code search for coding agents.</em></p>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache%202.0-blue.svg" alt="License" /></a>
</div>

Natural-language search that works like `grep`. Fast, local, and built for coding agents.

- **Semantic:** Finds concepts ("where do transactions get created?"), not just strings.
- **CPU-First:** Runs on CPU by default (Phase II hardening targets Linux+CPU).
- **Local & Private:** 100% local embeddings. No API keys required.
- **Auto-Isolated:** Each repository gets its own index automatically.
- **On-Demand Grammars:** Tree-sitter WASM grammars download automatically as needed.
- **Agent-Ready:** Native MCP server and integrations for Claude, Codex, Gemini, and OpenCode.

## Quick Start

1. **Install**

   ```bash
   cargo install --path . --bin ggrep
   ```

   Or build from source:

   ```bash
   git clone https://github.com/GoodFarmingAI/goodgrep.git
   cd goodgrep/Tools/ggrep
   cargo build --release
   ```

   For optional CUDA builds (deferred):

   ```bash
   cargo build --release --features cuda
   ```

2. **Setup (Recommended)**

   ```bash
   ggrep setup
   ```

   Downloads embedding models (~500MB) and tree-sitter grammars upfront. If you skip this, models download automatically on first use.

3. **Search**

   ```bash
   cd my-repo
   ggrep "where do we handle authentication?"
   ```

   **Your first search will automatically index the repository.** Each repository is automatically isolated with its own index. Switching between repos "just works".

## Coding Agent Integration

### Claude Code

1. Run `ggrep claude-install`
2. Open Claude Code (`claude`) and ask questions about your codebase.
3. The plugin auto-starts the `ggrep serve` daemon and provides semantic search.

### Codex CLI

1. Run `ggrep codex-install`
2. Open Codex (`codex`) and use semantic search via MCP.

### Gemini CLI

1. Run `ggrep gemini-install`
2. Open Gemini (`gemini`) and use semantic search via MCP.

### OpenCode

1. Run `ggrep opencode-install`
2. Open OpenCode (`opencode`) and use semantic search via MCP.

### MCP Server

ggrep includes a built-in MCP (Model Context Protocol) server:

```bash
ggrep mcp
```

This exposes a `good_search` tool that agents can use for semantic code search. The server auto-starts the background daemon if needed.

## Commands

### `ggrep [query]`

The default command. Searches the current directory using semantic meaning.

```bash
ggrep "how is the database connection pooled?"
```

**Options:**
| Flag | Description | Default |
| --- | --- | --- |
| `-m <n>` | Max total results to return | `10` |
| `--per-file <n>` | Max matches per file | `1` |
| `-c`, `--content` | Show full chunk content | `false` |
| `--compact` | Show file paths only | `false` |
| `--scores` | Show relevance scores | `false` |
| `-s`, `--sync` | Force re-index before search | `false` |
| `--dry-run` | Show what would be indexed | `false` |
| `--json` | JSON output format | `false` |
| `--no-rerank` | Skip ColBERT reranking | `false` |
| `--plain` | Disable ANSI colors | `false` |

**Examples:**

```bash
# General concept search
ggrep "API rate limiting logic"

# Deep dive (more matches per file)
ggrep "error handling" --per-file 5

# Just the file paths
ggrep "user validation" --compact

# JSON for scripting
ggrep "config parsing" --json
```

### `ggrep index`

Manually indexes the repository.

```bash
ggrep index              # Index current dir
ggrep index --dry-run    # See what would be indexed
ggrep index --reset      # Delete and re-index from scratch
```

### `ggrep serve`

Runs a background daemon with file watching for instant searches.

- Keeps LanceDB and embedding models resident for fast responses
- Watches the repo and incrementally re-indexes on change
- Communicates via Unix socket (or TCP on Windows)

```bash
ggrep serve              # Start daemon for current repo
ggrep serve --path /repo # Start for specific path
```

### `ggrep stop` / `ggrep stop-all`

Stop running daemons.

```bash
ggrep stop               # Stop daemon for current repo
ggrep stop-all           # Stop all ggrep daemons
```

### `ggrep clean`

Remove index data and metadata for a store.

```bash
ggrep clean              # Clean current directory's store
ggrep clean my-store     # Clean specific store by ID
ggrep clean --all        # Clean all stores
```

### `ggrep status`

Show status of running daemons.

### `ggrep list`

Lists all indexed repositories and their metadata.

### `ggrep doctor`

Checks installation health, model availability, and grammar status.

```bash
ggrep doctor
```

## Build Profiles

Phase II hardening targets Linux+CPU as the baseline.

**CPU-only (default):**

```bash
cargo build --release
```

**Optional CUDA (deferred):**

```bash
cargo build --release --features cuda
```

If/when CUDA is enabled, `GGREP_DISABLE_GPU=1` can force CPU even when CUDA is available.

## Architecture

ggrep combines several techniques for high-quality semantic search:

1. **Smart Chunking:** Tree-sitter parses code by function/class boundaries, ensuring embeddings capture complete logical blocks. Grammars download on-demand as WASM modules.

2. **Hybrid Search:** Dense embeddings (sentence-transformers) for broad recall, ColBERT reranking for precision.

3. **Quantized Storage:** ColBERT embeddings are quantized to int8 for efficient storage in LanceDB.

4. **Automatic Repository Isolation:** Stores are named by git remote URL or directory hash.

5. **Incremental Indexing:** File watcher detects changes and updates only affected chunks.

**Supported languages (37):** TypeScript, TSX, JavaScript, Python, Go, Rust, C, C++, C#, Java, Kotlin, Scala, Ruby, PHP, Elixir, Haskell, OCaml, Julia, Zig, Lua, Odin, Objective-C, Verilog, HTML, CSS, XML, Markdown, JSON, YAML, TOML, Bash, Make, Starlark, HCL, Terraform, Diff, Regex

## Configuration

ggrep uses a TOML config file at `~/.ggrep/config.toml`. All options can also be set via environment variables with the `GGREP_` prefix.

### Config File

```toml
# ~/.ggrep/config.toml

# ============================================================================
# Models
# ============================================================================

# Dense embedding model (HuggingFace model ID)
# Used for initial semantic similarity search
dense_model = "ibm-granite/granite-embedding-small-english-r2"

# ColBERT reranking model (HuggingFace model ID)
# Used for precise reranking of search results
colbert_model = "answerdotai/answerai-colbert-small-v1"

# Model dimensions (must match the models above)
dense_dim = 384
colbert_dim = 96

# Query prefix (some models require a prefix like "query: ")
query_prefix = ""

# Maximum sequence lengths for tokenization
dense_max_length = 256
colbert_max_length = 256

# ============================================================================
# Performance
# ============================================================================

# Batch size for embedding computation
# Higher = faster but more memory. Auto-reduces on OOM.
default_batch_size = 48
max_batch_size = 96

# Maximum threads for parallel processing
max_threads = 32

# Force CPU inference even when CUDA is available
disable_gpu = false

# Low-impact mode: reduces resource usage for background indexing
low_impact = false

# Fast mode: skip ColBERT reranking for quicker (but less precise) results
fast_mode = false

# ============================================================================
# Server
# ============================================================================

# TCP port for daemon communication
port = 4444

# Idle timeout: shutdown daemon after this many seconds of inactivity
idle_timeout_secs = 1800  # 30 minutes

# How often to check for idle timeout
idle_check_interval_secs = 60

# Timeout for embedding worker operations (milliseconds)
worker_timeout_ms = 60000

# ============================================================================
# Debug
# ============================================================================

# Enable model loading debug output
debug_models = false

# Enable embedding debug output
debug_embed = false

# Enable profiling
profile_enabled = false

# Skip saving metadata (for testing)
skip_meta_save = false
```

### Environment Variables

Any config option can be set via environment variable with the `GGREP_` prefix:

```bash
# Examples
export GGREP_DISABLE_GPU=true
export GGREP_DEFAULT_BATCH_SIZE=24
export GGREP_IDLE_TIMEOUT_SECS=3600
```

| Variable                    | Description           | Default       |
| --------------------------- | --------------------- | ------------- |
| `GGREP_STORE`               | Override store name   | auto-detected |
| `GGREP_DISABLE_GPU`         | Force CPU inference   | `false`       |
| `GGREP_DEFAULT_BATCH_SIZE`  | Embedding batch size  | `48`          |
| `GGREP_LOW_IMPACT`          | Reduce resource usage | `false`       |
| `GGREP_FAST_MODE`           | Skip reranking        | `false`       |

### Ignoring Files

ggrep respects `.gitignore` and `.ggignore` files (legacy `.smignore` is also supported).

Create `.ggignore` in your repository root:

```
# Ignore generated files
dist/
*.min.js

# Ignore test fixtures
test/fixtures/
```

### Manual Store Management

- **View all stores:** `ggrep list`
- **Override auto-detection:** `ggrep --store custom-name "query"`
- **Data location:** `~/.ggrep/`

## Troubleshooting

- **Index feels stale?** Run `ggrep index` to refresh.
- **Weird results?** Run `ggrep doctor` to verify models and grammars.
- **Need a fresh start?** `ggrep index --reset` or delete `~/.ggrep/`.
- **GPU OOM?** Batch size auto-reduces, or set `GGREP_DISABLE_GPU=1`.

## Building from Source

```bash
git clone https://github.com/GoodFarmingAI/goodgrep.git
cd goodgrep/Tools/ggrep
cargo build --release

# Run tests
cargo test
```

## Acknowledgments

ggrep is inspired by [osgrep](https://github.com/Ryandonofrio3/osgrep) and [mgrep](https://github.com/mixedbread-ai/mgrep) by MixedBread.

## License

Licensed under the Apache License, Version 2.0.
See [LICENSE](LICENSE) for details.
