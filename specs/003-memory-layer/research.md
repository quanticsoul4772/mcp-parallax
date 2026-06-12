# Phase 0 Research: Memory Layer

**Date**: 2026-06-12 · **Sources**: `docs/design/MEMORY_LAYER.md`,
`SDK_LANDSCAPE.md` §memory, `OFFLOAD_LANDSCAPE.md` §F/§G, the 001/002
implementations. One genuine unknown (S1); the rest are decisions.

## S1 — The gating spike: vector search under sqlx without unsafe

- **Question** (from `SDK_LANDSCAPE.md`: "spike the sqlx loading caveat before
  committing"): can sqlite-vec load into our sqlx pool given
  `#![forbid(unsafe_code)]`?
- **Finding**: the loading mechanism exists — sqlx's per-connection hook
  (`after_connect`/`connect_with`) *is* the pool-wide mechanism, proven by a
  two-connection `vec_version()` test. (Fittingly, this design came out of
  unstick's own first production invocation.) But every registration route
  costs something the constitution prices high: the `sqlite-vec` crate's init
  requires an `unsafe extern` call (forbidden in this crate; isolating it
  means a workspace split), and `SqliteConnectOptions::extension()` requires
  shipping per-platform loadable binaries.
- **Decision**: **v1 does not need sqlite-vec at all.** Store embeddings as
  BLOBs; score with brute-force cosine in process. At v1 scale (≤ 10k
  memories × 1024 dims ≈ 40 MB, ~10M multiply-adds per query) scoring is
  single-digit milliseconds — `examples/spike_bruteforce.rs` validates the
  blob round-trip through sqlx and asserts < 50 ms at 5k memories.
- **Named deviation**: `SDK_LANDSCAPE.md` §memory picked sqlite-vec for v1;
  this plan defers it to the scale path and amends the doc in the same change
  (Constitution I). Revisit when the store approaches ~50k memories or recall
  latency data says so.

## D2 — Embedder seam + thin Voyage client

- **Decision**: new trait `Embedder { embed_document(text), embed_query(text) }`
  returning the vector + token usage; implemented by a thin reqwest client
  against Voyage (`voyage-4` default, `VOYAGE_MODEL` to override), mirroring
  the Anthropic client's shape (retry/backoff/timeout, distinct failure
  class `embedding_provider`).
- **Rationale**: Voyage embeddings are asymmetric — `input_type:
  "document"` at save vs `"query"` at recall measurably improves retrieval;
  baking the distinction into the seam makes misuse impossible. Voyage 4 is
  the corpus keeper (shared embedding space across the family → no re-index
  on model switch).
- **Alternatives considered**: reusing `ModelClient` (wrong shape — returns
  JSON values, not vectors); local embedding models via candle/ort (heavy,
  and the corpus picked Voyage).

## D3 — Trust model and verify-at-save

- **Decision**: `trust ∈ {first_hand, verified, untrusted}` derived, never
  caller-set: first-hand provenance → `first_hand`; external + verification
  requested → run the existing verify ensemble on the content; supported →
  `verified`, refuted → **save rejected** with the findings; external without
  verification → `untrusted` (stored, labeled, down-ranked).
- **Rationale**: `MEMORY_LAYER.md`'s central move ("verify before you store…
  curated, not credulous") implemented with the corrective we already trust;
  rejection-on-refutation makes poisoning attempts visible instead of silent.
- **Alternatives considered**: refusing external saves entirely (loses real
  value — research results are external by nature); auto-verifying every save
  (3× model calls on every write; the corpus reserves verification for the
  untrusted path).

## D4 — Ranking: relevance-dominant, trust-tiered, recency tie-break

- **Decision**: per-memory `score = cosine(query, embedding) +
  0.02 × recency_decay` (half-life 30 days), computed per trust tier;
  results are ordered trusted-tiers-first only *within an ε relevance band*
  (ε = 0.05): an untrusted memory may appear above a trusted one only when
  its relevance advantage exceeds ε. Pure functions in `ranking.rs`,
  property-tested against FR-004's three clauses.
- **Rationale**: FR-004 verbatim — relevance dominates, recency breaks
  near-ties, untrusted never outranks trusted *of comparable relevance*. A
  multiplicative trust penalty was rejected because it silently mixes trust
  into relevance and makes the FR untestable.
- **Importance term deferred** (spec assumption): needs an LLM pass per write.

## D5 — Capability gating (FR-007) and catalog filtering

- **Decision**: `Config.voyage_api_key: Option<String>`; when `None`, the
  three memory tools are removed from the `ToolRouter` at construction and
  `Embedder` is never built. Implementation order of preference: (a)
  `ToolRouter` route-removal API if rmcp 1.7 exposes one; (b) compose two
  `#[tool_router(router = ...)]` blocks (rmcp supports named routers) and
  merge conditionally; (c) manual `list_tools`/`call_tool` filtering in the
  handler. The integration test pins the observable behavior (catalog
  without key == exactly the 002 catalog), not the mechanism.
- **Rationale**: Constitution VI; SC-005 makes "no behavior change without
  the key" the regression gate.

## D6 — Memory tool schemas are MCP-side only

- **Decision**: save/recall/forget outputs never travel the model hop, so the
  Anthropic grammar subset and the registry's flat invariant do not apply to
  them; recall's output nests an array of memory objects. Contracts are
  declared in `contracts/` and pinned by tests; local validation still runs.
  The only model-hop schema in this feature is verify's, unchanged.
- **Rationale**: the constrained-output contract (Constitution II) governs
  model generation; over-applying it to server-computed results would force a
  worse API for no grammar benefit. Documented here precisely because future
  contributors will pattern-match on "all schemas flat."

## D7 — Config: generic input bound (paying the 002 naming debt)

- **Decision**: introduce `INPUT_MAX_CHARS` (default 50 000) as the generic
  per-tool input bound used by verify, unstick, and memory; honor
  `VERIFY_MAX_CLAIM_CHARS` as a fallback alias (read when the new var is
  unset) so existing deployments keep working; error messages name
  `INPUT_MAX_CHARS`.
- **Rationale**: the 002 design review named this debt "at the next config
  touch" — this is that touch.

## Risks

- **Embedding-space coupling**: changing `VOYAGE_MODEL` across the voyage-4
  family keeps the shared space (corpus claim); switching families would
  silently degrade recall. Record the embedding model per memory row so a
  mismatch is detectable.
- **Recall precision is the product** (`MEMORY_LAYER.md`): SC-001's
  acceptance set is the measure; if it misses, the first lever is the
  document/query input-type usage, the second is the ε band.
- **Catalog gating mechanism** (D5) is the main rmcp-API unknown — resolved
  in implementation with the behavior pinned by test either way.
