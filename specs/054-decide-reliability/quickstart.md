# Quickstart: checking this feature by hand

Nothing here calls a model.

## 1. The description

```bash
cargo test decide_description          # the unit tests on the description and field text
cargo test the_served_decide_description
```

All pass. Before the text was corrected the description test failed on the
missing sentences and the forbidden stem, and the field test on the field
description. The served-equals-registered test fails only while the two
copies differ, which the work order makes happen once on purpose.

## 2. Nothing else moved

```bash
cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test
```

The tests that pin `decide`'s confidence values (0.725 for a 45-point gap,
0.525 for a 5-point gap, 0.5 for a tie) pass with their assertions unedited.

## 3. The live text

Search the seven live-text locations listed in `data-model.md` for the stem
`calibrat`, ignoring case. Two matches, both expected: the new test's own
search string, and the `check` tool's description in `src/server.rs`.

## 4. The evidence

```bash
python -I -X utf8 claudedocs/experiments/decide-stability/table.py
```

Its output matches `table-output.txt` in the same folder and each row of the
table in `spec.md`.

## 5. The record

`docs/design/NEW_SERVER_DESIGN.md` has a block dated 2026-11 after the
2026-07-21/22 amendment. Each of the seven statements in FR-006 can be found
in it.
