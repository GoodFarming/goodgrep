# ggrep eval suites

This folder contains **curated evaluation suites** for `ggrep` search quality (recall / ranking).

## Suite format

Suites are TOML files with:

- `version` (currently `1`)
- `[defaults]` for `k`, `per_file`, `rerank`, and `mode`
- `[[cases]]` entries with:
  - `id` (stable identifier)
  - `query` (natural language)
  - optional: `mode`, `k`, `per_file`, `rerank`
  - expectations:
    - `expect_any_path_contains = ["..."]` (case-insensitive substring match)
    - `expect_all_path_contains = ["..."]`
    - `expect_any_path_regex = ["..."]` (Rust regex)
    - `expect_all_path_regex = ["..."]`

## Run

From repo root:

```bash
cd /path/to/goodgrep

# writes a JSON report to /tmp by default
cargo +nightly run --manifest-path Tools/ggrep/Cargo.toml --no-default-features --bin ggrep -- \
  eval --path . --cases Datasets/ggrep/eval_cases.toml
```

To choose an output file:

```bash
ggrep eval --path . --cases Datasets/ggrep/eval_cases.toml --out /tmp/ggrep-eval.json
```

If you already indexed and want to skip re-syncing:

```bash
ggrep eval --no-sync --path . --cases Datasets/ggrep/eval_cases.toml --out /tmp/ggrep-eval.json
```

To iterate on one case quickly:

```bash
ggrep eval --path . --cases Datasets/ggrep/eval_cases.toml --only sync_audit_log_ddl
```

Quick smoke suite (indexes `Tools/ggrep` only):

```bash
ggrep eval --path Tools/ggrep --cases Datasets/ggrep/eval_smoke.toml --out /tmp/ggrep-eval-smoke.json
```
