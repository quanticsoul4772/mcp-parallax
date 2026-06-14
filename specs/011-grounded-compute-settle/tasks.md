# Tasks: Grounded Compute-Settle

**Feature**: `011-grounded-compute-settle` | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

Tests included (Constitution IV; spec FR-008). Both stories live in
`grounded_verify`'s aggregation, so they share the foundational schema/assembler
changes; each is independently verifiable through tool output. Everything is
**offline-testable** — the value is server-counted, not model-produced, so there is no
live-model-only check (unlike 010 SC-001).

## Phase 1: Setup

No new dependencies, no new module. `arithmetic::evaluate` (005) and the 010
`grounded_verify` mode are reused in place. No setup tasks.

## Phase 2: Foundational (blocking both stories)

These two changes are prerequisites for both US1 and US2.

- [ ] T001 Add the four flat nullable fields to `GroundedPass` in `src/modes/grounded_verify.rs` — `compute_property` (**nullable `String`**, server-validated to `lines|bytes|matches`), `compute_match_literal` (nullable string), `compute_operator` (**nullable `String`**, server-validated to `>|>=|<|<=|==|!=`), `compute_threshold` (nullable integer). **Nullable strings, NOT enums** (analyze H1: `Option<enum>` emits `anyOf`, which `assert_flat` rejects at boot); the closed sets are enforced server-side, an unrecognized value → abstain. Extend the prompt: when `needs_computation` is set and the claim is a line/byte/literal-match count vs a numeric bound, fill these to name *what* to count and the comparison (never the value or verdict), **and emit `verdict: supported` with empty `findings`** so the pass isn't dropped by the 010 refute-without-findings guard (analyze M1). Confirm `assert_flat` accepts the nullable scalar fields at boot.
- [ ] T002 In `src/grounded/assemble.rs`, extend `AssembledEvidence` with `units: Vec<RawUnit { text: String, bytes: u64 }>` (the verbatim per-read-unit content, in order) and populate it in the read loop alongside `manifest`; `text`/`manifest` unchanged. This lets the count run over raw source, not the header-framed evidence, and exposes `units.len()` for the single-source gate.

## Phase 3: User Story 1 — a countable claim is settled, not bounced (P1)

**Goal**: a majority-agreed, in-class, single-source compute spec is counted over the verbatim bytes and settled by the arithmetic engine — `supported`/`refuted` with the executed form, not `inconclusive`.

**Independent test**: the `server.rs > 1000 lines` reproduction (1224-line fixture) returns `supported` with `executed_form` `1224 > 1000`; the `> 5000` variant returns `refuted`.

- [ ] T003 [US1] Add a server-internal `ComputeSpec { property: Property{Lines,Bytes,Matches(String)}, operator: Op, threshold: i64 }` with **server-side parsing/validation** of the `compute_property`/`compute_operator` strings (unrecognized → not in-class), and a pure `agreed_spec(passes) -> Option<ComputeSpec>` in `src/modes/grounded_verify.rs`: returns `Some` only when a majority of the `needs_computation` passes carry an identical, complete, in-class spec (same validated property, operator, threshold, and literal for `matches`); otherwise `None`.
- [ ] T004 [P] [US1] Add pure counting functions in `src/modes/grounded_verify.rs` over a `RawUnit`: `Lines` = count of `\n` plus one for a non-empty unterminated final line (empty → 0); `Bytes` = the unit's `bytes`; `Matches(lit)` = non-overlapping occurrences of `lit` (empty literal → out-of-class). Pin the line convention with unit tests (LF-terminated and no-trailing-newline).
- [ ] T005 [US1] Add optional `executed_form: Option<String>` and `engine_result: Option<String>` to `GroundedVerdict` (server-assembled, `skip_serializing_if = "Option::is_none"`) in `src/modes/grounded_verify.rs`; `verify` and the per-pass schema untouched.
- [ ] T006 [US1] In grounded aggregation (`src/modes/grounded_verify.rs`), when a `needs_computation` majority holds AND `units.len() == 1` AND `agreed_spec` is `Some` AND the claim is **purely computable** (the agreeing passes carry no substantive judgment `findings` — the compound-claim gate, analyze M2): count the property over the single raw unit, build `format!("{value} {op} {threshold}")`, call `deterministic::arithmetic::evaluate`, and return `supported`/`refuted` from `holds` with `executed_form` + `engine_result` and a one-line `findings` note ("counted N lines"), `confidence` 1.0. (The else branch — including compound claims — is US2.)
- [ ] T007 [P] [US1] Unit tests in `src/modes/grounded_verify.rs`: agreed `lines > 1000` over a 1224-line raw unit → `supported`, `executed_form` `1224 > 1000`, `engine_result` `true`; `lines > 5000` → `refuted`; a `bytes` and a `matches` spec each settle; the count conventions from T004 hold; **a computable pass (verdict supported, empty findings, compute fields set) is accepted by `one_pass`, not dropped** (analyze M1).

**Checkpoint**: US1 settles the in-class single-source case end-to-end, offline-tested.

## Phase 4: User Story 2 — anything not cheaply computable still abstains (P1)

**Goal**: a `needs_computation` majority that is not an agreed in-class single-source spec falls back to 010's `inconclusive` (route to `check`) — no computed verdict over an underived value; the judgment path is unchanged.

**Independent test**: an out-of-class or multi-source computable-flagged claim returns `inconclusive`; a non-computable claim returns `supported`/`refuted` from the passes.

- [ ] T008 [US2] Complete the aggregation fallthrough in `src/modes/grounded_verify.rs`: when a `needs_computation` majority holds but `agreed_spec` is `None` (disagreement, unrecognized property/operator string, missing field) OR `units.len() != 1` (multi-source) OR the claim is **compound** (a substantive judgment finding among the agreeing passes, analyze M2) OR `arithmetic::evaluate` errors → return 010's `inconclusive` (route-to-`check` reason); never a verdict over an underived/unsettled value or a compound claim's count alone. The non-`needs_computation` judgment path stays exactly 010 (FR-006).
- [ ] T009 [P] [US2] Unit tests in `src/modes/grounded_verify.rs`: passes disagree on the spec → `inconclusive`; an unrecognized/out-of-class property string while flagged → `inconclusive`; `units.len() == 2` with an otherwise-valid spec → `inconclusive`; **a compound claim (valid spec + a substantive judgment finding) → `inconclusive`, not settled on the count** (analyze M2); a non-`needs_computation` majority → `supported`/`refuted` unchanged (no compute attempted, no `executed_form`).

**Checkpoint**: both stories complete and offline-tested; 010's no-confidently-wrong guarantee preserved.

## Phase 5: Polish & Cross-Cutting Concerns

- [ ] T010 [P] Integration tests in `tests/integration.rs` (011 block): a real 1224-line temp file with mock passes returning `needs_computation=true` + `compute_property=lines`, `compute_operator=>`, `compute_threshold=1000` → tool returns `supported` with `executed_form` `1224 > 1000`; `threshold=5000` → `refuted`; a multi-locator computable-flagged call → `inconclusive`; a non-computable claim → `supported`/`refuted` unchanged.
- [ ] T011 [P] Extend `examples/acceptance_grounded_verify.rs` with the compute-settle reproduction (server.rs line-count → `supported` with executed form), and update docs: the `grounded_verify` README Tools row and `CLAUDE.md` note that a computable claim is now *settled* (not just abstained) for the line/byte/match class.
- [ ] T012 Full gate: `cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`; record results in `quickstart.md` and check off this file.

## Dependencies & Execution Order

- **Foundational (T001–T002)** blocks everything (the pass fields and the raw units).
- **US1 (T003–T007)** then **US2 (T008–T009)**: US2's fallthrough is the else of US1's
  settle branch (T006), so US1's aggregation lands first; US2 completes and tests the
  fallback. Both are in the same file → sequential, not parallel, across T006/T008.
- **Polish (T010–T012)** after both stories.

## Parallel Execution Examples

- After T001/T002: T004 (counting fns) and T003 (agreed_spec) can proceed together; T007
  unit tests parallel with T009 once T006/T008 land.
- Polish: T010 and T011 parallel; T012 last.

## Implementation Strategy

US1 is the value (settling the claim); US2 is its safety boundary (never settling outside
the agreed in-class single-source spec). They ship together — US1 without US2 could
overreach, US2 without US1 is just 010. Everything is offline/deterministic: the
reproduction is a fixed-size temp file, the passes are mocked, and the value is
server-counted — **no live dogfood is required** (contrast 010 T013).
