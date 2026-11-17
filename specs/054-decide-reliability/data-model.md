# Data Model: Decide Stops Claiming Calibrated Confidence

No runtime data changes. The "data" of this feature is text and file lists.

## Strings

| Name | Value | Checked by |
|---|---|---|
| Forbidden stem | `calibrat`, any letter case | unit test on the registered description; one-time search over the live-text list |
| Required sentence 1 | `Confidence restates this run's top-two score gap (0.5 + gap / 200).` | unit test, exactly once |
| Required sentence 2 | `Scores come from one pass: an identical call can give different scores and a different winner (seen on one input with leads of 38 points one way and 33 the other), and the winner has changed when one sentence of context was removed and when the options were reordered.` | unit test, exactly once |
| Required sentence 3 | `Report the lead in points alongside the recommendation, and say when it is small.` | unit test, exactly once |
| Confidence field description | `This run's top-two score gap rescaled to [0.5, 1.0] (0.5 + gap / 200). An identical call can score differently.` | unit test: derived schema = this text = contract file |
| Dated line for historical records | ``> **2026-11 note:** statements here about `decide`'s confidence and about a stable margin band are revisited in specs/054-decide-reliability/spec.md.`` (a pointer only; it asserts nothing) | task-list check per file |

## Unchanged, and pinned by existing tests

| Thing | Where pinned |
|---|---|
| `confidence = 0.5 + 0.5 * min(margin, 100) / 100`, clamped to `[0.5, 1.0]` | unit tests in `src/modes/decide.rs` (dominant, near-tie, tie cases) and the integration test asserting 0.725 |
| Ranking and tie order | same unit tests |
| `DecidePass` schema and prompt template | not pinned today (the existing prompt test counts placeholders only); one new unit test holds both equal to their current text |
| Model request body | not pinned by any test; established by review of the diff |
| Result property names and numeric/enum constraints | `derived_schemas_match_the_contract_file` (it does not compare types or descriptions) |

## Live-text list (corrected in place; stem absent afterwards)

1. `src/modes/decide.rs` — module documentation, `DECIDE_DESCRIPTION`, the `confidence` field doc, two function doc comments, two comments in the test module. Expected remaining hit: the new test's own search string.
2. `src/server.rs` — the `decide` tool's description literal and the doc comment above it. Expected remaining hit: the `check` tool's description ("no calibration"), which is about another tool.
3. `specs/013-decide-methodology/contracts/decide.tool.json` — the `confidence` property's `description` only.
4. `specs/013-decide-methodology/contracts/decide.md` — the quoted description block.
5. `CLAUDE.md` — the `decide` sentence in the status paragraph.
6. `examples/acceptance_decide.rs` — the header comment.
7. `tests/integration.rs` — one test's comment and name.

Left alone because already true: `README.md`'s `decide` row ("derived from the score margin"); the contract file's opening comment.

## Historical records (one dated line each, otherwise untouched)

1. `specs/013-decide-methodology/spec.md`
2. `claudedocs/experiments/decide-order-bias/analysis.md`
3. `specs/016-push-memory/spec.md`
4. `specs/016-push-memory/research.md`
5. `specs/017-memory-consolidation/spec.md`

Plus one live code comment: `src/memory/push.rs` drops the words "the stable band".

## Evidence folder (`claudedocs/experiments/decide-stability/`)

| File | Content |
|---|---|
| `README.md` | what was measured and when; what is withheld and why; that model and effort are not recorded and routing was bypassed; why 60 first-stage runs are not in the table; how to run `table.py` |
| `results.jsonl` | one line per attempted call (460): `id` (one of `c00` to `c09`), `variant`, `perm`, and either `scores`, `methodology`, `input_tokens`, `output_tokens`, or `"error": "billing"` |
| `plan-redacted.json` | per case: `id`, `options` (count), `sentences` (count), `recorded_scores`, `recorded_gap` |
| `table.py` | standard-library Python; reads the two files above and prints each row of the specification's table |
| `table-output.txt` | the output of `table.py`, committed |
| `probe.rs.txt` | the probe's source, not compiled, header corrected |
