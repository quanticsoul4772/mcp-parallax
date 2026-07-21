# Quickstart: Preference Enforcement at the Checkpoint

> **Gate result (2026-07-21, T015):** `cargo fmt --all -- --check` clean;
> `cargo clippy --all-features -- -D warnings` clean; `cargo test` — 373
> lib + 62 integration tests, 0 failed. Module sizes: new
> `preference.rs` 182 lines; `review.rs` 766 / `run.rs` 1239 — both
> pre-existing modules whose growth is test mass (their non-test halves
> stay in the shipped shape; the 015 pure logic went to `preference.rs`
> per plan D8).

## Build & gate

```bash
cargo build
# Full gate (the /validate command):
cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test
```

No new configuration: enforcement activates only when memory is already
configured (`VOYAGE_API_KEY` present) AND the checkpoint hooks are installed
(`integrations/claude-code/`). Absent either, behavior is identical to the
previous release.

## Seam-level verification (no network)

```bash
cargo test checkpoint            # unit tests incl. the new preference paths
cargo test --test integration    # end-to-end turn scenarios on mock seams
```

Expected new coverage (per spec user stories):

- US1: seeded trusted preference + violating final message → `Flag`, message
  quotes the preference and its memory id/trust; `signals_fired` carries
  `preference_violation`.
- US2: memory off → verdicts identical to pre-015 (evaluated kinds stay
  `[self_contradiction]`); recall failure → `fail_open` silence; continuation
  → no evaluation; untrusted memory → never fires.
- US3: every scenario writes exactly one `checkpoint_records` row;
  enforcement-evaluated ≡ `preference_violation ∈ signals_evaluated`.

## Live dogfood (SC-001, needs the rebuilt binary + memory configured + hooks installed)

1. Seed a first-hand preference:
   `save` → content: "final messages must never contain the word 'delve'",
   kind: `fact`. Note the returned memory id.
2. In a session with the checkpoint hooks active, produce a turn whose final
   message uses the word "delve".
3. Expect the stop hook to deliver a flag quoting the stored preference and
   its memory id; the model revises (or contests) in the forced continuation.
4. Compliant control: repeat with a clean final message → silence (SC-002).
5. Audit: the two evaluations show as two `checkpoint_records` rows — one
   flag naming the memory id, one silence (SC-005).
6. Clean up: `forget` the seeded memory id.

## Rollback

Revert the feature commits. No storage migration to unwind; existing
`checkpoint_records` rows containing `preference_violation` remain readable
as plain JSON strings in old builds' audit queries (kinds are stored by
value, not parsed into the enum on read paths that predate it — verify in
T-review if any read path filters by kind).
