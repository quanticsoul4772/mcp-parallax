# Research: decide per-option fields

## Evidence

- `invocation_records`: 28 `decide` rows with outcome `validation_failure`
  between 2026-06-15 and 2026-11-17.
- Logs (Claude Desktop `mcp-server-mcp-parallax.log`, Claude Code
  `mcp-logs-parallax`): the failures read "decide produced a malformed
  assessment after the retry; last violation: assessment arity mismatch: N
  options but X scores / Y rationales", for example 3/3/4, 4/4/0, 5/10/0, 3/0/0,
  4/4/5.
- One `validation_failure` (2026-11-17 08:27 UTC) was an HTTP 400 credit-balance
  rejection, a separate classification defect, out of scope.

## D1 — Per-option fields, not parallel arrays

- **Decision**: one required integer `option_<NN>_score` and one required string
  `option_<NN>_rationale` per option position, `NN` the position as two digits from 01.
- **Rationale**: a required named field cannot be omitted or repeated, so the
  answer has exactly one score and one rationale per option or it does not
  parse. The grammar enforces required properties; it does not enforce array
  length.
- **Alternatives**: array of objects (illegal under `assert_flat`, as 013 D1
  found); a second violation-fed retry (fewer failures, not zero, another paid
  call); repairing mismatched arrays (ruled out: no fallbacks).

## D2 — One schema per count, registered at startup

- **Decision**: build the pass schema for every accepted count, 2 through 10, at
  registration; each goes through `assert_flat` and the sanitizer then. A call
  sends the schema for its option count.
- **Rationale**: keeps Constitution II's boot-time guarantee (an illegal schema
  fails startup, not a call) and keeps formats stable for the provider's grammar
  cache (nine formats at most).
- **Alternatives**: build per call (moves the gate to request time); one schema
  with 10 optional slots (optional fields bring back omission, the defect).

## D3 — Cap at 10 options

- **Decision**: reject more than 10 options before any model call with an error
  naming the limit; the tool description says "two to ten options".
- **Rationale**: decided 2026-11-20; covers counts seen in recorded
  inputs (2 to 6) with room; bounds the schema at 20 per-option fields.
- **Note**: option counts in other projects' use are not recorded, so the cap
  could reject a call that works today.

## D4 — Prompt wording follows the fields

- **Decision**: the prompt names the fields (`option_01_score`,
  `option_01_rationale`, …) and keeps the rules otherwise; the retry clause drops
  "exactly one score and one rationale per option" (now structural) and keeps
  the score-range and deciding-factors parts.
- **Rationale**: model-facing text must match the schema it sees.
- **Live check required** (project rule for model-facing changes).

## D5 — What stays

- Score range 0–100 is checked after parsing and feeds the single retry, as now.
- Empty `deciding_factors` still feeds the retry.
- `DecideResult`, ranking, confidence, `runner_up_reason` and invocation records
  are unchanged. `OptionAssessment` is assembled from the fields by position.
