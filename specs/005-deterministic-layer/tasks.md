---

description: "Task list for Deterministic Layer — Checkable Claims Settled by Execution"
---

# Tasks: Deterministic Layer — Checkable Claims Settled by Execution

**Input**: Design documents from `/specs/005-deterministic-layer/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: REQUIRED (Constitution Principle IV) — MockModelClient for the
translation hop; the engines are deterministic and are tested DIRECTLY
against ground-truth tables (mocking a solver tests nothing); in-process
rmcp for integration. The acceptance example is manual-run live spend
(translation quality is the only live question).

## Format: `[ID] [P?] [Story?] Description`

## Phase 1: Setup

- [ ] T001 Spike S1 in examples/spike_z3.rs (offline — GATES EVERYTHING, run first): add `z3` 0.20 with the `bundled` feature and `evalexpr` 13 to Cargo.toml; measure the clean-build wall time on Windows; parse an SMT-LIB 2 script via the crate, assert sat on a satisfiable system WITH an extracted model (witness), assert unsat on a contradiction, and confirm the in-engine timeout parameter takes effect; record the build-time measurement in specs/005-deterministic-layer/research.md D1. If the bundled build proves unworkable, STOP and decide the recorded fallback (arithmetic first, solver behind a cargo feature) before any later task

## Phase 2: Foundational

- [ ] T002 [P] Module scaffold + wire types: src/deterministic/mod.rs (constants per data-model.md §5: SOLVER_TIMEOUT_MS=10_000, EXPRESSION_MAX_CHARS=2_000, SMTLIB_MAX_CHARS=10_000, TRANSLATION_ATTEMPTS_MAX=2; Verdict/Engine enums) and src/deterministic/contract.rs (CheckParams/CheckResult matching contracts/check.tool.json, schemars derives, field-consistency invariant documented); register `pub mod deterministic;` in src/lib.rs; contract-sync test both directions (003/004 pattern)
- [ ] T003 [P] Translation mode in src/deterministic/translate.rs: the classify+translate prompt (decline bias verbatim — "when uncertain whether a claim is checkable, decline"; tolerances must be explicit; engine choice part of translation; the evalexpr function/operator whitelist embedded verbatim — analysis A1: the dialect is evalexpr's own, e.g. math::abs not abs, and syntax mistakes must not burn the semantic retry), the flat+closed hop schema per data-model.md §3 ({checkable, reason?, engine?, arithmetic_expression?, smtlib_constraints?, asserted?}), ModeRegistry registration (boot flat assertion), the pure cross-field validator (engine implies its field; asserted required for constraints; length bounds re-imposed locally), and the violation-fed single-retry loop (research.md D5: retries only on REAL violations passed verbatim; second failure → ValidationFailure naming translation); unit tests with MockModelClient: clean translation, decline shape, cross-field violation triggers retry, double failure is validation_failure with no verdict, and the prompt-content pins (decline-bias sentence AND the evalexpr whitelist both present verbatim)
- [ ] T004 [P] Arithmetic engine in src/deterministic/arithmetic.rs: evalexpr wrapper — length bound first, evaluate, map Boolean(true/false) through, non-boolean result and parse/eval errors → typed Violation (retryable); ground-truth table tests: true/false comparisons, explicit tolerance forms (a "roughly X" claim translated with its bound made explicit), division-by-zero → Violation, non-boolean expression → Violation, oversized expression rejected pre-eval, identical expression twice → identical result (SC-007)
- [ ] T005 Solver engine in src/deterministic/solver.rs: z3 wrapper — SMT-LIB 2 script in (declares + asserts; engine appends check-sat), SOLVER_TIMEOUT_MS applied in-engine, parse error → typed Violation, outcomes Sat(witness extracted from the model as a readable assignment) | Unsat | Unknown(→ the existing timeout class at the orchestration layer); ground-truth table tests: satisfiable system yields sat + witness naming each variable, contradiction yields unsat, malformed script → Violation, timeout parameter honored on a hard instance or by construction, identical script twice → identical outcome (depends on T001)

## Phase 3: US1 — settle a checkable claim by execution (P1) 🎯 MVP

- [ ] T006 [US1] Orchestration in src/deterministic/check.rs: validate input (FR-008, naming INPUT_MAX_CHARS) → translate (T003 loop) → execute on the chosen engine → pure verdict mapping per data-model.md §4 (sat/unsat crossed with asserted polarity; witness on whichever side holds the model; arithmetic bool direct) → deterministic explanation template over (claim, formal form, engine result, verdict) — the model NEVER phrases the verdict path (research.md D4); returns (CheckResult, in_tokens, out_tokens); unit tests through MockModelClient + real engines: true/false arithmetic verdicts match ground truth, sat-asserted-impossible refutes WITH witness, unsat-asserted-impossible supports, every response carries formal_form + engine_result (SC-003), field-consistency invariant holds on every path, solver Unknown surfaces as the timeout class with a solver-naming message
- [ ] T007 [US1] Server wiring in src/server.rs: `check` #[tool] via run_recorded (model = anthropic model — translation is the only metered call), description verbatim from contracts/check.tool.json, deterministic deps composed unconditionally (no gate — FR-010), get_info instructions mention `check`; unit tests: `check` present in the catalog with NO capability keys configured, one success record and one failure record with correct attribution (depends on T006)
- [ ] T008 [US1] Integration round trip in tests/integration.rs: catalog lists `check` with the contracted description/schemas even in the no-keys serve() (SC-005); full call through the real rmcp client with wiremock /v1/messages returning a scripted translation and the REAL engines executing — structured result validates against contracts/check.tool.json, verdict matches ground truth for one arithmetic and one constraints claim (witness present on the refuted-impossibility case), exactly one success record (SC-006); update the stdio smoke test's expected no-keys catalog to ["check","unstick","verify"] (depends on T007)

## Phase 4: US2 — honest refusal on uncheckable claims (P2)

- [ ] T009 [US2] Not-checkable path in src/deterministic/check.rs + tests/integration.rs: checkable=false short-circuits before any engine (unit: MockModelClient declining → verdict not_checkable, reason present, engine/formal_form/engine_result/witness all null, success outcome — not an error class); integration: a declined claim returns the not-checkable shape with one SUCCESS record; assert the translation prompt carries the decline-bias sentence verbatim (the classifier bias is prompt-borne and must not silently vanish in a prompt edit)

## Phase 5: US3 — symbolic feedback loop + translation defenses (P3)

- [ ] T010 [US3] Feedback-loop guarantees in src/deterministic/ tests + tests/integration.rs: unit — scripted first translation with a malformed expression triggers exactly one retry whose prompt contains the engine violation verbatim, valid second form proceeds to a verdict with translation_attempts=2 (SC-004); scripted double failure → validation_failure naming translation, NO verdict synthesized; integration — induced malformed-then-valid translation sequence through wiremock recovers end to end; every successful response in every prior test asserted to carry formal_form + engine_result (SC-003 sweep)

## Phase 6: Polish

- [ ] T011 [P] Acceptance example examples/acceptance_check.rs (live: ANTHROPIC_API_KEY only): ≥20 ground-truth claims spanning both engines (SC-001 100% verdict accuracy), ≥6 clearly uncheckable claims incl. one too-vague-to-bound numeric claim (SC-002 100% declined), auditability sweep (SC-003), one repeated check asserting identical engine result (SC-007); record results in specs/005-deterministic-layer/quickstart.md
- [ ] T012 [P] Docs: README.md + CLAUDE.md status (deterministic layer served; `check` always on — first ungated addition since core, rationale: pure in-process engines), repo layout gains deterministic/; note z3 bundled build cost for contributors (first clean build is slow, rust-cache amortizes in CI)
- [ ] T013 Full gate (`cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`) + code-reviewer and design-reviewer agent passes over the branch diff + apply findings

## Dependencies

T001 gates everything (the recorded fallback decision happens there if the
bundled build fails). T002/T003/T004 parallel after T001; T005 needs T001's
validated z3 patterns. T003+T004+T005 → T006 → T007 → T008; T006 → T009;
T003/T006 → T010. T011/T012 after stories; T013 last.

## Parallel opportunities

- After T001: T002 ∥ T003 ∥ T004 (T005 follows T001 directly too — its only
  dependency is the spike's validated API usage).
- After T007: T008 ∥ T009 prep; T010 after T008/T009 land.
- Polish: T011 ∥ T012.

## Implementation strategy

MVP = T001–T008: a working always-on `check` tool with both engines,
engine-decided verdicts, and full record parity. US2 adds the honest-decline
guarantee, US3 pins the feedback-loop and auditability properties. Only T011
spends live money (manual-run; translation is the only nondeterministic
piece — the engines need no live validation). T001 is the schedule risk:
the z3 bundled build decision happens there, first, with the fallback
recorded in research.md D1.
