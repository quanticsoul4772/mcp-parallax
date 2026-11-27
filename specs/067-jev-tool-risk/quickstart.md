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

Exits non-zero unless: at least 25 `tool_risk` rows; every outcome in the bound set (the existing outcomes plus `excluded`); every row joins to its checkpoint record; `model_answered` equals the pinned version on every row that made a Jev request; no row has `applied=true`; every redaction fixture is absent from captured request bodies; no Jev request row exists for a call excluded in code; and the hook does not trigger itself. It also fails if Jev adds any time to the critical path (SC-004).

## 5. Load (§5.5)

Run the live example with at least two sessions plus a research run at once, and record hook p99 latency.

Result (2026-11-27, `PARALLAX_BACKEND=claude_cli`): two tool-risk runs started at once during `acceptance_jev`'s research question; both passed SC-002 and SC-004. Gate response over 56 calls: p50 4 ms, p99 75 ms, max 75 ms.
