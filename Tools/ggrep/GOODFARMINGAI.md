# ggrep (GoodFarmingAI integration)

This repo hosts `ggrep`, originally developed in GoodFarmingAI, to improve recall across a hybrid corpus (code + planning docs + Mermaid).

## What’s different vs upstream

- **Docs/configs no longer “0-chunk”**: if tree-sitter finds no “definitions”, `ggrep` falls back to simple chunking so Markdown/YAML/JSON/TOML/etc remain searchable.
- **Markdown chunking by headings**: `.md`/`.mmd` is chunked by section headings so results return the relevant plan snippet.
- **Mermaid recall boost (index-time)**: Mermaid diagrams are summarized into a plain-text edge/message list for embedding **without modifying the stored snippet**.

## Install

CPU-only (recommended for local dev boxes without CUDA):

```bash
cd /home/adam/goodgrep/Tools/ggrep
cargo +nightly install --path . --no-default-features --bin ggrep --force
```

GoodFarmingAI fast config (CPU-friendly; avoids accidentally inheriting a large legacy config):

```bash
/home/adam/goodgrep/Scripts/ggrep/configure-fast.sh
```

### Build artifacts (disk usage)

Rust build artifacts can get large (`Tools/ggrep/target` was >10G). Prefer a cache directory outside the repo:

```bash
cd /home/adam/goodgrep/Tools/ggrep
/home/adam/goodgrep/Scripts/ggrep/cargo.sh +nightly install --path . --no-default-features --bin ggrep --force
```

Override the location if needed:

```bash
export GGREP_CARGO_TARGET_DIR="${HOME}/.cache/ggrep/target"
```

If a repo-local `Tools/ggrep/target` exists and disk is tight, it is safe to remove (build artifacts only).

## Use

Same UX as upstream, but the binary is `ggrep`:

```bash
ggrep "soil moisture decision irrigation"
ggrep search -m 50 --per-file 3 "where is this planned?" .
ggrep search -d -m 20 "where is irrigation gating planned?"
ggrep search -i -m 20 "where is irrigation gating implemented?"
ggrep search -p -m 20 "irrigation gating plan"
ggrep search -b -m 20 "irrigation scheduler stuck"
ggrep index --reset
ggrep search -n "plan for irrigation gating"
ggrep search -s "how does the scheduler decide irrigation?"
ggrep search -l "where is the irrigation decision documented?"
ggrep search --compact "soil moisture decision"   # file paths only (deduped)
```

## Agent integration (MCP)

Install ggrep into your preferred agent client:

```bash
ggrep claude-install
ggrep codex-install
ggrep gemini-install
ggrep opencode-install
```

### Intent modes (hybrid recall)

`ggrep` supports single-letter modes to bias results across **Code / Docs / Graphs** and prints a “context pack” grouped by bucket:

- `-d` discovery: balanced breadth (default for “where/how” questions)
- `-i` implementation: favors code
- `-p` planning: favors docs + diagrams
- `-b` debug: favors debugging/incident paths

### Snippet control

- `-n`: file + line only (no snippet)
- `-s`: short snippet preview
- `-l`: long snippet preview
- `-c`: full chunk content

### Search quality eval (recall/ranking)

Run the curated query suite and write a JSON report (for fine-tuning ranking/embedding tweaks). From the GoodFarmingAI repo root:

```bash
cd /path/to/GoodFarmingAI
ggrep eval --eval-store --out /tmp/ggrep-eval.json
```

Promote the evaluated `-eval` store to the canonical store id (so you don't re-index):

```bash
ggrep promote-eval --overwrite
```

If you already built the eval store and want to skip syncing:

```bash
ggrep eval --eval-store --no-sync --out /tmp/ggrep-eval.json
```

## Data/config location

`ggrep` now uses its own store so indexing changes don’t collide with other tools:

- Config: `~/.ggrep/config.toml` (seeded from legacy `~/.smgrep/config.toml` if present)
- Index: `~/.ggrep/data/<store-id>/`

After installing `ggrep`, run `ggrep index --reset` once to build the new store with the improved chunking + Mermaid augmentation.

### Config knobs (recall hardening)

- `query_prefix`: prepended to query text before embedding
- `doc_prefix`: prepended at **index time** for `Docs/diagrams` only (useful for embedding-model “prefix alignment”)
- Changing `dense_max_length`, `colbert_max_length`, or `doc_prefix` forces an index rebuild on the next `index`/`search`.

## Rollback

```bash
cargo uninstall ggrep
```

To revert indexing behavior, reinstall upstream `smgrep` and rebuild the index:

```bash
cargo install smgrep --force
smgrep index --reset
```
