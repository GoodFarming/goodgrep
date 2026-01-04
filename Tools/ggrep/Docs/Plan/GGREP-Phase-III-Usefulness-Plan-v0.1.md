# GGREP Phase III Usefulness Plan (v0.1)

Status: Draft v0.1  
Owner: ggrep / Engine Dev  
Scope: `Tools/ggrep`  
Target repo for tuning: GoodFarmingAI

Phase III is about turning GGREP from a “semantic entrypoint finder” into an agent-default workflow: structured,
budgeted, confidence-aware packs that let an agent build a correct mental model quickly and act safely.

Normative spec: `Tools/ggrep/Docs/Spec/GGREP-Phase-III-Usefulness-Spec-v0.1.md`

---

## 0) Preamble: Why this wave exists

### 0.1 What Phase II established (the foundation we are building on)

Phase II delivered the reliability substrate that makes “agent-first UX” worth investing in:

- Snapshot isolation (queries see old/new snapshot, never partial state).
- Crash-safe publish (atomic pointer swap to `ACTIVE_SNAPSHOT`).
- Multi-daemon operation (per-repo store isolation) and compatibility handshakes.
- Query QoS controls (admission, timeouts, resource budgets) and operator visibility (`status`/`health`).

With those invariants in place, Phase III can focus on usefulness without constantly re-litigating correctness.

### 0.2 Why target usefulness (the core motivation)

Agents are constrained by *context bandwidth*: reading 20–50 files to understand a subsystem is slow, expensive,
and often unnecessary if we can surface the right conceptual “map” with evidence.

GGREP’s unique advantage over plain text search is that it can:

- connect concepts across paradigms (code ↔ plans/specs ↔ diagrams),
- retrieve meaningful blocks even when identifiers are unknown, and
- give line-numbered evidence so an agent can jump directly to edits.

But to become the default tool ~80% of the time, GGREP must output *coherent working slates* (not just “top-N
chunks”) and must do so safely (budgeted, progressive disclosure) and honestly (confidence-aware).

### 0.3 What real feedback showed (pain → plan items)

Operator/agent feedback to date highlights specific usability gaps:

- Mode mismatch confusion (CLI defaults vs MCP defaults) changes results materially, especially for docs/diagrams.
- MCP lacks the CLI’s “safe knobs” (`--compact`, `--no-snippet`, snippet depth), causing unnecessary text exposure.
- Noise dilution from generated/WIP trees (e.g. archives/agent_outputs) reduces SSOT hunting quality.
- `status` output can be ambiguous (store ids without clear repo root / stale ambiguity).
- `index --dry-run` reports “scanned” rather than “indexable”, making it hard to reason about scope and ignore rules.
- `fast_mode` is overloaded (affects indexing, query embeddings, rerank, and anchor inclusion), making “rerank on/off”
  operationally confusing.

Phase III explicitly targets these with MCP parity, profiles/ignores, confidence gating, slate packs, and operator QOL.

### 0.4 Strategy (how we win without turning GGREP into an LLM)

Phase III does **not** try to “summarize the repo”. Instead, it makes retrieval evidence *structured* and *actionable*:

- **Slate packs**: file-centric grouping (anchor + best evidence) + coverage across code/docs/diagrams/config/tests.
- **Progressive disclosure**: paths-first → small evidence → expanded context → full content only on explicit opt-in.
- **Parity**: MCP and CLI expose the same shaping controls so agent clients can be safe by default.
- **Confidence**: when there is no strong match, say so and return less (or none), rather than returning noise.
- **Profiles**: opt-in dataset inclusion and noise suppression are explicit, fingerprinted, and budgeted.

---

## 1) Phase III success criteria (what “done” looks like)

For a representative set of GoodFarmingAI questions (auth, phenology, marker ledger, major architectural features):

- Agents can get a coherent slate in **1–2 calls** (CLI or MCP) without needing to open dozens of files.
- Slate output is **budgeted**, **deterministic**, and **confidence-aware** (low-confidence is explicit).
- MCP and CLI have **parity** on output shaping (paths/no-snippet/snippet depth).
- Noise trees are suppressed by default via repo SSOT (ignore + profiles), without preventing intentional inclusion
  of datasets when budgets/governance exist.
- Regression is measurable: eval suites cover both retrieval and slate usefulness (coverage + precision).

### 1.1 Metrics we track (and why)

GGREP usefulness is measured by “how quickly an agent can act correctly”, not by “how many related files we can list”.
Track these across representative GoodFarmingAI tasks:

- **Calls-to-slate:** median number of GGREP calls before the agent has enough evidence to start editing (target: ≤2).
- **Slate size:** bytes and approximate tokens per slate level (targets depend on level; defaults should be small).
- **Slate coverage score:** presence of at least one strong item in each expected bucket (Code/Docs/Graph/Config/Tests).
- **Precision under low confidence:** out-of-domain queries should yield “no strong matches” rather than unrelated files.
- **Operator trust:** `status`/`health` output makes repo/daemon state unambiguous (multi-daemon environments).

### 1.2 Default budgets (initial targets; tune with eval)

These are *starting targets* for “agent-default” outputs. Exact limits are enforced by implementation and may vary by
repo config/profile.

- Slate Level 0 (paths-only): ≤ 5 KB total output.
- Slate Level 1 (anchor + short evidence): ≤ 64 KB total output.
- Slate Level 2 (anchor + long evidence + neighbor context): ≤ 200 KB total output.
- Slate Level 3 (full chunk content): explicit opt-in; subject to existing hard caps (`max_response_bytes`, snippet caps).

---

## 2) Workstreams

### WS-A: Slate output (structured packs)

Goal: implement a new pack output that groups results by file, includes anchors + evidence, and enforces budgets.

Deliverables:

- A1: Slate is a **projection of the standard search pipeline**: `ggrep search --format=slate ...` (optional thin alias:
  `ggrep slate ...` that forwards to search).
- A2: JSON schema/versioning: define `slate_success` (versioned) and advertise via handshake (or bump `query_success`
  schema if embedding slate into it). Preserve standard metadata (`request_id`, fingerprints, timings, limits/warnings).
- A2.1: Update `Tools/ggrep/Docs/Spec/GGREP-Query-Daemon-Contracts-*` and JSON schemas to make slate/context requests
  contract-valid before implementation.
- A3: Slate selection policy (coverage-aware quotas across code/docs/graph + within-file anchor+evidence)
- A4: Progressive disclosure levels (paths-only → short → long → full)
- A5: Slate “navigation affordances” (consistent file+line anchors; explicit per-section ordering; minimal redundancy;
  `reason`/why-this-file metadata; optional breadcrumb compression for deep paths)

Acceptance tests:

- deterministic ordering test (same snapshot+query+profile yields same JSON)
- budget enforcement test (`limits_hit` correctly emitted)
- neighbor context expansion test (semantic boundary expansion is deterministic and budgeted)
- “coverage-aware” selection test on a curated fixture repo

### WS-H: Exploration sessions (context graph + delta slates)

Goal: upgrade GGREP from “retrieve” to “navigate” for agent workflows by supporting multi-hop exploration without
thrash: pinned snapshot, a lightweight orientation graph, and delta-only followups.

Deliverables:

- H1: Session protocol for slate/context requests:
  - `session=new|<token>`, `op=start|refine|expand`, `expand_targets=[...]`, `delta_only=true|false`
  - sessions MUST pin `snapshot_id` (session-scoped) so follow-ups are stable even if a new snapshot is published
- H2: Delta-only behavior:
  - maintain a seen-set (paths and optionally chunk ids) and suppress repeats deterministically
  - include `seen_set_digest` and updated `session_token` in responses
- H3: Context graph (budgeted, deterministic, metadata-first):
  - nodes: selected slate files (and optionally “anchor vs evidence” subnodes)
  - edges (cheap signals, computed only over selected files): import adjacency, markdown links, mermaid edges/messages,
    config-key mentions, test mentions
  - graph MUST NOT include raw content unless disclosure level explicitly requests snippets
- H4: CLI + MCP surfacing:
  - CLI: `ggrep search --format=slate --session ...` (optional `ggrep slate ...`)
  - MCP: add session fields to `context` tool requests/responses

Acceptance tests:

- deterministic token/graph ordering test (same snapshot+query+session token yields identical output)
- delta correctness test (`delta_only` never repeats previously seen files/evidence)
- budget enforcement test (node/edge caps, token size cap, total bytes cap)
- safety test (graph contains only metadata/structure; no raw content leakage)

### WS-B: MCP parity and agent safety

Goal: bring “safe knobs” to MCP and make defaults conservative.

Deliverables:

- B1: MCP output shaping parity with CLI (paths-only, no-snippet, snippet depth, include-anchors)
- B2: MCP `context` tool (the slate behavior: opinionated, budgeted, safe) + keep `search` for raw/flexible access
- B3: MCP schema documentation + examples
- B4: Default-mode alignment decision (make mode defaults predictable across CLI, shorthand CLI, and MCP)
- B5: Deprecation cleanup: aggressively deprecate MCP `good_search` alias and converge on a stable tool set

Acceptance tests:

- MCP tool output matches CLI JSON schema for equivalent settings
- MCP defaults never return full content unless requested

### WS-C: Confidence gating and “no strong matches”

Goal: stop returning noise without signaling; reduce false positives on out-of-domain queries.

Deliverables:

- C1: Deterministic confidence heuristic based on **relative signals** (score drop-off / z-score / ratio) and optional
  dense-vs-lexical agreement (avoid absolute thresholds that drift across model versions)
- C2: Standard warnings/flags: `low_confidence`, `no_strong_matches`
- C3: Selection behavior under low confidence (fewer results; prioritize anchors; optional prompt to rescope)
- C4: Precision guardrails for out-of-domain queries (prefer “no strong matches” over unrelated domains)

Acceptance tests:

- Low-confidence queries produce explicit flags and reduced output
- On known-good queries, confidence does not suppress correct results (regression gated)

### WS-D: Index profiles + selective dataset inclusion

Goal: allow explicit, fingerprinted index policies for what gets indexed.

Deliverables:

- D1: `.ggrep.toml` profile schema (index profiles + query profiles + optional view-only path filters)
- D2: Profile selection plumbing (`--profile <name>`, default profile)
- D3: Default GoodFarmingAI profile recommendations (suppress archives/agent outputs/external reviews; keep SSOT trees)
- D4: Optional dataset profile gated by explicit budgets (disk + bytes/sync)
- D5: Repo-root `.ggignore` guidance and recommended baseline templates per profile (tracked SSOT)
- D6: Profile reindex rules: subset-only “view profiles” SHOULD be query-time filters (no reindex). Profiles that
  *expand* eligibility beyond the current store MUST require a new store id (or layered stores).

Acceptance tests:

- Profile changes that expand eligibility require a new index identity (reindex/new store; no silent mixing).
- Query/profile filters that only narrow scope do not require reindex and are included in the query fingerprint.
- Dataset profile respects budgets and fails/degrades deterministically

### WS-E: Evaluation + regression

Goal: evolve eval from “find file” to “slate is useful”.

Deliverables:

- E1: Extend `Datasets/ggrep/eval_cases.toml` with domain cases and slate expectations
- E2: Add “slate coverage” checks (sections present when ground truth exists)
- E3: Add “precision” checks (low-confidence queries should not return unrelated domains)
- E4: Add baseline regression gating (pass-rate/MRR + slate coverage)

Acceptance tests:

- CI-friendly smoke eval on a small fixture repo (no model download)
- nightly eval on GoodFarmingAI with pinned snapshot id

### WS-E.1: Usefulness evaluation (what we add beyond “find a file”)

In addition to existing “expect path contains …” assertions, Phase III eval should add:

- slate section coverage assertions (expected buckets present when ground truth exists)
- confidence assertions (low-confidence queries flagged and outputs reduced)
- stability assertions (selection stable; only snippet depth changes across disclosure levels)

### WS-F: Operator QOL (daemon/status/index ergonomics)

Goal: make day-to-day operation predictable and debuggable (so agents + humans trust the tool under load).

Deliverables:

- F1: `ggrep status` shows canonical root (or best-effort repo label) per running daemon; stale entries are unambiguous.
- F2: `ggrep status` and `ggrep list/stores` explicitly report store id ↔ canonical root mapping.
- F3: `ggrep index --dry-run` reports *eligible/indexable* counts (and excluded reasons), not just scanned totals.
- F4: Clear lifecycle output for `serve/stop/stop-all` (what store/path was affected; avoid “stale” ambiguity).
- F5: Clarify indexing progress semantics in CLI output and JSON warnings (what “indexing 0%” means; when results are reliable).

Acceptance tests:

- Golden output test: status lists multiple daemons with distinct roots deterministically.
- Dry-run fixture: known ignored trees report correct “eligible vs excluded” counts.

### WS-G: Search quality and ranking hygiene (high-leverage fixes)

Goal: reduce “we returned something because you asked” behavior and improve cross-paradigm coherence.

Deliverables:

- G1: Untangle `fast_mode` into explicit, orthogonal knobs with correct fingerprinting boundaries:
  `index.skip_definitions` (config fingerprint), `query.skip_colbert_encode` (query fingerprint),
  `search.skip_rerank` (query fingerprint), and `output.include_anchors` (query fingerprint / output shaping).
- G2: Ensure “rerank on/off” is safe operationally (no output disappearance; consistent JSON/human behavior).
- G3: Improve docs/diagram readability in outputs (Mermaid summary previews; Markdown heading breadcrumbs as first-class).
- G4: Optional lightweight “link signals” for coherence (imports/exports adjacency, diagram links) used to enrich slates.

Acceptance tests:

- Regression: same query with rerank toggled still produces non-empty, coherent outputs (selection stable; ordering differs only where expected).
- Diagram/doc preview test: Mermaid/Markdown preview format is deterministic and budgeted.

---

## 3) Milestones (recommended order)

### M0 — Documentation and SSOT reset (this change set)

- Publish system preamble (as-is behavior): `Tools/ggrep/Docs/Spec/GGREP-System-Preamble-v0.1.md`
- Publish Phase III spec and plan (v0.1)
- Move Phase II governance docs under `Tools/ggrep/Docs/Archive/Phase-II/` (keep core contracts as SSOT)

### M1 — Operator QOL + MCP parity (fast, high leverage)

- Implement WS-F (status/root mapping, dry-run indexable counts, clearer lifecycle output)
- Implement WS-B (MCP shaping parity; conservative defaults)
- Implement WS-G early (decouple fast_mode and fix rerank/anchor coupling) as a blocker for Slate MVP

### M2 — Slate MVP (level 1: anchor + short evidence)

- Finalize contract/schema approach (new `slate_success` vs `query_success` v2) and handshake advertisement
- Implement file-grouped slate with strict budgets
- Include Code/Docs/Graph quotas aligned to intent mode
- Add MCP `context` tool (slate behavior)

### M3 — Sessioned exploration MVP (delta-only + light context graph)

- Add session tokens and snapshot pinning for slate/context
- Implement `delta_only` followups with deterministic seen-set suppression
- Emit a minimal, budgeted context graph (imports + links + diagram edges; expand later)

### M4 — Diagram + doc readability upgrades

- Prefer Mermaid edge/message summary in previews
- Emit Markdown heading breadcrumbs prominently in doc results

### M5 — Confidence gating + coverage heuristics

- Implement low-confidence detection and selection behavior
- Add coverage-aware selection rules (entrypoints/config/tests)

### M6 — Index profiles + dataset opt-in

- Add profile selection and fingerprinting
- Provide GoodFarmingAI recommended profiles and ignore sets

### M7 — Regression gating hardening

- Expand eval suites and make “slate usefulness” a first-class metric
- Add baseline comparisons and guardrails

---

## 4) Open design questions (to resolve early)

1) Slate schema: should it be a brand-new schema or an extension of the existing `query_success` JSON?
2) How do we represent “coverage” deterministically without embedding a bespoke ontology?
3) How much graph-building do we do (imports/exports edges) vs keep it heuristic and cheap?
4) Profile UX: `--profile` vs `--include-datasets` style toggles (prefer profiles for fingerprinting).
5) Default mode alignment: should shorthand CLI default to `discovery` to match agent expectations, or stay `balanced`?
6) Confidence thresholds: how conservative should “no strong matches” be initially to avoid suppressing recall?
7) Timeout semantics: does slate have a separate timeout budget, or do we support partial returns when nearing
   `query_timeout_ms`?
8) Session tokens: stateless vs daemon-stateful; token encoding + size caps; snapshot pinning semantics on GC.
9) Context graph edges: which edge types are high-signal vs misleading, and how do we map imports/links to file nodes
   deterministically without expensive repo-wide graph building?

---

## 5) Risks and mitigations

- **Risk: bloaty output** → enforce budgets + progressive disclosure; default to safe level.
- **Risk: reduced recall due to confidence gating** → gate via eval; keep confidence heuristic conservative at first.
- **Risk: profile explosion** → keep 2–3 profiles max per repo (default, datasets, debug).
- **Risk: fast_mode coupling** → split into explicit flags before building slate, so slate is not tied to
  “fast index” behavior.

---

## 6) Rough implementation difficulty (order-of-magnitude)

This is intentionally coarse; actual effort depends on how much contract/schema migration we choose.

- **Low:** status/root mapping, dry-run indexable counts, clearer lifecycle output (WS-F).
- **Low–Medium:** MCP parity flags + conservative defaults; deprecate `good_search` alias (WS-B).
- **Medium:** confidence gating (relative score/drop-off + flags) + eval assertions (WS-C + WS-E).
- **Medium:** fast_mode decoupling into orthogonal knobs + legacy compatibility mapping (WS-G).
- **Medium–High:** slate formatting/projection (selection, grouping, budgets, deterministic ordering) (WS-A).
- **Medium–High:** session tokens + delta-only slates + lightweight context graph (WS-H).
- **High:** layered stores / dataset overlays (if we choose “no reindex on profile switches” as a hard requirement) (WS-D).

## Appendix A) GoodFarmingAI-specific ignore/profile candidates (illustrative)

Exact paths will be finalized against the current GoodFarmingAI tree, but the intent is consistent:

- Exclude generated or archival output trees that are high-noise and rarely SSOT (e.g. `Phenology/archives/`,
  `Phenology/agent_outputs/`, `Phenology/external_reviews/`).
- Keep SSOT docs and diagrams indexed (Engine/Docs/**, Docs/diagrams/**).
- Keep configs and runbooks indexed (YAML/TOML/JSON/README patterns).
- Allow datasets only under an explicit dataset profile and explicit storage budgets.
