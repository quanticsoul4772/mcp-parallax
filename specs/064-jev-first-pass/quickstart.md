# Quickstart: verifying 064

## 1. Prerequisites

- A release build: `cargo build --release`.
- Check that the TypeSafe key is present, without printing it:

  ```bash
  if [ -n "${TYPESAFE_API_KEY:-}" ]; then echo "TYPESAFE_API_KEY: set"; else echo "TYPESAFE_API_KEY: not set"; fi
  ```

- Never use `echo $TYPESAFE_API_KEY`, `printenv`, `env`, `set -x` or `curl -v` near the key, and never pass it as a command-line argument.

## 2. Unit and integration tests

Run the full gate:

```bash
cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test --all-features --tests
```

## 3. Live shadow check (SC-004)

Run the example `examples/acceptance_jev.rs`. It reads `TYPESAFE_API_KEY` from the environment and the Exa and Voyage keys the same way the research live check does.

It sets both sites to shadow for its own run, then drives:

- real end-of-turn reviews from fixture transcripts;
- one standard research question.

```bash
PARALLAX_JEV_MODE_TURN_REVIEW=shadow PARALLAX_JEV_MODE_RESEARCH_CLAIMS=shadow \
  cargo run --release --example acceptance_jev
```

It asserts each of the following, and exits non-zero on any failure:

- at least N rows per site;
- every row joins to its checkpoint or invocation record;
- every row carries a bound outcome, and every row Jev answered carries the pinned answering model;
- no row is applied;
- an excluded claim leaves only an `excluded` row, with no request id, latency or answer;
- no redaction fixture appears in any captured request body.

It prints:

- rows per site;
- outcomes;
- the excluded share;
- Jev time on the critical path per run.

## 4. Measurements (FR-011)

Each measurement is a subcommand of the harness `examples/jev_backtest.rs`. Results go to `claudedocs/jev-064-measurements-<date>.md`. Raw inputs stay in the gitignored `data/jev/` folder.

| Measurement | Command | Reports |
|---|---|---|
| Noise | `cargo run --release --example jev_backtest -- noise` | 5 calls on each of 100 held-out items per question: within-item spread and decision flip rate |
| Self-agreement | `cargo run --release --example jev_backtest -- self-agreement` | The turn review and the verify ensemble, each re-run twice on about 100 items (runs on the subscription backend) |
| Option order | `cargo run --release --example jev_backtest -- order` | Each choice item under all 6 option orders: flip rate and McNemar's test |
| Base rate | `cargo run --release --example jev_backtest -- base-rate` | Share of turn reviews that flagged or captured, from existing `checkpoint_records` (read-only; approved first) |
| Injection | `cargo run --release --example jev_backtest -- injection` | The `directs_ai` question on `jev-1.14.0` against InjecAgent, BIPIA and at least 300 real research pages (datasets downloaded into an isolated folder) |

Re-running `cargo run --release --example jev_backtest -- report` reproduces every table from the saved inputs (SC-006).

## 5. Turning it on in your setup (FR-010)

After merge and rebuild, and with your approval of these exact changes:

1. **Back up** `~/.claude.json`.
2. **Add two keys** to the `env` block of the Parallax entry, changing only these keys:

   ```json
   "PARALLAX_JEV_MODE_TURN_REVIEW": "shadow",
   "PARALLAX_JEV_MODE_RESEARCH_CLAIMS": "shadow"
   ```

3. **Install the project's Stop hook**, which calls `checkpoint_turn` (from `integrations/claude-code/hooks.json`), in `.claude/settings.json`. The exact entry is shown for approval first.
4. **Restart the session.** The Parallax startup log shows one line per enabled site, naming `api.typesafe.ai` and the data sent.

## 6. Results

To be recorded after the live runs: the acceptance output, the measurement tables, and the base-rate figure that sizes the shadow period for 065 and 066.
