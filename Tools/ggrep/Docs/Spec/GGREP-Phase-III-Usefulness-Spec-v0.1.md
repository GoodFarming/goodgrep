# GGREP Phase III Usefulness Spec (v0.1)

Status: Draft v0.1  
Scope: `Tools/ggrep` (goodgrep repo)  
Primary target repo: GoodFarmingAI (hybrid corpus: code + docs + diagrams)  
Purpose: Define what “GGREP is the default tool for agents” means in concrete, testable requirements.

This document is **normative** where it uses MUST/SHOULD/MAY language.

For current-system behavior, see:

- `Tools/ggrep/Docs/Spec/GGREP-System-Preamble-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Snapshot-Index-Contracts-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Query-Daemon-Contracts-v0.1.md`

---

## 1) Problem statement

GGREP is already excellent at “where is the thing?” (entrypoint discovery), but agents often need a *coherent
working slate*:

- How the system is supposed to work (plans/specs)
- Where it fits in architecture (diagrams)
- Where it’s implemented (code)
- What the operational shape is (configs, runbooks, CLI/env)
- Where to safely modify it (hot paths + extension points)

Today, agents can assemble this manually by running multiple searches and opening many files. Phase III makes
GGREP output that slate directly, with strict budgets and progressive disclosure.

---

## 2) Phase III north star

When an agent asks:

> “How is auth handled in this app?”

GGREP MUST be able to return a structured pack that includes:

1) The **routing/entrypoints** (where auth is applied)
2) The **mechanism** (session/token/headers/claims/permissions)
3) The **configuration** surface (env vars, config files, defaults)
4) The **policy** surface (roles, permissions, allow/deny rules)
5) The **integration boundaries** (middleware, adapters, external providers)
6) The **tests/runbooks** that validate/operate it
7) The **diagrams/plans** that explain it (when present)

…with line numbers and file paths so the agent can immediately jump to edits.

The same must hold for other “deep context” questions (e.g. “citrus phenology”, “marker ledger”, “pack designer”).

---

## 3) Design principles (requirements)

### USE.MUST.001 — Progressive disclosure by default

GGREP MUST support a safe, low-leakage default path for agents:

- paths-first → evidence snippets → expanded context → full content (only when explicitly requested).

This MUST be available in both CLI and MCP surfaces (no “CLI has safe knobs, MCP does not” mismatch).

### USE.MUST.002 — Coherent packs, not raw top-N chunks

GGREP MUST offer a first-class “pack” output that groups evidence coherently, prioritizing:

- conceptual coverage across code + docs + diagrams,
- within-file coherence (anchor + best evidence),
- minimal redundancy (dedupe and merge),
- navigability (paths + line numbers + “next action” hints).

### USE.MUST.003 — Explicit confidence and “no strong matches”

GGREP MUST NOT return arbitrary low-quality results without signaling.

- When confidence is low, GGREP MUST emit an explicit “low confidence / no strong matches” indicator and SHOULD
  return fewer results (or none) rather than flooding the pack with noise.
- Confidence MUST be computed deterministically from retrieval signals available at query time (e.g. score
  distribution and **relative separation** (drop-off / z-score / ratio), with optional dense-vs-lexical agreement as
  a guardrail. Confidence MUST NOT rely on absolute thresholds that drift with model versions.

### USE.MUST.004 — Budgeted output (token and bytes)

All pack outputs MUST be governed by explicit, enforceable budgets:

- max total bytes
- max bytes per file section
- max number of files
- max number of evidence chunks per file

Budgets MUST be surfaced in JSON metadata as `limits` and `limits_hit`.

Budgets MUST be enforced in **bytes**. Implementations MAY emit an **estimated token count** derived from bytes
(e.g. `estimated_tokens ~= total_bytes / 4`) but MUST NOT depend on a tokenizer for enforcement.

### USE.MUST.005 — Repo-SSOT profiles (index + query)

GGREP MUST support tracked repo configuration (already present as `.ggrep.toml`) that can define:

- index profiles (what is eligible to be indexed),
- query profiles (pack defaults, quotas, snippet depth),
- safety budgets.

Repo config MUST remain “untrusted input” and must not be able to disable hard safety caps.

### USE.MUST.006 — Determinism and reproducibility

Given a fixed snapshot id, query string, and query profile, GGREP MUST produce:

- deterministic selection and ordering of pack components,
- deterministic warnings/limits ordering,
- stable JSON schemas with versioning.

This is required to regress usability improvements safely (eval gating).

### USE.MUST.007 — Orthogonal knobs (index vs query vs formatting)

“Speed” and “detail” knobs MUST be orthogonal and correctly fingerprinted:

- Index-shape knobs (what is indexed, how chunks are generated) MUST be part of the config fingerprint (index identity).
- Query-time ranking knobs (e.g. rerank, query encoding) MUST be part of the query fingerprint.
- Output shaping knobs (anchors, snippet depth, slate level/format) MUST be part of the query fingerprint.

Legacy “one flag controls everything” (e.g. `fast_mode`) MUST be decomposed before slate becomes the default.

---

## 4) Core feature: “Slate” packs

Phase III introduces a new output concept: **Slate** (aka “working slate”).

The slate is an agent-oriented synthesis of retrieval results, produced without an LLM and without requiring the
agent to open dozens of files just to form a mental model.

### USE.MUST.010 — `slate` output exists (CLI + MCP)

Slate MUST be implemented as a **projection of the standard search pipeline**, not a separate retrieval stack.

GGREP MUST expose a slate generation entrypoint in:

- CLI (`ggrep search --format=slate "query"`; `ggrep slate ...` MAY exist as a thin alias)
- MCP (`context` tool; internally this is the “slate” behavior)

### USE.MUST.011 — Slate structure

The slate MUST include these sections (when relevant evidence exists):

1) **Overview**
   - what GGREP believes the query is about (short, provenance-only; no hallucinated claims)
2) **Code**
   - entrypoints + core mechanism + extension points
3) **Docs**
   - relevant plans/specs/runbooks with heading breadcrumbs (where applicable)
4) **Diagrams**
   - relevant diagrams and a human-readable extracted structure (edges/messages), not raw Mermaid when possible
5) **Config & Ops**
   - config files, env vars, flags, operational docs (status/health/runbooks)
6) **Tests**
   - validating tests and fixtures that anchor behavior

Each item MUST include:

- repo-relative file path
- line number (or best-effort location)
- a compact evidence snippet (budgeted)
- semantic tags (e.g. Anchor/Definition/Test/Doc/Graph)
- a `reason` field explaining why the file/item was included (e.g. `semantic_match`, `entrypoint_rule`,
  `config_rule`, `anchor_for_context`)

### USE.MUST.012 — File-centric grouping (anchor + evidence)

Within the slate, GGREP MUST group evidence by file. For each selected file, the slate SHOULD include:

- the file’s anchor chunk (imports/exports/top comments/preamble) *or* equivalent file summary
- 1–N evidence chunks relevant to the query

### USE.MUST.013 — Coverage-aware selection

Slate selection MUST optimize for coverage, not just top scores:

- include at least one “entrypoint-ish” file when present (e.g. API routes, middleware, top-level wiring)
- include at least one “policy-ish” file when present (e.g. permissions, roles, checks)
- include at least one “configuration-ish” file when present (env/config)
- include docs/diagrams when the repo contains them and they match above a threshold

This is a *selection policy* layered on top of retrieval, not a replacement for semantic ranking.

Entrypoint/policy/config/test detection SHOULD be primarily driven by repo configuration (e.g. path globs/regex in
`.ggrep.toml`) so the policy is testable and cheap. Expensive graph-centrality heuristics are optional and deferred.

### USE.MUST.014 — Progressive disclosure inside the slate

Slate results MUST be expandable deterministically:

- Level 0: paths-only slate (safe triage)
- Level 1: anchor + short evidence snippets
- Level 2: anchor + long evidence + neighbor context
- Level 3: full chunk content (explicit opt-in)

The same slate should be reproducible across levels (selection stable; only snippet depth changes).

“Neighbor context” expansion MUST be deterministic and SHOULD prefer semantic boundaries (e.g. Markdown heading
blocks, code function/class boundaries) within the byte budget, rather than fixed line counts.

### USE.MUST.015 — Deterministic ordering (slate)

Slate output MUST define an explicit ordering:

- Group results by file.
- Sort files by (max evidence score desc), then (path asc) as a stable tiebreaker.
- Sort evidence within a file by (start offset asc), then (chunk id asc).

### USE.MUST.016 — Exploration sessions (snapshot-pinned)

GGREP MUST support optional **exploration sessions** for slate/context requests to enable multi-hop agent workflows
without thrash.

- A session MUST pin a single `snapshot_id` for follow-up requests (even if a newer snapshot becomes active).
- A session MUST be represented by an opaque `session_token` (preferred: stateless token; daemon-stateful sessions MAY
  exist but MUST be bounded with TTL).
- `session_token` MUST be treated as untrusted input and MUST be size-bounded.

### USE.MUST.017 — Delta-only followups (no repeats)

When a session is provided and `delta_only=true`, GGREP MUST:

- suppress files (and optionally evidence chunks) that were already returned earlier in the session
- do so deterministically (no stochastic sampling)
- return an updated `session_token` and a stable `seen_set_digest` so clients can verify continuity

### USE.MUST.018 — Context graph (metadata-first orientation)

Slate/context responses MUST be able to include a lightweight, budgeted **context graph** that helps an agent orient
and navigate:

- Nodes MUST correspond to items already in the slate (file nodes; chunk subnodes are optional).
- Edges MUST be derived only from cheap signals over the selected items (e.g. import adjacency from anchors, markdown
  links, mermaid edges/messages, config-key mentions, test mentions).
- The graph MUST NOT contain raw content unless the disclosure level explicitly requests snippets/content.
- Node/edge counts and byte size MUST be capped and surfaced via `limits` / `limits_hit`.
- Ordering MUST be deterministic (sorted nodes/edges with stable tie-breaks).

---

## 5) MCP parity and agent safety

### USE.MUST.020 — MCP output shaping controls

MCP MUST expose safe output shaping equivalent to CLI:

- paths-only / compact mode
- no-snippet mode
- snippet depth controls
- explicit “include anchors” toggle

### USE.MUST.021 — MCP defaults are conservative

Default MCP behavior SHOULD be “paths-first” or “slate level 1” depending on client needs, but MUST never default
to full content.

MCP tool surfaces SHOULD converge on a stable set (`search`, `context`) and deprecate legacy aliases (e.g.
`good_search`) to reduce client confusion.

---

## 6) Index profiles (selective inclusion, datasets, noise suppression)

Phase III introduces the concept of **index profiles**: named policies for what is eligible to index.

Motivation:

- Some repos contain large or noisy trees (archives, agent outputs, raw datasets).
- Sometimes datasets *are* important (e.g. schema fixtures, labeled eval corpora), but only when storage budgets
  and governance exist.

### USE.MUST.030 — Explicit profile selection

GGREP MUST allow selecting:

- an index profile (eligibility for indexing), and
- a query profile (pack defaults, quotas, and output shaping), and optionally
- a view-only path filter (subset selection for “SSOT hunting”)

…via:

- repo config (`.ggrep.toml`) default profile
- CLI override (`--profile <name>` or equivalent)

### USE.MUST.031 — Profiles are fingerprinted

Index profile selection MUST be part of the **index identity** (config fingerprint inputs) when it changes
eligibility. Profiles that *expand* eligibility beyond what the current store contains MUST trigger a new index
identity (reindex/new store, or layered stores).

Query profiles and view-only filters that only *narrow* scope MUST NOT require reindex and MUST be included in the
query fingerprint.

### USE.MUST.032 — Profile-driven ignore behavior

Profiles MUST be able to:

- exclude known-noise trees (e.g. `Phenology/archives/`, `Phenology/agent_outputs/`)
- optionally include `Datasets/**` under explicit budgets

### USE.MUST.033 — Budgeted dataset inclusion

If a profile includes datasets, it MUST also require explicit budgets, at minimum:

- max bytes indexed per sync
- max file size
- max store disk usage (store budget)

If budgets are exceeded, GGREP MUST fail fast or degrade deterministically (depending on `--allow-degraded`).

Degradation strategy MUST be deterministic (e.g. lexicographic truncation or hash-based sampling with a stable seed),
so snapshot reproducibility is preserved.

---

## 7) Evaluation and regression gating (GoodFarmingAI)

### USE.MUST.040 — Expand eval from “find file” to “slate usefulness”

The eval suite MUST expand beyond “expected path contains” to validate slate usefulness, including:

- coverage: presence of at least one file in each required slate section when ground truth exists
- precision: avoid returning irrelevant files for low-confidence queries
- reproducibility: stable outputs across runs for the same snapshot id

### USE.SHOULD.041 — Domain cases (GoodFarmingAI)

The suite SHOULD include domain cases that reflect real agent work:

- “How is auth handled?”
- “How does citrus phenology work end-to-end?”
- “What is the marker ledger, where is it written, and how is it used?”

Each case SHOULD pin to:

- at least one diagram path (if present),
- at least one plan/spec path,
- at least one code entrypoint path,
- at least one core mechanism path.

---

## 8) Non-goals (Phase III)

- Changing embedding model families as the primary lever.
- Requiring cloud services or external APIs.
- Turning GGREP into an LLM summarizer; GGREP’s job is to *retrieve and structure evidence* under strict budgets.
