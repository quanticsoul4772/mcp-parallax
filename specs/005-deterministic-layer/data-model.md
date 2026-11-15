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
| engine | "arithmetic" \| "constraints" \| null | one-part claim: null iff not_checkable; bundle: set only when every part ran on the same engine |
| formal_form | string \| null | the executed expression / SMT-LIB 2 script (FR-007 auditability) |
| engine_result | string \| null | raw engine output: evaluated value, or sat/unsat |
| witness | string \| null | solver model: satisfying assignment (sat-supports) or counterexample (sat-refutes) |
| explanation | string | deterministic template over (claim, form, result) — never model-phrased (D4) |
| reason | string \| null | why not checkable (verdict = not_checkable only) |
| translation_attempts | integer | 1 or 2 (the violation-fed retry — FR-005) |
| parts | array (1–12) of part results | one per separately checked part, in claim order: claim, verdict, engine, formal_form, engine_result, witness, reason |

Field consistency (server-guaranteed), for every entry of `parts` and for the
top level of a one-part claim: verdict ≠ not_checkable ⇒ engine, formal_form,
engine_result present; verdict = not_checkable ⇒ reason present,
engine/formal_form/engine_result/witness null. For a bundle of two or more
parts the top level is a summary: verdict is refuted if any part is refuted,
supported only if every part is supported, not_checkable otherwise;
formal_form and engine_result hold one `[n] …` line per part whenever any part
was executed; witness lists only the parts that have one; reason lists the
parts that were not checked, and only when the verdict is not_checkable.

## 3. Model-hop schema (flat + closed — Principle II)

One call, classify + translate (research.md D3):

| field | type | notes |
|---|---|---|
| parts | string[] | each part restated to stand alone; 1 to 12 entries |
| checkable | boolean[] | per part; decline-biased |
| reasons | string[] | per part; empty string when checkable |
| engines | string[] | per part; "arithmetic" or "constraints", empty string when not checkable (values enforced by the cross-field validator) |
| forms | string[] | per part; the expression (≤ 2000 chars) or the SMT-LIB 2 declares + asserts (≤ 10000 chars), empty string when not checkable |
| asserted | string[] | per part; "satisfiable" or "unsatisfiable" for a constraints part, empty string otherwise |

The six lists are index-aligned and must be the same length; flat+closed
forbids an array of objects.

Cross-field consistency is checked by a pure validator after parse; a
violation is a translation violation (fed to the single retry, like an
engine parse error — D5).

## 4. Pure functions (`check.rs`, engine wrappers)

- `arithmetic::evaluate(expr) -> Result<bool, Violation>` — evalexpr;
  non-boolean result and parse/eval errors are Violations (retryable);
  expression length bound enforced first.
- `solver::check(smtlib, asserted) -> Result<SolverOutcome, Violation>` —
  parse errors are Violations; `SolverOutcome` = Sat(witness) | Unsat |
  Unknown (→ `timeout` class; message: "solver returned unknown (timeout or
  incompleteness)" — Z3 returns unknown for theory incompleteness too).
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
- `PARTS_MAX = 12` (parts per claim; more is a translation violation).
- `PARTS_CONCURRENCY = 4` (parts of one claim executing at once).

## 6. Outcome classes used by `check` (no taxonomy changes — D6)

`invalid_input` (FR-008), `refusal`/`truncation`/`retries_exhausted` (the
translation call, existing), `timeout` (provider timeout AND solver
timeout/unknown — messages distinct), `validation_failure` (schema violation
on the hop, or translation failed after the retry — message names
translation), `cancelled`. `not_checkable` is a **success** (the tool did
its job), carried in the result, not an error class.
