# Quickstart: Jev at turn end

## 1. Build and test

```bash
cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test --all-features --tests
```

## 2. Shadow

Set `PARALLAX_JEV_MODE_TURN_SIGNALS=shadow` with `TYPESAFE_API_KEY` present. At each turn end one `turn_signals` row is written; the checkpoint result is unchanged.

## 3. Live acceptance (SC-004)

```bash
PARALLAX_BACKEND=claude_cli DATABASE_PATH=./data/jev-acceptance.db cargo run --release --example acceptance_jev_signals
```

It exits non-zero unless every fixture turn end has one joined row and no captured request carries user text or tool output.

## 4. Report from stored records

```bash
cargo run --release --example jev_backtest -- report
```

## 5. Turn it on in the operator's setup

Add `PARALLAX_JEV_MODE_TURN_SIGNALS=shadow` to the Parallax `env` block, changing only that key, with a backup of `~/.claude.json` taken; restart Claude Code.
