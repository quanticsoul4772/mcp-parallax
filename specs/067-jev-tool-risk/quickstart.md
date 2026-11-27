# Quickstart: Jev on tool calls (067)

## 1. Gate

```bash
cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test --all-features --tests
```

## 2. Spike (§6.3.1)

Install the widened matcher in a test session, restart, then call one non-Parallax tool and one Parallax tool. Expected: a `checkpoint_records` row (boundary `action`) for the non-Parallax tool, none for the Parallax tool, and none caused by the hook's own call.

## 3. Shadow on

Set `PARALLAX_JEV_MODE_TOOL_RISK=shadow` with `TYPESAFE_API_KEY` present, restart, make tool calls. Each Jev request or code exclusion leaves one `jev_records` row at site `tool_risk`, joined to its checkpoint record.

## 4. Live acceptance (§5.5)

```bash
cargo run --release --example acceptance_tool_risk
```

Exits non-zero unless: at least N `tool_risk` rows; every outcome in the bound set; every row joins to its checkpoint record; `model_answered` equals the pinned version; no row has `applied=true`; every redaction fixture is absent from captured request bodies; no Jev request row exists for a call excluded in code; and the hook does not trigger itself. It reports Jev time on the critical path summed per run against a stated limit.

## 5. Load (§5.5)

Run the live example with at least two sessions plus a research run at once, and record hook p99 latency.
