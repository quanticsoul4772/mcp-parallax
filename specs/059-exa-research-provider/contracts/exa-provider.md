# Contract: research on Exa

## Operator surface

- `research` is in the catalog iff `EXA_API_KEY` is set.
- `EXA_MAX_AGE_HOURS`: integer -1..=720, default 168. Anything else is a
  startup error naming it.
- `BRAVE_API_KEY`, `FETCH_ALLOW_PRIVATE` or `FETCH_TIMEOUT_MS` set: startup
  error naming the variable. For `BRAVE_API_KEY` the message names
  `EXA_API_KEY`.

## Outbound requests

- Only to Exa (`POST /search`), one per scoped angle, with `x-api-key`.
- Parallax makes no request to any source page.
- Retried on network errors, 429 and 5xx, up to `MAX_RETRIES`, waiting
  `Retry-After` when given; other 4xx are not retried.

## Tool result

- Unchanged except: each `sources[]` entry gains `truncated: bool`.

## Invocation record and telemetry

- `invocation_records.search_cost_usd`: the run's summed Exa cost, or null
  when any Exa request reported none; null for other tools.
- The `parallax.research` span and the invocation metrics carry
  `parallax.search.cost_usd` when the record has it. `cost_usd` and
  `gen_ai.*` attributes keep their meaning (model cost and usage).
- `specs/007-observability-layer/contracts/telemetry.md` lists the new
  attribute.
