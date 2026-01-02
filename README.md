# goodgrep (ggrep)

This repo hosts the `ggrep` tool and its Phase II hardening plans/specs.

## Repo Layout
- `Tools/ggrep/` - ggrep source, docs, plans, specs
- `Scripts/ggrep/` - helper scripts
- `Datasets/ggrep/` - evaluation suites

## Quick Start

From the repo root:

```bash
cd /home/adam/goodgrep/Tools/ggrep
cargo +nightly build
```

To avoid large build artifacts in-repo, prefer the wrapper:

```bash
/home/adam/goodgrep/Scripts/ggrep/cargo.sh +nightly build
```

See `Tools/ggrep/README.md` for full usage.

To index another repo (for example, GoodFarmingAI), run `ggrep` from that repo
or pass `--path /path/to/repo`.
