# Data Model: Provider rejections recorded as their own outcome

In-memory and record-level only; no storage migration.

## AppError::ProviderRejected (new)

| Field | Type | Notes |
|---|---|---|
| status | u16 | the provider's HTTP status (4xx, never 429) |
| detail | String | the provider's response body; on the Anthropic client, followed by the effort-remedy hint when it applies |

- Display: `provider rejected the request: HTTP {status}: {detail}`.
- Maps to `Outcome::ProviderRejected`.
- Terminal: not retried (as the `Client` error it replaces was not).
- Not metered: the provider does not bill a rejected request.

## Outcome (one value added)

| Value | Recorded | Meaning |
|---|---|---|
| `provider_rejected` | yes | the model provider refused the request with an HTTP 4xx other than 429 |

All other values keep their meaning. `validation_failure` now means only a
failed contract check on a response the provider returned: local schema
validation, or an out-of-contract successful response.

## What stays the same

- `AppError::Client` for transport failures and out-of-contract successful
  responses, and its `validation_failure` outcome.
- HTTP 429 and 5xx: retried; exhausted retries are `retries_exhausted`.
- The OpenAI-compatible strategy ladder: a 400 rejecting the strategy moves to
  the next rung.
- `embedding_provider` and `search_provider` for the Voyage and Brave clients.
- The invocation record's shape.
