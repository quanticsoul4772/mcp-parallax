# Contract: recorded outcome values after 057

The invocation record's `outcome` and the telemetry attribute `parallax.outcome`
take exactly these values:

```text
success
refusal
truncation
timeout
retries_exhausted
invalid_input
validation_failure
provider_rejected
search_provider
embedding_provider
cancelled
```

`config_error` exists in the error taxonomy but never on a record: it occurs at
startup, before any invocation exists.

`provider_rejected`: the model provider answered with an HTTP 4xx other than
429. The recorded error text is
`provider rejected the request: HTTP {status}: {provider message}`.

Bound by a test in `src/error.rs` that reads
`specs/001-core-layer/contracts/invocation-record.schema.json` and
`specs/007-observability-layer/contracts/telemetry.md`.
