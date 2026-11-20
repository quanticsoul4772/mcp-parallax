# Quickstart: verify decide per-option fields

## Automated

```bash
cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test --all-features --tests
```

Baseline (T001, 2026-11-20, before any decide change): 677 passed + 1 ignored
(lib), 30 passed (bin), 82 passed (integration). The bin count includes three
new unit tests for the CLAUDE.md active-feature checker, which now checks each
"N tasks closed" claim against the link in its own paragraph; it had paired
040's claim with every link and failed once 056's tasks.md existed.

Tests to look for:

- every count 2..=10 has a registered format that passes `assert_flat`, with
  exactly one score and one rationale field per position and no score or
  rationale arrays;
- a call sends the format for its own option count;
- 11 options is rejected before any model call, naming the limit; 10 is accepted;
- identical scores give an identical `DecideResult` to the pre-change version;
- a score outside 0–100 and empty `deciding_factors` still feed the single retry.

## Live check (after rebuilding the release binary)

Call `decide` through the running server with 2, 4 and 6 options. Each result
must list one assessment per option in input order. Record the counts, results
and cost here when run.

Run 2026-11-20 on the rebuilt release binary (built 10:59:13; all four server
processes started after it), through `decide` on claude-sonnet-5, one request
per call, no retry:

- 2 options: 2 assessments in input order; SQLite 94, PostgreSQL 22;
  recommended SQLite.
- 4 options: 4 in input order; every merge 45, weekly 78, monthly 72,
  milestone 58; recommended weekly (lead 6).
- 6 options: 6 in input order; email 40, chat bot 68, dashboard 88, shared
  drive 45, issue tracker 60, stand-up 25; recommended dashboard (lead 20).

Each rationale described its own option, so no answer was shifted by position.
Tokens and cost were not read: the servers' invocation database is set by a
`DATABASE_PATH` this check could not locate.
