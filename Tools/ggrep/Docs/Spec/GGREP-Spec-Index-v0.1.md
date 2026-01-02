# GGREP Spec Index (v0.1)

Status: Draft v0.1  
Scope: `Tools/ggrep` (goodgrep repo)  
Purpose: Declare normative ownership and schema versions to prevent spec drift.

## Phase II Scope (Execution)

- Supported OS: Linux-only (Windows/macOS deferred).
- Embedding runtime: CPU-only (no CUDA/GPU requirements).
- Filesystems: local only (no NFS/SMB shared-store in Phase II).
- Shared-store scope: same-host, same-group.

Platform-specific notes may appear in specs as informative future work, but are not Phase II gating items.

## Normative Ownership

If requirements conflict, the following precedence applies:

1) Snapshot/Index Contracts  
2) Query/Daemon Contracts  
3) Phase II Hardening Plan  
4) Implementation Checklist

## SSOT Documents

Normative (MUST/SHOULD/MAY language is binding):

- `Tools/ggrep/Docs/Spec/GGREP-Snapshot-Index-Contracts-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Query-Daemon-Contracts-v0.1.md`

Governance/Execution (normative decisions + gates):

- `Tools/ggrep/Docs/Plan/GGREP-Phase-II-Hardening-Plan-v0.1.md`
- `Tools/ggrep/Docs/Plan/GGREP-Phase-II-Implementation-Checklist-v0.1.md`

Conformance (MUST coverage map):

- `Tools/ggrep/Docs/Spec/GGREP-Conformance-Map-v0.1.md`

Informative (external/official docs notes):

- `Tools/ggrep/Docs/Research/External-Docs-Notes.md`

## Artifact Schema Versions (Current)

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

## Requirement ID Scheme

- Snapshot/Index contracts use `IDX.MUST.###`.
- Query/Daemon contracts use `IPC.MUST.###`.
- IDs MUST be stable once shipped and referenced by the Conformance Map.

## Normative vs Informative

- Sections that use MUST/SHOULD/MAY are **Normative**.
- Examples, rationale, and explanatory text are **Informative**.

## Ownership (Code Modules)

Ownership is tracked here to prevent silent divergence between docs and code.

| Contract Area | Intended Owner Module |
| --- | --- |
| Snapshot publish + manifests | `Tools/ggrep/src/snapshot` |
| Lease + locking | `Tools/ggrep/src/lease` |
| Change detection | `Tools/ggrep/src/changes` |
| Query engine + ranking | `Tools/ggrep/src/query` |
| Daemon + IPC | `Tools/ggrep/src/daemon` |
| Status/health/audit | `Tools/ggrep/src/status` |

Module boundaries MUST be enforced with `pub(crate)` scoping and a lightweight dependency lint to prevent
cross-layer imports (daemon depends on query engine, not snapshot internals).
