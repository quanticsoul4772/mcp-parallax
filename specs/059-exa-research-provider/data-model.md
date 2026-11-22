# Data Model: Exa replaces Brave for research

## Search request (per angle)

| Field | Source |
|---|---|
| query | the scoped angle |
| count | `per_angle_count` (still capped at 20 per request) |
| search type | from depth (research.md D2: quick and standard `auto`, deep `deep-lite`) |
| include domains | `constraints.domains_allow` (may be empty) |
| exclude domains | `constraints.domains_deny` (may be empty) |

## Search response

- **Search hit**: `url`, `title`.
- **Request cost**: `costDollars.total` in USD, or none when Exa omits it.

## Contents request (one per run, kept sources only)

| Field | Source |
|---|---|
| urls | the kept candidates after dedup, domain filter and `max_sources` (≤ 60) |
| max age hours | `EXA_MAX_AGE_HOURS` (default 168) |
| max characters | `EXTRACT_TEXT_MAX_CHARS` (16,000) |

## Contents response

- **Page**: `url`, `title`, `text` (possibly empty), `truncated` (text
  length ≥ 99% of max characters); a URL with an `error` status is absent.
- **Request cost**: as for search.

## Run meter (existing, extended)

- Adds the sum of reported Exa costs and a flag set when any Exa request
  reported none.

## Invocation record (existing, extended)

| Field | Type | Meaning |
|---|---|---|
| `search_cost_usd` | nullable real | Sum of the run's Exa costs; null when any request reported none, and for every tool other than `research`. |

Stored in a new nullable `invocation_records.search_cost_usd` column, added by
the existing additive migration. Older rows read as null.

## Research result (existing, one additive field)

- `sources[].truncated` (bool): the source's text reached the character cap.
  Every other field is unchanged; `fetched_at` keeps its meaning as the time
  Parallax received the source's text.

## Configuration

| Variable | Change |
|---|---|
| `EXA_API_KEY` | New. Presence enables `research`. |
| `EXA_MAX_AGE_HOURS` | New. Integer -1..=720, default 168. |
| `BRAVE_API_KEY` | Removed. Present without `EXA_API_KEY` → startup error naming both; with `EXA_API_KEY`, ignored. |
| `FETCH_ALLOW_PRIVATE` | Removed. Present → startup error naming it. |
| `FETCH_TIMEOUT_MS` | Removed. Present → startup error naming it. |
