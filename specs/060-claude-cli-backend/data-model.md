# Data Model: Claude Code CLI backend

## Configuration (new and changed)

| Variable | Default | Range / values | Notes |
|---|---|---|---|
| `PARALLAX_BACKEND` | `anthropic` | adds `claude_cli` | |
| `CLAUDE_CLI_PATH` | `claude` | any path or command name | must run `--version` at startup (D9) |
| `CLAUDE_CLI_MODEL` | `claude-sonnet-5-5` | model id | this backend's default model (D4) |
| `CLAUDE_CLI_CONCURRENCY` | `4` | `1..=32` | live CLI processes, server-wide (D5) |
| `ANTHROPIC_API_KEY` | — | — | not required on `claude_cli`; removed from the child's environment |

A present-but-unparseable value is a startup error, as elsewhere.

## CLI call (in memory)

| Field | Source |
|---|---|
| program | `CLAUDE_CLI_PATH` |
| arguments | contract §Arguments: model, schema, isolation flags, system prompt |
| stdin | the prompt |
| environment | server environment minus the D3 list, plus `CLAUDE_CODE_EFFORT_LEVEL` |
| working folder | the per-process empty folder |
| timeout | `REQUEST_TIMEOUT_MS` |

## CLI reply (parsed fields only)

`subtype`, `is_error`, `stop_reason`, `terminal_reason`, `api_error_status`,
`result` (error text only), `structured_output`, `usage.input_tokens`,
`usage.cache_creation_input_tokens`, `usage.cache_read_input_tokens`,
`usage.output_tokens`, `total_cost_usd`. Unknown fields are ignored.

## `Completion` (changed)

Adds `reported_cost_micros: Option<u64>` — `None` on the anthropic and
openai_compat backends.

## `invocation_records` (changed)

| Column | Type | Values |
|---|---|---|
| `cost_basis` | `TEXT NULL` | `price_table`, `cli_estimate`; `NULL` on rows written before 060 |
| `outcome` | existing | adds `usage_limit` |

## Telemetry (changed)

Span attribute `parallax.cost_basis` (same values); outcome value
`usage_limit` on spans and metrics.
