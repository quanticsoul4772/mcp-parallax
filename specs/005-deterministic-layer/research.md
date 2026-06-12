# Research: Deterministic Layer (005)

**Date**: 2026-06-12. Sources: `docs/design/DETERMINISTIC_LAYER.md`,
`docs/design/SDK_LANDSCAPE.md` §deterministic, crates.io searches run
2026-06-12, and the 003/004 implementation experience (D7 server-assembly,
mode registration, run_recorded).

## D1 — Constraint engine: `z3` 0.20, bundled, SMT-LIB 2 as the formal target

**Decision**: the `z3` crate 0.20.0 (prove-rs bindings, the landscape's named
pick) with the `bundled` feature (vendored build — no system Z3 install).
The model emits the constraint problem as an **SMT-LIB 2 script** (declares +
asserts, no `check-sat` — the engine appends it); the solver wrapper parses
it, checks satisfiability under a 10 s in-engine timeout, and extracts a
model (witness) on sat.

**Rationale**: corpus pick (Principle I). SMT-LIB 2 text as the target keeps
the model-facing contract a single string field (flat schema), makes parse
errors the free violation signal the feedback loop needs, and uses the
solver's own standard input language rather than inventing one. Witnesses
(satisfying assignments) come from the solver, not the model — unforgeable.

**Alternatives considered**: building constraints through the crate's typed
AST API from a structured schema — needs a nested schema (not flat) or a
fragile bespoke text format; pure-Rust solvers (`splr`/`varisat` — SAT only,
no arithmetic; LP crates — no booleans/unsat cores). External solver binary —
a system dependency, against the bundled requirement.

**Named risk (spike S1 gates this)**: `bundled` compiles Z3's C++ — a heavy
one-time build. S1 measures Windows + CI build time and validates parse →
check-sat → get-model round trip. The recorded fallback if unworkable:
arithmetic engine first, solver behind a cargo feature — a deviation to be
decided at spike time, not silently.

**FFI note**: `z3-sys` contains the unsafe FFI; this crate's
`forbid(unsafe_code)` governs our code, not dependencies (same standing as
sqlx/reqwest internals).

**Spike S1 result (2026-06-12): PASS.** Clean bundled build: **4 m 58 s** on
the Windows dev machine (acceptable; rust-cache amortizes it in CI, ubuntu
runners ship cmake). Build prerequisite finding: cmake is required and was
not on PATH — the VS 2022 Build Tools' bundled cmake works via the `CMAKE`
env var (documented for contributors in T012). Round trip validated: sat
with witness naming the variables, unsat on contradiction, the timeout
parameter bounds a hard instance (Unknown in 68 ms at a 50 ms cap), and
determinism holds. Parse-failure detection: Z3 parses the script
**atomically** — a malformed script yields 0 accepted assertions even when
earlier asserts were valid, so the assertion-count check catches full and
partial failures without unsafe FFI. The crate's `from_string` would panic
on interior NUL (`CString::new`) — the wrapper rejects NUL first.

## D2 — Arithmetic engine: `evalexpr` 13 (named refinement of the landscape's wording)

**Decision**: `evalexpr` 13 for the arithmetic family. The formal target is a
**boolean-valued expression** encoding the claim, with tolerances explicit
(e.g. `math::abs(1840 * 0.63 - 1159) <= 0.5`). The wrapper evaluates with a
length bound (≤ 2000 chars) and maps: `Boolean(true)` → supported,
`Boolean(false)` → refuted, any non-boolean result or eval error → a real
violation fed to the retry.

**Rationale**: the landscape says "a `meval`/`fend`-style expression crate";
crates.io (2026-06-12): `meval` 0.2 is unmaintained (2017) and numeric-only —
no boolean comparisons, so the claim-as-expression shape wouldn't work;
`fend-core` 1.5.8 is excellent (arbitrary precision, unit-aware) but is a
calculator — string-in/string-out without native boolean comparison results.
`evalexpr` 13 natively evaluates comparisons and boolean operators into a
typed `Value::Boolean`, which is exactly the verdict-bearing shape. Named
refinement of the landscape wording; `fend-core` stays the upgrade path when
the units/dates/conversions row is built (deferred).

**Dialect pinning (analysis A1)**: evalexpr's function names and operator
set are its own dialect (`math::abs`, not `abs`) — the translation prompt
MUST embed the allowed function/operator whitelist verbatim, or first
translations will routinely burn the single retry on syntax instead of
semantics. A unit test pins the whitelist's presence in the prompt (the same
prompt-borne-guarantee pattern as the decline-bias pin).

**Known bound (named)**: f64 arithmetic — exact integer arithmetic beyond
2^53 and arbitrary precision are not v1 claims; the explicit-tolerance
requirement in the formal target is the working mitigation, and claims
needing exact big-number arithmetic should translate to the solver (integers
in Z3 are arbitrary precision). **Hardened after acceptance run 1**: exact
`==`/`!=` over float-producing arithmetic produced confidently wrong
refutations (`0.15 * 240 == 36` is false in f64) — the engine now rejects
that shape as a retryable violation, forcing the tolerance form through the
feedback loop; pure-integer equality stays exact. The dialect whitelist is
prompt-borne + violation-enforced; evaluating against a builtins-disabled
context (making it engine-enforced) is named follow-up hardening.

## D3 — One model call: classify + translate in a single flat schema

**Decision**: one constrained call returns
`{checkable: bool, reason: string|null, engine: "arithmetic"|"constraints"|null,
arithmetic_expression: string|null, smtlib_constraints: string|null,
asserted: "satisfiable"|"unsatisfiable"|null}`. `checkable: false` carries
`reason` and ends the run as a successful not-checkable result. The prompt
carries the decline bias verbatim ("when uncertain whether a claim is
checkable, decline — a crisp answer to the wrong question is worse than no
answer") and requires tolerances to be explicit in the expression.

**Rationale**: spec assumption (fewer hops, bias lives in the prompt); the
schema is flat+closed. Cross-field consistency (engine implies its field;
asserted required for constraints) is enforced by a **pure validator** after
parsing — a violation here is a translation violation, fed to the retry like
an engine parse error.

**Implementation forcing (named)**: `engine`/`asserted` are **nullable
strings**, not enums — schemars encodes `Option<enum>` as `anyOf`, which
fails the flat+closed assertion. The allowed values live in the field
descriptions, the prompt, and the cross-field validator (an unknown value is
a retryable violation). Follow-up tightening: a `schema_with` override
emitting `{"type":["string","null"],"enum":[...]}` would push the constraint
into the provider grammar; deferred until measured prompt-compliance data
says it is needed. The validator additionally rejects scripts containing
comments or stateful commands (`;`, `push`/`pop`/`reset`/`set-option`,
`assert-soft`) so the parse-detection count can never desynchronize.

## D4 — Verdict mapping and explanation: server-assembled (004 D7, applied harder)

**Decision**: pure functions map engine output to the verdict:
arithmetic `true`/`false` → supported/refuted; solver `sat`/`unsat` crossed
with the claim's `asserted` polarity → supported/refuted (+witness on the
side that has a model — a satisfying assignment when sat supports, a
counterexample when sat refutes). `unknown`/solver-timeout → the existing
`timeout` class; the message stays honest about both causes ("solver
returned unknown (timeout or incompleteness) after N ms") since Z3 also
returns unknown for theory incompleteness, not only deadline. The **explanation is a
deterministic template** over (claim, formal form, engine result, verdict) —
no second model hop, nothing model-phrased in the verdict path.

**Rationale**: Principle V — in 004 the model still wrote the answer prose;
here even the explanation is assembled, because the explanation IS the
verdict's justification and must not be spinnable.

## D5 — Feedback loop: one retry on real violations only

**Decision**: violations that trigger the single re-translation: evalexpr
parse/eval error or non-boolean result; Z3 parse error; cross-field
validator failure (D3). The retry prompt carries the violation verbatim.
What does NOT retry: a clean engine verdict the caller dislikes (no such
signal exists), refusals (existing class), engine timeout (the formalization
was valid — retrying the same hard instance buys nothing; surfaces as
`timeout`). Second translation failure → `validation_failure` with a
translation-naming message; never a synthesized verdict.

**Rationale**: the corpus's loop re-prompts on *ground truth* signals only.
Timeout-is-terminal mirrors the 001 client policy (the budget was consumed).

## D6 — No new config, no new outcome classes, no gate

**Decision**: no new environment variables (`SOLVER_TIMEOUT_MS` = 10 000 and
`EXPRESSION_MAX_CHARS` = 2 000 are constants in `deterministic/mod.rs`);
no **Outcome class** changes (engine timeout → `timeout`, translation
failure → `validation_failure`, both with class-distinct messages); the tool
is always in the catalog (engines are pure in-process — Constitution VI
gates effects beyond the process, and there are none; the `.z3-trace`
artifact Z3's Debug build would have written is compiled out via the
`z3-sys` opt-level override, pinned by test). Records attribute to the
anthropic model (translation is the only metered call).

**Named amendment**: delivering the class-distinct timeout message required
`AppError::Timeout` to gain a message-bearing `what` field ("request" vs
"solver (timeout or incompleteness)") — a variant-shape change rippling
through the four clients. The Outcome class set and the 001
invocation-record contract are unchanged.

**Rationale**: YAGNI + taxonomy stability; the 001 invocation-record contract
needs no enum addition for this feature.

## Spike (run before anything depends on the solver)

- **S1 — z3 bundled build + round trip** (`examples/spike_z3.rs`, offline):
  add the dependency, measure clean-build wall time on Windows, parse an
  SMT-LIB 2 script, assert sat on a satisfiable system with a extracted
  model, assert unsat on a contradiction, confirm the in-engine timeout
  parameter works. CI implication measured on the first push (rust-cache
  amortizes to lockfile changes). Fallback recorded in D1 if unworkable.
