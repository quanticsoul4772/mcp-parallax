# Quickstart: Push Memory

## Build & gate

```bash
cargo build
cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test
```

No new configuration: `surface` enters the catalog only when `VOYAGE_API_KEY`
is set, and nothing invokes it until the `UserPromptSubmit` hook entry from
`integrations/claude-code/hooks.json` is installed.

## S2 spike (precondition for the integration entry)

S1 (006) never exercised `UserPromptSubmit`. Before the hooks.json entry is
final, live-verify with a real Claude Code session:

1. Add a provisional `UserPromptSubmit` mcp_tool hook pointing at `surface`.
2. Submit a prompt; capture what payload fields the harness actually passes
   (`prompt` vs `user_prompt`; substitution stringification per S1 round 2).
3. Return a non-empty result and confirm `additionalContext` reaches the
   model's context (ask the model to quote it back).
4. Record the verified shapes in `examples/spike_hooks.md` (S2 section) and
   fix `hooks.json` + the result mapping accordingly.

## Seam-level verification (no network)

```bash
cargo test push                      # unit: selection pipeline, template, suppression
cargo test --test integration surface # end-to-end on wiremock embeddings
```

Expected coverage per user story:

- US1: seeded trusted memory + related prompt ⇒ surfaced with id/trust/label;
  cap and ordering respected; advisory template exact.
- US2: unrelated prompt ⇒ silence (no hookSpecificOutput); memory-off ⇒
  tool absent from catalog, all existing tests unchanged; embedder failure
  and budget timeout ⇒ fail-open silence + record; untrusted never surfaced;
  second call same session ⇒ suppressed; new session ⇒ surfaces again.
- US3: exactly one `push_records` row per evaluation (surfaced ids, silence,
  degraded); `pushed_memory_ids` reads back what was recorded.

## Live dogfood (SC-001/SC-002/SC-006, after merge + rebuild + S2-verified hooks)

1. `save` a first-hand fact tied to a distinctive topic; note the id.
2. New session; submit a prompt about that topic ⇒ expect the labeled
   memory block in context (the model will reference it unprompted).
3. Same session, related prompt again ⇒ expect no repeat (SC-006).
4. Unrelated prompt ⇒ expect nothing (SC-002).
5. Inspect `push_records`: three rows — one surfacing, two silences.
6. `forget` the seed.

## Rollback

Revert the commits. The `push_records` table is additive and inert for old
builds; the hooks entry uninstalls by removing it (nothing else changes —
the 006 uninstall property).
