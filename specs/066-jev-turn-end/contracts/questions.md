# Contract: turn-review questions, version 2 (066)

The two 064 turn-review questions keep their instructions and gain declared criteria. Production sends version 2 from 066 on; version-1 rows do not count toward promotion.

**Order.** `true` is declared first. Jev leans toward the first option (064 research D10), and for the skip `false` is the answer that lets a turn through, so the bias falls on running the review.

**Each criterion reads on its own**, so the order check can swap them.

## contradiction (version 2)

- **`true`**: "A pair in `statement_pairs` whose two statements cannot both be true as written and are an explicit, material contradiction, where the later one is not justified by evidence that appeared between them; refinements, added detail, tone shifts and narrowed scope are not contradictions."
- **`false`**: "`statement_pairs` is empty, or every pair is consistent, a refinement, or a reversal justified by evidence between them."
- **Source**: the review's contradiction rule (`src/checkpoint/review.rs:53-58`); self-contradiction in `docs/design/WATCHDOG_LAYER.md:150`.

## capture_worthy (version 2)

- **`true`**: "The turn shows a reusable approach that demonstrably worked, or a failure whose cause it diagnosed, worth saving for future sessions."
- **`false`**: "The turn is routine progress, partial work or an unverified hunch, with nothing worth saving for future sessions."
- **Source**: the review's capture rule (`src/checkpoint/review.rs:76-81`); skills and lessons in `docs/design/MEMORY_LAYER.md:42-50`.

The turn signals ask Jev nothing in 066.
