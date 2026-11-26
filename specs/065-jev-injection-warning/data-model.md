# Data Model: Jev injection warning on research sources

## Jev site and mode (`src/traits/classifier.rs`)

- `JevSite::ResearchInjection`: suffix `RESEARCH_INJECTION`, stored as `research_injection`; added to `JevSite::ALL`, so the records-schema binding test and the config suffix set pick it up.
- `JevMode::On`: added to `JevMode::ALL`. Accepted by config only for `ResearchInjection` with a pinned threshold for the configured model.

## Injection settings (`src/config.rs`, `JevSettings`)

| Field | Source | Default | Rules |
|---|---|---|---|
| `research_injection` | `PARALLAX_JEV_MODE_RESEARCH_INJECTION` | `Off` | `off` / `shadow` / `on`; a non-off value needs `TYPESAFE_API_KEY`; `on` needs `policy::injection_threshold(model)`; otherwise a startup error naming the variable (and `PARALLAX_JEV_MODEL` for a missing threshold) |

## Pinned threshold (`src/jev/policy.rs`)

- `INJECTION_THRESHOLDS`: `[("jev-1.14.0", 0.35)]`.
- `injection_threshold(model) -> Option<f64>`.
- `injection_decision(p: Option<f64>, threshold) -> Warned | Clear | Unscreened(reason)`: a missing probability is never `Warned`.

## Source screening result (research result, per source; only with the site `on`)

| Field | Type | Meaning |
|---|---|---|
| `jev_injection.screened` | bool | Jev returned a valid answer from the pinned model in time |
| `jev_injection.probability` | number or null | P(yes) when screened |
| `jev_injection.warned` | bool | screened and probability ≥ threshold |
| `jev_injection.reason` | string or null | when not screened: `timeout`, `classifier_provider`, `provider_rejected`, `validation_failure`, `redaction_failed`, `disabled` |

## Stats additions (only with the site `on`)

| Field | Type | Meaning |
|---|---|---|
| `jev_warned_pages` | integer | kept pages with `warned = true` |
| `jev_unscreened_pages` | integer | kept pages with `screened = false` |

## Injection record (`jev_records` row, 064 table)

- `site = research_injection`, `mode` = `shadow` or `on`.
- `questions_json`: one entry, `directs_ai`, version 1, with the probability and band (`warned` / `clear` / missing).
- `model_requested`, `model_answered`, `outcome`, `request_id`, `latency_ms`, `input_tokens`, `cost_usd` as in 064.
- `invocation_id`: the research invocation; `source_ids`: `[source id]`.
- `label_json`: `{ "patterns063": ["<063 pattern name>", ...] }`, names only, possibly empty.
- `state_json`: null (page text is not stored); `redaction_count` recorded.
- `applied`: 1 only for an `on` row whose decision is `warned`; 0 otherwise.

## Invocation record

- `jev_cost_usd` includes the injection rows' cost (064 field).
