# Gemini review: GGREP Phase III docs (2026-01-04)

Model: `gemini-3-pro-preview` (Gemini CLI, headless wrapper)  
Reviewer role: “principal engineer”  
Reviewed docs:

- `Tools/ggrep/Docs/Spec/GGREP-Spec-Index-v0.2.md`
- `Tools/ggrep/Docs/Spec/GGREP-System-Preamble-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Phase-III-Usefulness-Spec-v0.1.md`
- `Tools/ggrep/Docs/Plan/GGREP-Phase-III-Usefulness-Plan-v0.1.md`
- `Tools/ggrep/Docs/Spec/GGREP-Query-Daemon-Contracts-v0.1.md`

Invocation (representative):

```bash
cat /tmp/gemini_phase3_review_prompt.txt | \
  bash Scripts/gemini_headless.sh \
    -- "Respond to stdin; no edits."
```

## High-level takeaways

- **Contracts/schema need to lead**: Slate/context output shape conflicts with `query_success` (flat `results`) unless we
  introduce a new `slate_success` schema or bump `query_success` to a new version.
- **Agent-facing metadata matters**: add standardized `estimated_tokens`, `confidence`, and `match_reason`/`reason` so
  agents can make safe decisions programmatically.
- **Profile UX needs discovery/orchestration**: if index profile changes imply a new `store_id`, clients need a way to
  discover profiles/stores and route requests to the correct daemon.

## Full Gemini output (verbatim)

### Section 1: Top 10 improvements/corrections

1.  **Define `slate_success` Schema Immediately:** The *Phase III Spec* and *Plan* are ambiguous about whether Slate fits into the existing `query_success` (v1) schema. The Contract's `results` array is flat; Slate requires a hierarchical or grouped structure (Code/Docs/Config). **Correction:** Explicitly define `slate_success` (v1) in the Contracts now to prevent schema drift, or define `query_success` (v2) with a polymorphic `results` shape.
2.  **Add `estimated_tokens` to Response Metadata:** `USE.MUST.004` rightly enforces bytes for safety, but agents operate on token budgets. **Improvement:** Add an `estimated_tokens` field (using a cheap heuristic like `bytes / 4`) to the `limits` or `usage` section of the JSON response to help agents manage context windows without implementing their own tokenizer.
3.  **Expose `match_reason` in Result Schema:** `USE.MUST.011` mentions a `reason` field (e.g., `semantic_match`, `entrypoint_rule`, `anchor_for_context`). **Correction:** Add this field to the Contract's result schema. It is critical for agents to understand *why* a file is present (evidence vs. context).
4.  **Formalize `confidence` Signal:** `USE.MUST.003` requires explicit low-confidence signaling. **Improvement:** Add a `confidence` field (e.g., enum `strong`, `weak`, `none` or a normalized `0.0-1.0` score separate from the raw ranking score) to the `query_success` schema so agents can programmatically decide to discard the slate.
5.  **Profile Discovery Mechanism:** If switching profiles changes the `store_id` (per `USE.MUST.031`), the CLI/Agent needs to discover available profiles *before* connecting to a specific daemon/store. **Improvement:** Add a `ggrep profiles` command or `discovery` mode to the CLI/Contract that lists valid profile/store-id pairs for a repo root.
6.  **Advertize Slate Capability in Handshake:** The Contract `IPC.MUST.001` negotiation is strict. **Correction:** Add `"slate"` to the `client_capabilities` list in the handshake example. The daemon should reject Slate requests from clients that didn't declare capability (or vice versa), ensuring the client can parse the new schema.
7.  **Standardize "Level" Parameter:** `USE.MUST.014` defines progressive disclosure levels (0-3). **Improvement:** Add `slate_level` (integer 0-3) to the request contract explicitly, rather than relying on loose flags (`--compact`, `--short-snippet`).
8.  **Decouple `fast_mode` in Contract Migration:** `fast_mode` is overloaded. **Correction:** The Plan proposes decoupling it. The Contract should explicitly map the legacy `fast_mode` flag to the new orthogonal knobs (`index.skip_definitions`, `query.skip_rerank`) to ensure backward compatibility for existing scripts while moving forward.
9.  **Budget-Exceeded Error Code:** **Improvement:** Differentiate "busy" (queue full) from "budget_exceeded" (request too large for Slate Level). If an agent asks for Level 3 and hits the byte cap, return a specific error or a warning with a partial result, guiding the agent to request a lower level.
10. **Explainability for Slate Selection:** `ggrep explain` focuses on retrieval timings. **Improvement:** For Slates, explainability must cover the *selection policy* (e.g., "File X included because it fills the 'Config' bucket quota").

### Section 2: Contradictions / missing constraints

*   **Schema Shape Conflict:** `GGREP-Query-Daemon-Contracts-v0.1.md` defines `query_success` with a flat `results` array. `GGREP-Phase-III-Usefulness-Spec-v0.1.md` describes Slate as a structured pack (Overview, Code, Docs).
    *   *Constraint:* Slate output MUST use a distinct top-level key (e.g. `slate`) or a distinct schema type to avoid breaking v1 clients expecting a flat list.
*   **Profile/Store Identity:** `USE.MUST.031` says expanding profiles triggers a new index identity (new Store ID). The Daemon Contract says a daemon pins a Store ID at startup.
    *   *Constraint:* The CLI MUST handle the orchestration of locating/spawning the correct daemon for a requested profile. A single daemon process cannot "switch" profiles if the Store ID changes.
*   **Missing "Slate" in Contracts:** The `GGREP-Query-Daemon-Contracts-v0.1.md` does not currently mention the `slate` format or its specific parameters (levels, buckets).
    *   *Constraint:* The Contract is SSOT. It must be updated to include the Slate request/response structures before implementation begins.
*   **Token Budgeting Gap:** The Spec forbids tokenizer-based *enforcement* (good for perf) but doesn't mandate a standard "token cost" reporting mechanism.
    *   *Constraint:* Agents need a standardized `estimated_tokens` metric to align with LLM context limits, even if enforcement is byte-based.

### Section 3: Evaluation ideas

*   **Slate Coherence & Coverage:**
    *   Test Case: "Auth" query.
    *   Assertion: Result MUST contain >0 files in "Code", >0 in "Config", >0 in "Docs" (if repo has them).
    *   Assertion: "Config" file returned is `auth.json` (or similar), not `random.xml`.
*   **Progressive Disclosure Determinism:**
    *   Test Case: Query "X", request Level 1, then Level 2.
    *   Assertion: Level 2 includes *all* files from Level 1 (stability), plus more content/context. Files should not disappear when increasing detail.
*   **Budget Enforcement:**
    *   Test Case: Request Level 3 (Full Content) on a query matching 100 files.
    *   Assertion: Response size < `max_response_bytes`. `limits_hit` contains `max_response_bytes`. Response is valid JSON (truncated safely).
*   **Profile Isolation:**
    *   Test Case: Index with `profile=default` (excludes `Datasets`). Query for dataset file -> 0 results.
    *   Test Case: Index with `profile=datasets`. Query for dataset file -> 1 result.
    *   Assertion: `store_id` for both runs is different.
*   **Confidence Precision:**
    *   Test Case: Nonsense query "color of the sky".
    *   Assertion: `confidence` is "low" or "none". Results list is empty or minimal (anchors only).

### Section 4: Sequencing recommendations

1.  **M1: Foundations & Parity (WS-B, WS-F, WS-G)**
    *   Fix `status` ambiguity (Store ID <-> Root).
    *   Decouple `fast_mode` (Index vs Query knobs).
    *   Align MCP/CLI flags (safe defaults).
2.  **M2: Contract Updates (Schema)**
    *   Update `GGREP-Query-Daemon-Contracts` with `slate_success` schema, `confidence` fields, and `match_reason`.
    *   Define Handshake capability `"slate"`.
3.  **M3: Slate MVP (WS-A)**
    *   Implement "Level 1" Slate (Paths + Anchors + Short Evidence).
    *   Implement "Coverage" selection logic (Buckets).
    *   Enforce Byte Budgets.
4.  **M4: Profiles & Dataset Support (WS-D)**
    *   Implement `.ggrep.toml` profile loading.
    *   Implement CLI orchestration to spawn daemons by Profile/Store ID.
5.  **M5: Advanced Usefulness (WS-C, WS-E)**
    *   Implement Confidence heuristics.
    *   Add "Overview" generation and Diagram summaries.
    *   Run full "Usefulness Eval" suite.
