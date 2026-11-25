# Contract: record additions

- `jev_records.site` gains `research_injection`; the two-way schema binding test (064) covers it through `JevSite::ALL`.
- `jev-record.schema.json` (064 contract) gains a `research_injection` branch: `label_json` is `{ "patterns063": [string] }`; `state_json` is null; `applied` is 1 only for an `on` row whose decision is `warned`.
- The invocation record's `jev_cost_usd` sums every Jev row of the run, injection rows included.
- Telemetry: the research invocation span gains `parallax.jev.injection.warned` and `parallax.jev.injection.unscreened` (integers); `specs/007-observability-layer/contracts/telemetry.md` lists them.
