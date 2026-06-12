# Data Model: Deterministic Layer (005)

Wire types in `deterministic/contract.rs` (MCP-side). The translation target
is the one model hop (flat+closed — Principle II). Engines are pure; nothing
persists except the standard invocation record.

## 1. Request (tool input — contract `check.tool.json`)

| field | type | rules |
|---|---|---|
| claim | string | required; non-empty after trim; ≤ INPUT_MAX_CHARS (FR-008) |
| context | string (nullable) | optional background the claim depends on; same bound |

## 2. Response (tool output)

| field | type | notes |
|---|---|---|
| verdict | "supported" \| "refuted" \| "not_checkable" | engine-decided (or honest decline — FR-004) |
| engine | "arithmetic" \| "constraints" \| null | null iff not_checkable |
| formal_form | string \| null | the executed expression / SMT-LIB 2 script (FR-007 auditability) |
| engine_result | string \| null | raw engine output: evaluated value, or sat/unsat |
| witness | string \| null | solver model: satisfying assignment (sat-supports) or counterexample (sat-refutes) |
| explanation | string | deterministic template over (claim, form, result) — never model-phrased (D4) |
| reason | string \| null | why not checkable (verdict = not_checkable only) |
| translation_attempts | integer | 1 or 2 (the violation-fed retry — FR-005) |

Field consistency (server-guaranteed): verdict ≠ not_checkable ⇒ engine,
formal_form, engine_result present; verdict = not_checkable ⇒ reason present,
engine/formal_form/engine_result/witness null.

## 3. Model-hop schema (flat + closed — Principle II)

One call, classify + translate (research.md D3):

| field | type | notes |
|---|---|---|
| checkable | boolean | decline-biased |
| reason | string \| null | required when checkable = false |
| engine | "arithmetic" \| "constraints" \| null | required when checkable |
| arithmetic_expression | string \| null | boolean-valued evalexpr expression; tolerances explicit; ≤ 2000 chars (local validator) |
| smtlib_constraints | string \| null | SMT-LIB 2 declares + asserts (no check-sat) |
| asserted | "satisfiable" \| "unsatisfiable" \| null | the claim's polarity; required for constraints |

Cross-field consistency is checked by a pure validator after parse; a
violation is a translation violation (fed to the single retry, like an
engine parse error — D5).

## 4. Pure functions (`check.rs`, engine wrappers)

- `arithmetic::evaluate(expr) -> Result<bool, Violation>` — evalexpr;
  non-boolean result and parse/eval errors are Violations (retryable);
  expression length bound enforced first.
- `solver::check(smtlib, asserted) -> Result<SolverOutcome, Violation>` —
  parse errors are Violations; `SolverOutcome` = Sat(witness) | Unsat |
  Unknown (→ `timeout` class).
- `verdict(engine_outcome, asserted) -> (Verdict, Option<witness>)` —
  sat × satisfiable → supported(witness); sat × unsatisfiable →
  refuted(counterexample); unsat × unsatisfiable → supported; unsat ×
  satisfiable → refuted; arithmetic true/false → supported/refuted.
- `explanation(claim, form, result, verdict) -> String` — deterministic
  template (D4).

## 5. Constants (`deterministic/mod.rs` — no new env vars, D6)

- `SOLVER_TIMEOUT_MS = 10_000` (in-engine; exceeded → existing `timeout`
  class, message names the solver).
- `EXPRESSION_MAX_CHARS = 2_000`; `SMTLIB_MAX_CHARS = 10_000`.
- `TRANSLATION_ATTEMPTS_MAX = 2` (initial + one violation-fed retry).

## 6. Outcome classes used by `check` (no taxonomy changes — D6)

`invalid_input` (FR-008), `refusal`/`truncation`/`retries_exhausted` (the
translation call, existing), `timeout` (provider timeout AND solver
timeout/unknown — messages distinct), `validation_failure` (schema violation
on the hop, or translation failed after the retry — message names
translation), `cancelled`. `not_checkable` is a **success** (the tool did
its job), carried in the result, not an error class.
