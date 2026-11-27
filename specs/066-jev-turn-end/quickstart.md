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
