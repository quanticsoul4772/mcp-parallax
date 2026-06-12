# Data Model: Memory Layer

**Date**: 2026-06-12 · **Source**: spec.md Key Entities + research.md D2-D7

## 1. Memory (table `memories`, idempotent migration on the existing store)

| Column | Type | Notes |
|---|---|---|
| `id` | TEXT PK | UUID v4 |
| `content` | TEXT | the memory itself; bounded by `INPUT_MAX_CHARS` |
| `kind` | TEXT | `skill` \| `lesson` \| `fact` |
| `origin` | TEXT | caller-stated provenance ("solved in session X", "from a URL") |
| `external` | INTEGER (bool) | first-hand (0) vs external (1) — the poisoning pivot |
| `trust` | TEXT | `first_hand` \| `verified` \| `untrusted` — derived, never caller-set (D3) |
| `tags` | TEXT | JSON array of strings |
| `embedding` | BLOB | f32 little-endian vector (document-type embedding) |
| `embedding_model` | TEXT | e.g. `voyage-4` — mismatch detection (risk note) |
| `created_at` | TEXT | RFC 3339 via `TimeProvider` |

## 2. Tool inputs/outputs (MCP-side only — D6; contracts in `contracts/`)

**save** in: `{ content: string, kind: enum, origin: string, external: bool,
tags?: string[], verify?: bool }` → out: `{ id: string, trust: enum,
findings: string[] }` (findings non-empty only when verification ran; a
refuted save is an error carrying the findings, not a result).

**recall** in: `{ query: string, kind?: enum|null, limit?: int (1..=MEMORY_RECALL_LIMIT_MAX 20, default`MEMORY_RECALL_LIMIT`5) }` →
out: `{ memories: [ { id, content, kind, origin, external, trust, created_at,
score } ] }` — nested array is legal here (no model hop).

**forget** in: `{ id: string }` → out: `{ forgotten: bool }`; unknown id →
distinct not-found error (`invalid_input` class with "no memory with id").

## 3. Ranking (pure functions, `ranking.rs` — D4)

`effective = cosine + 0.02 × 2^(−age_days/30) + (ε=0.05 if trusted)`; sort by
effective desc, deterministic id tie-break. Adding ε to trusted memories
implements the band as one clean total order: `{first_hand, verified}` precede
`untrusted` at comparable relevance, and an untrusted memory outranks a
trusted one only when its relevance advantage exceeds ε adjusted by the
recency delta (at most ±0.02 — the effective band is 0.03..0.07 depending on
relative age). The reported `score` stays the raw cosine.
Properties pinned by tests: (1) relevance dominates beyond the band;
(2) recency breaks near-ties; (3) untrusted never above trusted within the
band; (4) the band edge under a maximal age gap (0.03).

## 4. Outcome taxonomy extension

New class `embedding_provider` (Voyage refusal/5xx/timeout/etc. surface
through the same retry policy as the Anthropic client, terminal classes map
to this one class + the existing `timeout`/`retries_exhausted` where exact).
`AppError::EmbeddingProvider(String)` ↔ record outcome `embedding_provider`.
Contract enum in 001's record schema gains the value (addition, not a
change to existing values).

## 5. Config (extended — D5/D7)

| Var | Default | Notes |
|---|---|---|
| `VOYAGE_API_KEY` | unset | **presence enables the memory capability** (FR-007) |
| `VOYAGE_MODEL` | `voyage-4` | shared embedding space across the family |
| `MEMORY_RECALL_LIMIT` | `5` | default top-k; server max 20 |
| `INPUT_MAX_CHARS` | `50000` | generic input bound; `VERIFY_MAX_CLAIM_CHARS` honored as fallback alias (D7) |

## 6. Seams

`Embedder { embed_document(&str) -> (Vec<f32>, usage), embed_query(&str) -> (Vec<f32>, usage) }`
— mockable; implemented by `client/voyage.rs`. Memory store functions take the
existing `SqliteStorage` pool. Verify-at-save calls `verify::run` via the
registry (unchanged).

## Relationships

```text
Config(voyage key present?) ──no──► tools absent from catalog; no Embedder built
        │ yes
        ▼
save ── Embedder.embed_document ──► memories row (trust per D3; external+verify → verify::run)
recall ─ Embedder.embed_query ───► ranking.rs over all rows ──► top-k
forget ───────────────────────────► DELETE by id
all three ──► run_recorded ──► InvocationRecord (tool = save|recall|forget)
```
