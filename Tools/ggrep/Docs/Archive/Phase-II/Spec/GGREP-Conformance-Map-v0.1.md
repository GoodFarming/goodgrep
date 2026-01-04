# GGREP Conformance Map (v0.1)

Status: Archived (Phase II; moved under `Docs/Archive/Phase-II/Spec`; Phase III conformance mapping TBD)  
Scope: `Tools/ggrep` (goodgrep repo)  
Purpose: Map critical MUST requirements to tests or runtime assertions.

Phase III SSOT:

- `Tools/ggrep/Docs/Spec/GGREP-Spec-Index-v0.2.md`

## Conformance Table

Each MUST requirement must be enforced by a test or a runtime assertion with a stable identifier.

| Requirement ID | Requirement (MUST) | Source | Enforcement | Test/Assertion |
| --- | --- | --- | --- | --- |
| IDX.MUST.001 | Tombstone enforcement is structural (no bypass) | Snapshot/Index Contracts | Test + lint | `tests::conformance::tombstone_structural_no_bypass` |
| IDX.MUST.002 | Publish preflight checks lease_epoch + owner | Snapshot/Index Contracts | Runtime assert | `assert::lease_epoch_preflight` |
| IDX.MUST.003 | Durability barrier before manifest write | Snapshot/Index Contracts | Integration test | `tests::durability_barrier_test::durability_barrier_smoke` |
| IDX.MUST.004 | GC requires writer lease | Snapshot/Index Contracts | Runtime assert | `assert::gc_requires_writer_lease` |
| IDX.MUST.005 | Offline readers take shared lock; GC takes exclusive | Snapshot/Index Contracts | Integration test | `tests::conformance::offline_reader_lock_gc_exclusive` |
| IPC.MUST.001 | Protocol handshake + highest common version | Query/Daemon Contracts | Unit test | `tests::ipc::handshake_highest_common_version` |
| IPC.MUST.002 | IPC framing + payload caps enforced | Query/Daemon Contracts | Robustness test | `tests::ipc_robustness_test::test_ipc_rejects_oversized_payload` |
| IPC.MUST.003 | Output sanitization (no ANSI/control chars) | Query/Daemon Contracts | Unit test | `tests::output::sanitize_control_chars` |
| IPC.MUST.004 | Deterministic ordering + tie-break rules | Query/Daemon Contracts | Regression test | `tests::query::deterministic_ordering_tiebreak` |
| IDX.MUST.006 | Checksums verified before query | Snapshot/Index Contracts | Integration test | `tests::snapshot::checksum_verification` |
| IDX.MUST.007 | Compaction prunes tombstones | Snapshot/Index Contracts | Integration test | `tests::compaction::tombstone_prune` |
| IPC.MUST.005 | Open handle budgets enforced | Query/Daemon Contracts | Load test | `tests::limits::open_handle_budget` |
| GOV.MUST.001 | Module boundaries enforced via pub(crate) + lint | Spec Index | Lint test | `tests::module_boundary_test::module_boundary_lint` |

## Required Gates

- Conformance map MUST be updated when new MUST requirements are added.
- CI MUST fail if any critical MUST lacks enforcement or has an unresolved Test/Assertion identifier.
