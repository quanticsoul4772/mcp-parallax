# Tasks: Checkpoint Layer — Harness-Triggered Correctives

**Input**: Design documents from `specs/006-checkpoint-layer/`
**Prerequisites**: plan.md, research.md (D1–D9, S1–S2), data-model.md, contracts/ (3 tools), quickstart.md

**Tests**: REQUIRED (constitution IV) — written through the trait seams, included in each task below. Detectors are tested against ground-truth trajectory tables, never mocked.

**Organization**: by user story. US1 (post-batch loop/failure flags) is the MVP and proves the whole sensor→brain→feedback chain deterministically; US2 adds the memory-paired gate; US3 adds the end-of-turn review with the layer's only model hop.

## Phase 1: Setup

- [X] T001 Create the module skeleton: `src/checkpoint/mod.rs` with all constants from data-model.md §1 (`WINDOW_ENTRIES`, `WINDOW_BYTES`, `WINDOW_BATCHES`, `REPEAT_THRESHOLD`, `FAILURE_THRESHOLD`, `GATE_BUDGET_MS`, `GATE_RELEVANCE_TAU` placeholder pending S2, `REVIEW_CANDIDATES_MAX`, `COOLDOWN_WINDOW_MS`, built-in `GATE_RISK_PATTERNS`) plus `Boundary`/`Verdict`/`SignalKind`/`Signal` (serde snake_case; `signal_key` = kind + stable hash of normalized evidence); declare submodules; wire `pub mod checkpoint;` into `src/lib.rs`; create `integrations/claude-code/` directory. Unit tests pin serde casing and `signal_key` stability.

## Phase 2: Foundational (blocking prerequisites for all stories)

- [X] T002 [P] `TrajectoryReader` seam: `src/traits/trajectory.rs` — `read(path, session_id) -> Result<TrajectoryWindow, AppError>` + `FsTrajectoryReader` enforcing data-model.md §5 (canonicalize, `.jsonl` extension, regular file, session-id match, tail window capped at `WINDOW_BYTES`/`WINDOW_ENTRIES`; violations are `ValidationFailure`, never partial reads). Register in `src/traits/mod.rs`. Tests: tempfile fixtures for the happy path + every rejection (wrong extension, missing file, session mismatch, oversize truncation-to-window).
- [X] T003 [P] Trajectory window model: `src/checkpoint/trajectory.rs` — `TrajectoryWindow`/`TrajectoryEntry` (data-model.md §2), harness-JSONL parsing into entries (tool calls with `batch_index`, `failed`; assistant messages), and `normalize_input` (whitespace collapse, volatile-field dropping: ids, timestamps, absolute temp paths). Tests pin the normalization rules with a ground-truth table (equal/unequal pairs) — the precision lever (D5).
- [X] T004 [P] Storage: `checkpoint_records` table per data-model.md §6 — migration, `Storage` trait methods (insert; cooldown lookup by `session_id` + `signal_key` within window via `TimeProvider`; rate aggregates: flag/hold/suppression/fail-open counts per boundary), SQLite + mock implementations. Tests through the seam: insert/lookup round trip, cooldown window edge (inside vs outside), aggregates.
- [X] T005 [P] Wire contracts: `src/checkpoint/contract.rs` — `CheckpointActionParams`/`CheckpointBatchParams`/`CheckpointTurnParams` and shared `CheckpointResult` exactly per `contracts/*.tool.json` (per-boundary verdict subsets enforced at construction). Contract-sync tests assert params/result schemas and tool descriptions match the three contract JSONs verbatim (the 005 pattern).
- [ ] T006 S2 spike (gates D4/SC-003): `examples/spike_embed_latency.rs` — 50 sequential Voyage query embeds, report p50/p95; record the measurement and the resulting decision (τ value; semantic-in-gate vs lexical fallback vs SC-003 amendment) in research.md D4 and data-model.md §1. Requires `VOYAGE_API_KEY`; run once and commit findings.

**Checkpoint**: seams, storage, contracts exist — user stories can start.

## Phase 3: User Story 1 — Catch the loop the model can't see (P1) 🎯 MVP

**Goal**: post-batch checkpoint flags loops/repeated failures from purely deterministic screening; silence on benign windows; one record per evaluation.

**Independent test**: replay a seeded loop trajectory and benign trajectories through `checkpoint_batch` (in-process, mock reader) — flag with specific evidence vs silence (spec US1 acceptance scenarios).

- [X] T007 [P] [US1] Detectors: `src/checkpoint/screen.rs` — `repetition(window)` (≥ `REPEAT_THRESHOLD` same normalized action within `WINDOW_BATCHES`) and `repeated_failure(window)` (≥ `FAILURE_THRESHOLD` consecutive failures of same action), both pure, returning `Signal`s with evidence strings naming action + count. Ground-truth table tests: US1-AS1 (4× near-identical → fires), US1-AS2 (3× consecutive failures → fires), benign windows (varied tools, 3× non-consecutive failures, 3× repetition) → no signal; determinism (same window twice → identical signals).
- [X] T008 [US1] Batch orchestration: `src/checkpoint/run.rs` — `run_batch(deps, params)`: validate → `TrajectoryReader` → screen → cooldown filter (`Storage` lookup per `signal_key`, suppressed ⇒ silence + `suppressed: true`) → assemble message from fixed templates parameterized only by evidence (FR-005/SC-007) → write checkpoint record. Fail-open wrapper: any error/timeout ⇒ `silence` + `fail_open: true` + record (FR-008). Tests via mocks: flag round trip, cooldown suppression, fail-open on reader error, exactly-one-record per evaluation.
- [X] T009 [US1] Server registration: `checkpoint_batch` `#[tool]` in `src/server.rs` via `run_recorded` (tool id `checkpoint_batch`), description verbatim from `contracts/checkpoint_batch.tool.json`, `CheckpointDeps` wired (reader, storage, clock). Integration tests in `tests/integration.rs`: catalog presence (all gating combinations), seeded-transcript flag round trip, benign silence, fail-open parity, invocation + checkpoint records both written.
- [ ] T010 [US1] S1 spike (gates the sensor plane; live Claude Code): scratch hook config wiring `PostToolBatch` (and a `Stop` probe) to `checkpoint_batch` via the `mcp_tool` handler against the dev server. Verify and record in `examples/spike_hooks.md`: payload shape delivered as tool input; tool-result → hook-control mapping (`decision:"block"`); self-trigger exemption (hook-invoked Parallax calls must not re-fire hooks — spec edge case 1); `PostToolBatch` availability; `Stop.last_assistant_message` + continuation-indicator field names (if `Stop` lacks the final-message field, record the fallback: `checkpoint_turn` reads the final assistant message from the transcript tail and `final_message` becomes optional in the contract). On failure: switch to D1's named fallback (`command` handler + one-shot CLI mode) and record the deviation in research.md D1 before T011.
- [ ] T011 [US1] Sensor plane v1: `integrations/claude-code/hooks.json` (batch entry per S1 findings) + `integrations/claude-code/README.md` (install/merge instructions, uninstall-restores-prior-state, fail-open behavior, what each hook does). Manual end-to-end protocol in a live session, results recorded in the README or quickstart: induced loop → flag visible to the model; benign work → nothing; **SC-004 live check** — kill the server mid-session with hooks installed → session proceeds, no blocked actions; **SC-006 inertness check** — uninstall the hooks, run a benign session → zero new `checkpoint_records` rows.

**Checkpoint**: US1 fully functional — the MVP loop closes end to end.

## Phase 4: User Story 2 — Hold a risky action that contradicts the record (P2)

**Goal**: pre-action gate holds risk-matched actions conflicting with verified stored constraints, deterministically, within the hard budget; everything else passes free.

**Independent test**: seeded constraint memory + contradicting risk-matched action → hold quoting the memory; benign action → silence with no evaluation; induced timeout → fail-open.

- [X] T012 [P] [US2] Gate logic: `src/checkpoint/gate.rs` — `risk_matched(tool_name, tool_input)` against built-in `GATE_RISK_PATTERNS` + `CHECKPOINT_GATE_PATTERNS` env override (parse in `src/config.rs`: comma-separated substrings; present-but-unparseable is an error per config convention), and `constraint_hold(action_text, memories)` — query embed via `Embedder`, existing pure cosine ranking, hold iff top constraint-kind memory ≥ `GATE_RELEVANCE_TAU` (τ from S2). Pure decision functions. Tests: pattern matching table (defaults + env override + non-risk pass), mock-embedder relevance threshold edges (≥ τ holds, < τ silent, non-constraint kind ignored, empty store silent).
- [X] T013 [US2] Action orchestration: `run_action(deps, params)` in `src/checkpoint/run.rs` — non-risk-matched ⇒ immediate silence with no evaluation recorded as such (FR-013); risk-matched ⇒ gate under `GATE_BUDGET_MS` hard timeout (`tokio::time::timeout`) ⇒ hold message quotes the memory verbatim (US2-AS1) or silence; timeout/error ⇒ fail-open + record (US2-AS3). Tests: hold round trip, pass-through, budget timeout via a deliberately slow mock embedder, record content.
- [X] T014 [US2] Server registration + sensor update: `checkpoint_action` `#[tool]` in `src/server.rs` (description verbatim from contract JSON; gate works without `VOYAGE_API_KEY` by degrading to silence — memory-paired signal inactive, recorded); integration tests (hold with mocked embedder wiring, no-friction pass, fail-open parity, records). Add the `PreToolUse` entry (matcher narrowed to `Bash`/`Write`/`Edit` per D2) to `integrations/claude-code/hooks.json` + README; map `hold` → `permissionDecision:"ask"` per S1 findings.

**Checkpoint**: US2 independently deliverable on top of foundational.

## Phase 5: User Story 3 — End-of-turn review against the trajectory (P3)

**Goal**: deterministic candidate mining gates at most one blind flat-schema model hop; confirmed contradiction ⇒ forced-continuation flag citing both statements; zero candidates ⇒ no hop, silence.

**Independent test**: seeded contradiction session → flag citing both statements; benign session → silence with `review_ran = 0`; screening-fired-but-cleared → silence with `review_ran = 1`.

- [X] T015 [P] [US3] Candidate mining: `src/checkpoint/review.rs` — `mine_candidates(window, final_message, recall_hits)`: (a) memory recall hits of decision/constraint kind above the relevance floor; (b) transcript pairs — earlier assistant-message sentences vs final-message sentences with high lexical overlap + opposing polarity cues; capped at `REVIEW_CANDIDATES_MAX`; candidates carry verbatim statements stripped of surrounding justification (FR-012) plus a compact summary of tool outcomes observed between the two statements (FR-004(d) intervening-evidence input). Pure. Ground-truth table tests: seeded reversal pair found, paraphrase-without-negation not a candidate, cap respected, empty inputs ⇒ empty, between-statements summary populated.
- [X] T016 [US3] Review hop + assembly: register `checkpoint_review` mode (flat+closed schema `{contradicts, statement_a, statement_b, basis}` per data-model.md §4; decline-biased blind prompt — explicit and material contradictions only, and a reversal justified by intervening evidence is NOT a contradiction, applied via the candidates' between-statements summaries per FR-004(d)) and pure verdict assembly (hop output → flag message citing both statements, or silence; `review_ran` recorded). `MockModelClient` tests: contradicts ⇒ flag with both statements in message; cleared ⇒ silence + `review_ran`; zero candidates ⇒ hop never invoked (US3-AS2); schema flatness asserted.
- [X] T017 [US3] Turn orchestration + registration: `run_turn(deps, params)` in `src/checkpoint/run.rs` — `continuation: true` ⇒ screening-only (FR-014, no second forced continuation); cooldown via `signal_key`; fail-open wrapper. `checkpoint_turn` `#[tool]` in `src/server.rs` (description verbatim from contract JSON). Integration tests: contradiction round trip, benign silence with no hop, continuation guard, records. Add the `Stop` entry (flag → `decision:"block"` forced continuation; pass `continuation` from the Stop payload's indicator per S1) to `integrations/claude-code/hooks.json` + README.

**Checkpoint**: all three boundaries live; sensor plane complete.

## Phase 6: Polish & Cross-Cutting

- [ ] T018 [P] Acceptance: `examples/acceptance_checkpoint.rs` — assemble the corpora (≥20 benign trajectories from this repo's recorded sessions + synthetic; ≥12 seeded covering all four v1 signals, committed as test assets per spec Assumptions) and replay through all three boundaries in-process. Assert SC-001 (≥95% silence, zero holds on benign), SC-002 (≥80% catch at first observable checkpoint; 100% of seeded memory-contradicting actions held), SC-003 (100% within budget; p95 per D4's recorded decision), SC-004 (replay a corpus slice with deps made unavailable mid-run — every evaluation returns silence + `fail_open`, none errors outward), SC-005 (one record per evaluation; rates computable by SQL alone), SC-007 (every flag message contains its specific evidence); seeded corpus includes one evidence-justified reversal that MUST stay silent (FR-004(d) negative case). SC-004/SC-006 session-level halves are T011's live protocol. Record results (all runs, honestly) in `specs/006-checkpoint-layer/quickstart.md`; tune `GATE_RELEVANCE_TAU`/thresholds only from this evidence, updating data-model.md §1.
- [ ] T019 [P] Docs + corpus sync: update `CLAUDE.md` and `README.md` status (checkpoint layer, sensor-plane install pointer); verify `docs/design/WATCHDOG_LAYER.md` amendment's deliverable-shape claims match what shipped and amend in the same change if measurement disagreed (constitution I); note S1/S2 findings in `docs/design/SDK_LANDSCAPE.md` §watchdog if they changed the stack picture.
- [ ] T020 Full gate (`cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test`) + code-reviewer and design-reviewer agent passes over the branch diff + apply findings.

## Dependencies & Execution Order

- Phase 1 → Phase 2 → stories. T002/T003/T004/T005 are [P] (different files); T006 (S2) anytime after setup, **before T012**.
- US1: T007 needs T003; T008 needs T002+T003+T004 (and T007); T009 needs T005+T008; T010 (S1) needs T009 (a free, pure tool to probe); T011 needs T010.
- US2: T012 needs T006 (τ) + T003; T013 needs T012 (and shares `run.rs` with T008 — sequential); T014 needs T013 + T010 (hook-output mapping).
- US3: T015 needs T003; T016 needs T015; T017 needs T016 (shares `run.rs` — sequential after T013) + T010.
- Polish: T018 ∥ T019 after all stories; T020 last.
- `run.rs` and `server.rs` and `hooks.json` are shared files — tasks touching them (T008→T013→T017; T009→T014→T017; T011→T014→T017) are sequential by design.

## Implementation Strategy

US1 alone is a shippable MVP: it proves payload delivery, trajectory reading, deterministic screening, cooldown, records, and model-visible feedback — with zero model-hop cost and zero credentials beyond what the server already has. US2 adds the gate (after S2 fixes τ and the latency story), US3 adds the only model hop. The two spikes are scheduled exactly before the work they gate: S2 (T006) before gate logic, S1 (T010) after the first free tool exists and before any hook config is written. Precision (SC-001) is tuned exclusively from T018's recorded evidence — thresholds never move on intuition.
