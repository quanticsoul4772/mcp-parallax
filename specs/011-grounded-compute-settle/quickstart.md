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
