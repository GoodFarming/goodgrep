<div align="center">
  <img src="Tools/ggrep/assets/logo.png" alt="ggrep" width="128" height="128" />
  <h1>ggrep</h1>
  <p><em>Semantic code search for coding agents.</em></p>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache%202.0-blue.svg" alt="License" /></a>
</div>

---

Natural-language search that works like `grep`. Fast, local, and built for coding agents.

## Why ggrep?

Traditional code search (grep, ripgrep, IDE search) finds exact text matches. But when you're exploring a codebase, you often think in concepts: *"where is authentication handled?"* or *"how does the rate limiter work?"*

ggrep bridges this gap:

- **Semantic search**: Find code by meaning, not just string matching. Ask *"where do transactions get created?"* and get results even if the code uses `create_txn` or `new_transaction`.
- **CPU-first**: Runs entirely on CPU. No GPU required, no cloud APIs, no API keys.
- **100% local**: All embeddings computed locally. Your code never leaves your machine.
- **Language-aware chunking**: Tree-sitter parses code by function/class boundaries, so each result is a complete, meaningful unit.
- **Agent-ready**: Native MCP server for Claude Code, Codex CLI, Gemini CLI, and OpenCode.

## Quick Start

**Install from source:**

```bash
git clone https://github.com/GoodFarming/goodgrep.git
cd goodgrep/Tools/ggrep
cargo build --release
```

The binary will be at `target/release/ggrep`. Add it to your PATH or run directly.

**First-time setup (optional):**

```bash
ggrep setup
```

Downloads embedding models (~500MB) and tree-sitter grammars upfront. If you skip this, models download automatically on first use.

**Search a codebase:**

```bash
cd /path/to/your/repo
ggrep "where is authentication handled?"
```

Your first search automatically indexes the repository. Each repository gets its own isolated index.

## How It Works

ggrep combines several techniques for high-quality semantic search:

1. **Smart Chunking**: Tree-sitter parses code by function/class boundaries, ensuring each embedding captures a complete logical block. Markdown is chunked by headings. Mermaid diagrams are preprocessed for better recall.

2. **Hybrid Search**: Dense embeddings (sentence-transformers) for broad semantic recall, plus ColBERT reranking for precision on top candidates.

3. **Snapshot Isolation**: Queries always see a consistent view of the index, never partial state during updates.

4. **Background Daemon**: File watcher detects changes and incrementally re-indexes. Keep `ggrep serve` running for instant searches.

5. **Per-Repository Isolation**: Each repository gets its own index, identified by git remote URL or directory hash. Switching repos "just works".

### Supported Languages (37)

TypeScript, TSX, JavaScript, Python, Go, Rust, C, C++, C#, Java, Kotlin, Scala, Ruby, PHP, Elixir, Haskell, OCaml, Julia, Zig, Lua, Odin, Objective-C, Verilog, HTML, CSS, XML, Markdown, JSON, YAML, TOML, Bash, Make, Starlark, HCL, Terraform, Diff, Regex

## Commands

### Search

```bash
# Quick search (shorthand)
ggrep "how is the database connection pooled?"

# Full control with ggrep search
ggrep search "API rate limiting logic"
ggrep search --per-file 5 "error handling"      # More results per file
ggrep search --compact "user validation"         # File paths only
ggrep search --json "config parsing"             # JSON output for scripting
```

**Search modes** (bias results toward different content types):

| Flag | Mode | Best for |
|------|------|----------|
| `-d` | Discovery | Broad exploration across code, docs, and diagrams |
| `-i` | Implementation | Code-focused results |
| `-p` | Planning | Docs and diagrams |
| `-b` | Debug | Debugging and incident-related code |

**Output control:**

| Flag | Effect |
|------|--------|
| `-n`, `--no-snippet` | File + line only |
| `-s`, `--short-snippet` | Short preview |
| `-l`, `--long-snippet` | Longer preview |
| `-c`, `--content` | Full chunk content |
| `--compact` | File paths only (deduplicated) |

### Indexing

```bash
ggrep index              # Index current directory
ggrep index --dry-run    # Preview what would be indexed
ggrep index --reset      # Delete and rebuild from scratch
```

### Daemon

```bash
ggrep serve              # Start background daemon (file watching + fast searches)
ggrep stop               # Stop daemon for current repo
ggrep stop-all           # Stop all ggrep daemons
```

### Status and Maintenance

```bash
ggrep status             # Show daemon and index status
ggrep health             # Check system health
ggrep list               # List all indexed repositories
ggrep doctor             # Verify models and grammars
ggrep gc                 # Clean up old snapshots
ggrep compact            # Merge index segments
```

## AI Agent Integration

ggrep includes a built-in MCP (Model Context Protocol) server for seamless integration with coding agents.

### Claude Code

```bash
ggrep claude-install
```

Then open Claude Code (`claude`). The ggrep plugin auto-starts and provides semantic search.

### Codex CLI

```bash
ggrep codex-install
```

### Gemini CLI

```bash
ggrep gemini-install
```

### OpenCode

```bash
ggrep opencode-install
```

### MCP Server (Manual)

```bash
ggrep mcp
```

Exposes MCP tools:
- `search`: Semantic search (returns JSON matching `ggrep search --json`)
- `ggrep_status`: Index and daemon status
- `ggrep_health`: System health checks

## Configuration

ggrep uses `~/.ggrep/config.toml` for global settings. All options can also be set via `GGREP_*` environment variables.

### Key Options

```toml
# Performance
default_batch_size = 48      # Embedding batch size (auto-reduces on OOM)
max_threads = 32             # Parallel processing threads
disable_gpu = false          # Force CPU even when CUDA available

# Daemon
port = 4444                  # TCP port for daemon
idle_timeout_secs = 1800     # Shutdown after 30 min idle
```

### File Ignoring

ggrep respects `.gitignore` and also reads `.ggignore` files:

```
# .ggignore example
dist/
*.min.js
test/fixtures/
```

## Project Status

ggrep is in active development. The current release (Phase II) provides:

- Reliable snapshot isolation (queries never see partial index state)
- Crash-safe atomic updates
- Multi-daemon operation for different repositories
- Query admission control and timeouts
- Maintenance commands (gc, compact, audit, repair)

**Coming in Phase III**: Structured "slate" output for agents (file-grouped results with evidence), progressive disclosure, confidence-aware ranking, and MCP parity with CLI features.

## Repository Structure

```
goodgrep/
├── Tools/ggrep/           # Main ggrep source code and documentation
│   ├── src/               # Rust source
│   ├── Docs/              # Specs, plans, and research
│   └── tests/             # Test suites
├── Scripts/ggrep/         # Helper scripts
├── Datasets/ggrep/        # Evaluation test cases
└── README.md              # This file
```

## Building from Source

**Requirements:**
- Rust (nightly recommended for best performance)
- ~500MB disk space for models (downloaded on first use)

```bash
git clone https://github.com/GoodFarming/goodgrep.git
cd goodgrep/Tools/ggrep

# Standard build
cargo build --release

# Run tests
cargo test

# Install to cargo bin directory
cargo install --path .
```

**Optional CUDA support** (for GPU acceleration):

```bash
cargo build --release --features cuda
```

## Troubleshooting

- **Index feels stale?** Run `ggrep index` to refresh.
- **Weird results?** Run `ggrep doctor` to verify models and grammars.
- **Need a fresh start?** Run `ggrep index --reset` or delete `~/.ggrep/`.
- **GPU OOM?** Batch size auto-reduces, or set `GGREP_DISABLE_GPU=1`.

## Acknowledgments

ggrep is inspired by [osgrep](https://github.com/Ryandonofrio3/osgrep) and [mgrep](https://github.com/mixedbread-ai/mgrep) by MixedBread.

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.
