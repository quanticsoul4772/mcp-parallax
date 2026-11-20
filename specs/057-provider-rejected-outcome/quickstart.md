# Quickstart: verify the provider_rejected outcome

## Automated

```bash
cargo fmt --all -- --check && cargo clippy --all-features --all-targets -- -D warnings && cargo test --all-features --tests
```

Baseline (T001, 2026-11-20, before any code change): 691 passed + 1 ignored (lib), 32 passed (bin), 82 passed (integration).

Tests to look for:

- on both clients, a credit-balance 400, a 401, a 403, a 404 and a
  rejected-request 400 each give `provider_rejected`, with the status in the
  error text;
- an out-of-contract successful response still gives `validation_failure`;
- 429 and 5xx still retry;
- the OpenAI-compatible strategy ladder still moves on a strategy-rejecting 400;
- the code's recorded outcomes equal the record schema's list and the telemetry
  contract's list, read from the files;
- every outcome round-trips through its string form.

## Live check

Not needed: no prompt, schema or request changes; only how a provider
rejection is classified after it arrives.
