# Quickstart: Memory Consolidation and Auto-Capture

> **Gate result (2026-07-23, T018):** `cargo fmt --all -- --check` clean;
> `cargo clippy --all-features -- -D warnings` clean; `cargo test` — 407
> lib + 69 integration tests, 0 failed. New module
> `src/memory/consolidate.rs` ≈ 530 lines including its full test mass.
> One implementation-time design correction, named: with memory configured
> the turn hop now runs at EVERY turn end (not only when candidates exist) —
> capture is an every-turn judgment and its screen is the hop's own decline
> bias; without memory the 006-era no-candidates-no-pass gate stands. This
> changes the turn boundary's cost profile for memory-on sessions and is
> recorded in the run_turn comment and the CHANGELOG entry.

## Build & gate

```bash
cargo build
cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test
```

No new configuration. Consolidation rides admission (memory-gated already);
capture rides the installed Stop hook (`checkpoint_turn`'s input is
unchanged — no integration edit).

## Migration check (the first ALTER TABLE)

```bash
cargo test migration          # includes the pre-017 fixture-DB compatibility test
```

The fixture test opens a database created by the pre-017 schema, runs
`connect`, and asserts: three new columns present with correct
defaults/backfill, all pre-existing rows byte-identical, `push_records`/
`checkpoint_records` untouched. Manually: back up
`%APPDATA%\Claude\parallax.db` before first launch of the new binary; the
migration is loud on failure and additive on success.

## Seam-level verification

```bash
cargo test consolidate        # screens, apply rules, trust guard, decline bias
cargo test push               # active-only filtering + reinforcement
cargo test checkpoint         # capture judgment, cap, quarantine, fail-open
cargo test --test integration # end-to-end admission + capture scenarios
```

Expected per user story: US1 — update supersedes, context coexists,
uncertain keeps both, superseded excluded from recall/push/gate but
inspectable; US2 — duplicate merges byte-identical survivor, trust guard
holds, dissimilar never merges; US3 — capture proposes ≤ cap quarantined
candidates, uneventful turns propose nothing, candidates never push-surface;
US4 — one audit row per action, zero content mutations, forget works on
every status.

## Live dogfood (post-merge + rebuild + restart)

1. **Supersession**: `save` a fact; `save` its update in different words →
   expect recall/push to return only the update; inspect the superseded
   original + the audit row.
2. **Berlin/Lisbon**: `save` a standing fact, then a context-specific
   statement on the same subject → both remain active.
3. **Merge**: `save` the same fact twice, reworded → one canonical,
   byte-identical survivor.
4. **Capture**: run a turn that solves something concrete; at turn end,
   check `consolidation_records` for a `capture_proposed` row and the
   candidate in `recall` (labeled untrusted); confirm push never surfaces
   it; `forget` it.
5. Record results in `tasks.md`; clean all seeds.

## Rollback

Revert the commits. The new columns and table are additive and inert for
old builds (old code neither reads nor writes them); no data loss either
direction.
