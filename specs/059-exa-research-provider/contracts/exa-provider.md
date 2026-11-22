# Contract: research on Exa

## Operator surface

- `research` is in the catalog iff `EXA_API_KEY` is set.
- `EXA_MAX_AGE_HOURS`: integer -1..=720, default 168. Anything else is a
  startup error naming it.
- `FETCH_ALLOW_PRIVATE` or `FETCH_TIMEOUT_MS` set: startup error naming the
  variable.
- `BRAVE_API_KEY` set without `EXA_API_KEY`: startup error naming both; with
  `EXA_API_KEY` set, it is ignored.

## Outbound requests

- Only to Exa, with `x-api-key`: one `POST /search` per scoped angle (no
  text requested), then one `POST /contents` per run for the kept sources.
- Parallax makes no request to any source page.
- Retried on network errors, 429 and 5xx, up to `MAX_RETRIES`, waiting
  `Retry-After` when given; other 4xx are not retried.

## Tool result

- Unchanged except: each `sources[]` entry gains `truncated: bool`.

## Invocation record and telemetry

- `invocation_records.search_cost_usd`: the run's summed Exa cost, or null
  when any Exa request reported none; null for other tools.
- The `parallax.research` span carries `parallax.search.cost_usd` when the
  record has it. Metrics are unchanged: a dollar value is not used as a
  metric attribute. `cost_usd` and
  `gen_ai.*` attributes keep their meaning (model cost and usage).
- `specs/007-observability-layer/contracts/telemetry.md` lists the new
  attribute.
