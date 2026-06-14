# Quickstart: Grounded Compute-Settle

No new configuration. `grounded_verify` (gated on `GROUNDED_VERIFY_ROOT`) now *settles*
the narrow class of computable claims it abstained on in 010, instead of routing them
back to the caller.

## A countable claim is answered, not bounced

```jsonc
{ "claim": "src/server.rs is over 1000 lines", "locators": [ { "path": "src/server.rs" } ] }
// 010: => { "verdict": "inconclusive", "reason": "computable property — route to `check`" }
// 011: => { "verdict": "supported", "executed_form": "1224 > 1000", "engine_result": "true",
//           "findings": ["counted 1224 lines"] }
```

The server counts the lines over the verbatim bytes it read and lets the deterministic
engine decide — the same auditable form a direct `check` returns. A false comparison
settles the other way:

```jsonc
{ "claim": "src/server.rs is over 5000 lines", "locators": [ { "path": "src/server.rs" } ] }
// => { "verdict": "refuted", "executed_form": "1224 > 5000", "engine_result": "false" }
```

## Still abstains outside the narrow class

Anything that is not a line/byte/literal-match count of a **single** source vs a numeric
threshold falls back to 010's behavior — no computed verdict over a value the server
could not derive:

```jsonc
// multi-source aggregate, or a property needing parsing
// => { "verdict": "inconclusive", "reason": "computable property — route to `check`" }
```

A non-computable judgment claim is unchanged — stance-blind passes, `supported`/`refuted`.

## Supported computable class (v1)

- **lines** — line count of the source.
- **bytes** — byte/size of the source.
- **matches** — count of a literal string in the source.

Each compared with a numeric threshold (`>`, `>=`, `<`, `<=`, `==`, `!=`), over a
**single** named source. Everything else abstains.

## Validation

- Offline (`cargo test`): the reproduction (1224-line fixture) returns `supported` with
  `1224 > 1000`; the `> 5000` variant returns `refuted`; an out-of-class or multi-source
  computable claim returns `inconclusive`; a non-computable claim is unchanged. All
  deterministic — the value is server-counted, so unlike 010 there is **no**
  live-model-only check here.

## Validation results (full gate, 2026-06-14)

`cargo fmt --all -- --check` clean · `cargo clippy --all-features --all-targets -- -D
warnings` clean · `cargo test` **319 lib + 53 integration, 0 failed** ·
`cargo run --example acceptance_grounded_verify` ALL CHECKS PASS (incl. `011 SC-001
computable claim ⇒ settled supported (1224 > 1000)`).

New coverage:

- **Counting (pure):** line convention pinned (LF-terminated and unterminated both → N;
  empty → 0); byte and literal-match counts; `ComputeSpec::from_pass` validates the
  property/operator strings server-side (unrecognized → out-of-class), `matches` requires
  a non-empty literal.
- **Settle (US1):** `lines > 1000` over a 1224-line source → `supported`, `1224 > 1000`,
  `engine_result` `true`, `findings` `["counted 1224 lines"]`, confidence 1.0;
  `> 5000` → `refuted`; byte and match specs settle; a lone computable pass (supported +
  empty findings) is accepted by `one_pass`, not dropped (M1).
- **Abstain (US2):** disagreeing specs, out-of-class property, multi-source
  (`units.len() == 2`), and **compound claims** (a valid spec plus a substantive judgment
  finding, M2) all return `inconclusive`; the non-computable judgment path carries no
  `executed_form`.
- **H1 confirmed:** the nullable-string compute fields register flat+closed at boot
  (`pass_schema_registers_flat_and_closed...` passes) — no `anyOf` rejection.

**011 is complete** — fully offline (no live dogfood needed; the value is server-counted).
