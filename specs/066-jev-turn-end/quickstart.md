# Quickstart: Jev at turn end

## 1. Build and test

```bash
cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test --all-features --tests
```

## 2. Turn signals

Set `CHECKPOINT_TURN_SIGNALS=on`. At each turn end one `turn_signal_records` row is written; the checkpoint result is unchanged and nothing new is sent.

## 3. Live acceptance (SC-004)

```bash
cargo run --release --example acceptance_turn_signals
```

It builds its own checkpoint dependencies with the signals on and uses a fresh temporary database, so it reads neither `CHECKPOINT_TURN_SIGNALS` nor `DATABASE_PATH`. It exits non-zero unless every fixture turn end has one joined row with the expected detector results and no sentinel string reaches a request body.

## 4. Report from stored records

```bash
cargo run --release --example jev_backtest -- report
```

The report includes the turn-signal fire rates and exclusion reasons.

## 5. Live check before the PR (T032a, 2026-11-26)

On `claude_cli`, with `PARALLAX_JEV_MODE_TURN_REVIEW=shadow` and `PARALLAX_JEV_MODE_RESEARCH_CLAIMS=shadow` set for the run, `examples/acceptance_jev.rs` passed with the version-2 turn-review questions: 6 of 6 reversal turns flagged; 6 turn_review Jev rows, all successful (max latency 183 ms); research 161 findings and 168 claims verified, 131 Jev successes and 37 excluded (share 0.22); SC-004 passed (137 states sent, none with the planted secret). An earlier attempt on the anthropic backend failed on API credit and is not counted.
