# GGREP Spec Index (v0.2)

Status: Draft v0.2 (Phase III SSOT)  
Scope: `Tools/ggrep` (goodgrep repo)  
Purpose: Declare normative ownership and schema/version governance to prevent drift during Phase III (usefulness).

Phase III builds on the Phase II correctness and daemon contracts. Those contracts remain SSOT.

---

## Phase III Scope (Execution)

- Primary goal: agent usefulness (coherent packs, progressive disclosure, MCP parity).
- Supported OS: Linux-only (Windows/macOS deferred).
- Embedding runtime: CPU-only baseline (optional CUDA remains non-goal unless explicitly reopened).
- Filesystems: local only baseline (no multi-host shared-store semantics in Phase III unless explicitly reopened).

---

## Normative Ownership (precedence)

If requirements conflict, the following precedence applies:

1) Snapshot/Index Contracts (correctness + isolation)  
2) Query/Daemon Contracts (protocol + QoS)  
3) Phase III Usefulness Spec  
4) Phase III Usefulness Plan  
5) Phase II governance docs (archived; historical context only)

---

## SSOT Documents

### Normative (binding MUST/SHOULD/MAY)

Core correctness and daemon behavior:

- `Tools/ggrep/Docs/Spec/GGREP-Snapshot-Index-Contracts-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Query-Daemon-Contracts-v0.1.md`

Phase III usefulness:

- `Tools/ggrep/Docs/Spec/GGREP-Phase-III-Usefulness-Spec-v0.1.md`

### Governance / execution (binding decisions + delivery gates)

- `Tools/ggrep/Docs/Plan/GGREP-Phase-III-Usefulness-Plan-v0.1.md`

### Descriptive foundation (non-normative but treated as “as-is truth”)

- `Tools/ggrep/Docs/Spec/GGREP-System-Preamble-v0.1.md`

### Research (informative)

- `Tools/ggrep/Docs/Research/External-Docs-Notes.md`
- `Tools/ggrep/Docs/Research/Assumption-Tests.md`
- `Tools/ggrep/Docs/Research/Target-Environment-Matrix.md`
- `Tools/ggrep/Docs/Research/Embedding-Model-Policy.md`
- `Tools/ggrep/Docs/Research/Gemini-Phase-III-Review-2026-01-04.md`

### Archived (Phase II governance)

- `Tools/ggrep/Docs/Archive/Phase-II/Plan/GGREP-Phase-II-Hardening-Plan-v0.1.md`
- `Tools/ggrep/Docs/Archive/Phase-II/Plan/GGREP-Phase-II-Implementation-Checklist-v0.1.md`
- `Tools/ggrep/Docs/Archive/Phase-II/Plan/GGREP-Phase-II-Test-Policy-v0.1.md`
- `Tools/ggrep/Docs/Archive/Phase-II/Spec/GGREP-Spec-Index-v0.1.md`
- `Tools/ggrep/Docs/Archive/Phase-II/Spec/GGREP-Conformance-Map-v0.1.md` (Phase II conformance mapping; Phase III mapping TBD)

---

## Artifact Schema Versions (current)

| Artifact | Schema Version |
| --- | --- |
| snapshot manifest | 1 |
| index_state.json | 1 |
| writer_lease.json | 1 |
| chunk row schema | 1 |
| query_success JSON | 1 |
| query_error JSON | 1 |
| status JSON | 1 |
| health JSON | 1 |

Phase III may add a new `slate_success` schema. If so, it MUST be versioned and listed here.

---

## Requirement ID Scheme

- Snapshot/index contracts: `IDX.MUST.###`
- Query/daemon contracts: `IPC.MUST.###`
- Governance requirements: `GOV.MUST.###`
- Phase III usefulness requirements: `USE.MUST.###`

IDs MUST be stable once shipped and referenced by tests/eval/conformance.

---

## Ownership (code modules)

Ownership is tracked here to prevent silent divergence between docs and code.

| Contract Area | Intended Owner |
| --- | --- |
| Repo identity + fingerprints | `Tools/ggrep/src/identity.rs` |
| Config load/validation | `Tools/ggrep/src/config.rs` |
| Ignore semantics | `Tools/ggrep/src/file/ignore.rs` |
| Sync / indexing | `Tools/ggrep/src/sync.rs` |
| Snapshot publish + manifests | `Tools/ggrep/src/snapshot/*` |
| Lease + fencing | `Tools/ggrep/src/lease.rs` |
| Offline reader locks | `Tools/ggrep/src/reader_lock.rs` |
| Vector store + retrieval | `Tools/ggrep/src/store/*` |
| Ranking + modes + profiles | `Tools/ggrep/src/search/*` |
| Daemon + IPC | `Tools/ggrep/src/cmd/serve.rs`, `Tools/ggrep/src/ipc.rs`, `Tools/ggrep/src/usock/*` |
| Status/health/audit/repair/gc | `Tools/ggrep/src/cmd/status.rs`, `Tools/ggrep/src/cmd/health.rs`, `Tools/ggrep/src/cmd/audit.rs`, `Tools/ggrep/src/cmd/repair.rs`, `Tools/ggrep/src/cmd/gc.rs` |
| MCP server | `Tools/ggrep/src/cmd/mcp.rs` |
