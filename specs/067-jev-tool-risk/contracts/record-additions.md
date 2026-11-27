# Contract: record additions (067)

- `jev_records.site` gains `tool_risk`; the two-way schema binding test (064) covers it through `JevSite::ALL`.
- `jev-record.schema.json` (064 contract) gains a `tool_risk` branch: `state_json` is null; `applied` is always 0; `label_json` on rows Jev answers is the big model's `action_class` answer `{ "class": ... }` with `label_source` = model id plus prompt version, and null on excluded rows (owner's answer, 2026-11-27). The contract also gains `label_cost_usd` (number or null, minimum 0).
- `checkpoint_records`: the gate's verdict, message and signals are unchanged with the site off or on (§6.3.8).
