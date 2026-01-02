# GGREP Migration Plan (GoodFarmingAI -> goodgrep)

## Objectives
- Move the full GGREP toolchain out of `GoodFarmingAI/Tools/ggrep` into a standalone repo at `/home/adam/goodgrep`.
- Preserve all GGREP code, docs, plans, specs, and scripts (no functional loss).
- Keep GoodFarmingAI usable as an indexed workspace without hosting GGREP internals (no vendored `Tools/ggrep`, `Scripts/ggrep`, or `Datasets/ggrep`).
- Avoid large build artifacts in repo (no `target/`), keep storage stable.

## Success Criteria
- All GGREP-related files live in `/home/adam/goodgrep` with no missing assets.
- GoodFarmingAI no longer contains `Tools/ggrep/`, `Scripts/ggrep/`, or `Datasets/ggrep/` (only a pointer doc if needed).
- New repo builds and runs with the same or better behavior.
- Documentation remains consistent (paths, instructions, and references updated).
- No new large artifacts are stored in-repo (no `target/` or caches).

## Scope (What Moves)
- Primary: `GoodFarmingAI/Tools/ggrep/**` (all code, docs, plans, specs, scripts).
- Supporting:
  - `GoodFarmingAI/Scripts/ggrep/**`
  - Any GGREP datasets under `**/Datasets/**` (ensure they live in `Datasets/` in new repo).
  - Repo-level helper docs that are GGREP-only (update or move as appropriate).
- Out of scope (stays in GoodFarmingAI): general orchestration, unrelated apps, or other tools.

## Non-Goals
- No refactor or feature changes during migration.
- No change to GGREP contracts or Phase II plan content beyond path updates.

## Milestones and Checklists

### M0: Decisions and Preconditions
- [x] Decide migration method (copy-only selected; preserve history later if needed).
- [x] Decide new repo root layout (keep `Tools/ggrep` subdir to minimize path edits).
- [ ] Decide GoodFarmingAI integration approach: pointer doc vs. submodule vs. external install.
- [ ] Confirm shared-store permissions policy for multi-user setups (e.g., group/ACL for shared store).
- [x] Confirm target repo license and ownership metadata.
- [ ] Decide versioning policy and initial tag target (e.g., `v0.1.0`).

### M1: Inventory and Mapping
- [x] Run a repo-wide GGREP inventory (paths + references):
  - `rg --files -g '*ggrep*'` and `rg -n 'ggrep|GGREP'` for references.
- [x] Identify any GGREP files outside `Tools/ggrep/` and `Scripts/ggrep/`.
- [x] Audit programmatic consumers in GoodFarmingAI (scripts/tools calling `Tools/ggrep/...`).
- [x] Create a path mapping table for every GGREP file moved.
- [x] Identify dependencies on GoodFarmingAI pathing (hardcoded paths in scripts/docs).
- [x] Identify dataset-like files to rehome under `Datasets/` in new repo.
- [x] Record baseline commit hashes for traceability before move.
- [ ] Decide if a temporary shim is needed for legacy `Tools/ggrep` path consumers.

#### Inventory Findings (2025-01-02)
#### Inventory Findings (2026-01-02)
- GoodFarmingAI baseline commit: `4dd6c69bed28e9d4ab244647042d8f4ca46e8d16` (working tree dirty)
- GGREP tree: `GoodFarmingAI/Tools/ggrep/**`
- Scripts: `GoodFarmingAI/Scripts/ggrep/cargo.sh`, `GoodFarmingAI/Scripts/ggrep/configure-fast.sh`
- Datasets: `GoodFarmingAI/Datasets/ggrep/README.md`, `GoodFarmingAI/Datasets/ggrep/eval_cases.toml`, `GoodFarmingAI/Datasets/ggrep/eval_smoke.toml`
- Hardcoded GoodFarmingAI paths (update on migration):
  - `Tools/ggrep/GOODFARMINGAI.md`
  - `Tools/ggrep/Docs/Spec/GGREP-Snapshot-Index-Contracts-v0.1.md` (canonical_root example)
  - `Datasets/ggrep/README.md`

### M2: New Repo Bootstrap (/home/adam/goodgrep)
- [x] Initialize git repo and write a minimal README.
- [x] Create root structure: `Scripts/`, `Datasets/`, `Tools/` (Docs remain under `Tools/ggrep`).
- [x] Add `.gitignore` to exclude `target/`, `.venv/`, `.env`, caches, logs.
- [x] Add build hygiene: `CARGO_TARGET_DIR` default (avoid `target/` in repo).
- [x] Copy/port `Scripts/ggrep/cargo.sh` or equivalent build wrapper.
- [x] Add/verify dependency locks (`rust-toolchain.toml`, `pyproject.toml`/`requirements.txt` as needed).
- [x] Ensure a `LICENSE` exists at the new repo root.

### M2.5: Dependency Isolation (Preflight)
- [x] Confirm the new repo builds in isolation (no implicit parent repo dependencies).
- [x] Verify all path-based dependencies in `Cargo.toml` resolve after relocation.

### M3: Migration Execution
- [x] Copy GGREP core tree (source remains until M4 cleanup):
  - `GoodFarmingAI/Tools/ggrep/** -> /home/adam/goodgrep/Tools/ggrep/**`
- [x] Copy GGREP scripts:
  - `GoodFarmingAI/Scripts/ggrep/** -> /home/adam/goodgrep/Scripts/ggrep/**`
- [x] Copy GGREP datasets:
  - `GoodFarmingAI/Datasets/ggrep/** -> /home/adam/goodgrep/Datasets/ggrep/**`
- [x] Update documentation paths that still reference `GoodFarmingAI/Tools/ggrep`.
- [x] Update scripts to resolve repo root dynamically (no hardcoded `GoodFarmingAI`).
- [x] Update README to explain how GGREP indexes external repos (e.g., GoodFarmingAI).
- [x] Re-verify `Cargo.toml` path dependencies after any layout changes.

### M4: GoodFarmingAI Cleanup and Integration Stub
- [x] Remove `GoodFarmingAI/Tools/ggrep/`, `GoodFarmingAI/Scripts/ggrep/`, and `GoodFarmingAI/Datasets/ggrep/` completely.
- [ ] Add pointer doc in GoodFarmingAI (example: `Docs/ggrep-integration.md`). (defer until Phase II hardening operational)
- [x] Update any GoodFarmingAI docs or scripts referencing the old path.
- [ ] Add environment variable or config note (e.g., `GGREP_HOME=/home/adam/goodgrep`). (defer until Phase II hardening operational)
- [ ] If needed, add a temporary shim wrapper for legacy `Tools/ggrep` path consumers. (defer until Phase II hardening operational)

### M5: Validation and QA
- [x] File parity check between source and target:
  - `rsync -na` or checksums to confirm no missing files.
- [x] Build validation in new repo:
  - [x] `cargo check` (no-default-features)
  - [x] `cargo test` (no-default-features)
  - [x] `cargo clippy` (pathing and lint sanity; warnings present)
  - [x] `ggrep --help` runs
  - [x] `python3 -m py_compile` for any Python scripts (N/A: no Python files)
- [x] Doc sanity: verify all links/paths in specs and plans.
- [x] Storage hygiene: verify no large artifacts in repo (`du -sh`, `rg --files -g 'target'`).

### M5.5: CI/CD Bootstrap
- [x] Add minimal CI (local or hosted) running `cargo fmt`, `cargo test`, `cargo clippy`.
- [x] Add pre-commit or make targets to standardize local checks.

### M6: Cutover and Rollback Plan
- [ ] Tag last in-repo GGREP state in GoodFarmingAI (before removal).
- [ ] Tag initial import in goodgrep repo.
- [ ] Verify GGREP can index GoodFarmingAI from new location.
- [ ] Rollback plan documented (restore from tag or copy back if needed).

### M6.5: User Environment Configuration
- [ ] Provide an install step (link binary into `~/.local/bin` or update PATH).
- [ ] Document developer vs. production invocation (`cargo run` vs. built binary).

## Path Mapping (Initial Draft)
- `GoodFarmingAI/Tools/ggrep/** -> /home/adam/goodgrep/Tools/ggrep/**`
- `GoodFarmingAI/Scripts/ggrep/** -> /home/adam/goodgrep/Scripts/ggrep/**`
- `GoodFarmingAI/Tools/ggrep/Docs/** -> /home/adam/goodgrep/Tools/ggrep/Docs/**`
- `GoodFarmingAI/Tools/ggrep/GOODFARMINGAI.md -> /home/adam/goodgrep/Tools/ggrep/GOODFARMINGAI.md` (update for new repo)
- `GoodFarmingAI/Datasets/ggrep/** -> /home/adam/goodgrep/Datasets/ggrep/**`

## Risks and Mitigations
- Shared-store permissions mismatch: define group/ACL model early; document mode.
- Hardcoded GoodFarmingAI paths: require repo-root discovery in scripts.
- Large build artifacts: enforce `CARGO_TARGET_DIR` and `.gitignore` for `target/`.
- Missing files: enforce parity checks and checksum validation.
- Path-based dependency breakage: verify `Cargo.toml` path dependencies after move.

## Open Questions
- Do we preserve history via git subtree/filter-repo later (post-copy)?
- Should GoodFarmingAI link via submodule, stub doc, or external install?
- What is the shared-store permissions baseline (single-user vs shared)?

## Exit Criteria
- GoodFarmingAI does not contain GGREP code.
- All GGREP docs/plans/specs are in goodgrep repo.
- Build/test passes in goodgrep repo.
- Pointer doc exists for GoodFarmingAI integration.
