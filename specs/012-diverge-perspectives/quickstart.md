# Quickstart: Diverge — Independent Perspectives

No new configuration. `diverge` is always in the catalog (like `verify`/`unstick`).

## Break out of one framing

```jsonc
{ "problem": "Our onboarding flow has too many steps; we need to cut steps." }
// =>
{
  "perspectives": [
    { "lens": "invert",     "framing": "What if more steps, not fewer, is the fix — each step that earns trust?",
                            "implication": "Reframes the goal from brevity to confidence; measure completion, not length." },
    { "lens": "actor",      "framing": "Whose problem is this — is it the user's, or the team's metric?",
                            "implication": "If users aren't dropping off, the step count may be a vanity concern." },
    { "lens": "assumption", "framing": "The framing assumes steps cause the drop-off; what if it's the first step's ask?",
                            "implication": "Cutting steps wouldn't help if a single high-friction ask is the real cause." }
  ],
  "passes": 3
}
```

Each framing departs from the anchored "cut steps" reading in a different direction, and
each is labeled with the lens that produced it. The set is deduplicated — two passes that
land on the same reframing collapse to one.

## What it does not do

```jsonc
// Diverge never returns a verdict (use `verify`) or a single chosen step (use `unstick`):
{ "problem": "..." }   // => { "perspectives": [ ... ], "passes": 3 }   // framings only
```

A stated preference in `context` ("I think we should just rewrite it") does **not**
collapse the set — the passes are stance-blind and still surface framings that depart
from the preference.

## Validation

- Offline (`cargo test`): the `k` lens prompts differ; the per-pass schema is flat +
  closed; the deterministic dedup collapses constructed near-identical framings and keeps
  distinct ones; the prompt exposes only problem + context (no stance); zero completed
  passes returns the dominant failure.
- **Live** (dogfood): SC-001 — that real problems scatter into ≥3 distinct framings — and
  SC-003 — that a stated stance does not narrow the set — are confirmed against the running
  model (a mock cannot diverge), as `verify`'s SC-001 was.

## Validation results (full gate, 2026-06-14)

`cargo fmt --all -- --check` clean · `cargo clippy --all-features --all-targets -- -D
warnings` clean · `cargo test` **331 lib + 56 integration, 0 failed** ·
`cargo run --example acceptance_diverge` ALL CHECKS PASS.

New coverage:

- **Pure:** the `k` lens prompts are pairwise distinct; `LENSES` is non-empty with unique
  names and cycles at `k > len`; `normalize`/`jaccard` behave; `dedup` collapses
  near-identical framings (Jaccard ≥ 0.8, earliest lens kept) and keeps distinct ones;
  the per-pass schema registers flat + closed (two string fields, no `lens` model field).
- **Wired (mocked):** one lens-labeled `Perspective` per completed pass (invert/actor/
  horizon at k=3); an empty-`framing` pass is dropped; zero completions → dominant
  failure; an empty problem is rejected before any model call; identical framings across
  passes deduplicate to one.
- **Integration:** `diverge` is always in the catalog (no gate); distinct per-lens
  framings return lens-labeled and deduplicated with **no** `verdict`/`confidence` field
  and exactly one record; a stated preference in `context` is accepted and the tool stays
  blind (no extra stance slot).

**Pending live (T013, post-rebuild):** SC-001 (≥3 distinct framings), SC-003 (stance does
not narrow the set), and SC-004 (no over-divergence on a narrow problem) — the
offline-impossible checks (a mock cannot diverge). The `acceptance_diverge` example is the
offline scaffold.
