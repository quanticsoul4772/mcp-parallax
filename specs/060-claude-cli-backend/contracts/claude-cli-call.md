# Contract: one Claude Code CLI call

The exact process Parallax starts for one model call on `claude_cli`. Tests
assert against this document through the `CliRunner` double.

## Arguments (in this order)

```text
<CLAUDE_CLI_PATH>
  -p
  --output-format json
  --json-schema <sanitized schema, compact JSON>
  --model <routed model id>
  --strict-mcp-config
  --mcp-config {"mcpServers":{}}
  --tools ""
  --setting-sources project
  --safe-mode
  --no-session-persistence
  --max-turns 4
  --system-prompt <SYSTEM_PROMPT>
```

`SYSTEM_PROMPT`: "Answer the request using only the required structured
output. Do not use tools."

The prompt is never an argument.

## stdin

The prompt, UTF-8, then end of input.

## Environment

Inherited, minus: `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`,
`CLAUDE_CODE_USE_BEDROCK`, `CLAUDE_CODE_USE_VERTEX`,
`CLAUDE_CODE_USE_FOUNDRY`, `CLAUDE_CODE_EFFORT_LEVEL`.
Plus: `CLAUDE_CODE_EFFORT_LEVEL=<low|medium|high|xhigh|max>` for a routed or
per-call effort, else `CLAUDE_CODE_EFFORT_LEVEL=unset`; and
`CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`, `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`.

## Working folder

One folder per server process, created fresh with a unique name under the
OS temp directory (`create_dir`, so an existing folder is never reused;
private to the operator on Unix). A `.cmd`/`.bat` `CLAUDE_CLI_PATH` is
refused at startup: a timeout could kill only the shell.

## Bounds

Killed at `REQUEST_TIMEOUT_MS`, or when the invocation is cancelled. At most
`CLAUDE_CLI_CONCURRENCY` processes live at once, server-wide.

## Reply classification

| Condition (checked in order) | Result | Outcome |
|---|---|---|
| process not found | error naming `CLAUDE_CLI_PATH` | `validation_failure` (client) |
| stdout not one JSON object | error with stderr (≤ 2,000 chars) | `validation_failure` (client) |
| `stop_reason` = `refusal` | refusal | `refusal` |
| `api_error_status` 401 or 403 | "sign in to Claude Code" | `provider_rejected` |
| `api_error_status` 429, or a usage-limit `api_error` | "subscription usage limit reached" | `usage_limit` (not retried) |
| other 4xx | provider message | `provider_rejected` |
| 5xx | retried under `MAX_RETRIES` | `retries_exhausted` if all fail |
| `error_max_structured_output_retries`, or no `structured_output` | schema failure, fed to the single retry | `validation_failure` |
| `error_max_turns` | turn cap reached | `validation_failure` |
| success with `structured_output` | `Completion` | — |

`Completion.input_tokens` = `input_tokens + cache_creation_input_tokens +
cache_read_input_tokens`; `output_tokens` = `output_tokens`;
`reported_cost_micros` = `round(total_cost_usd × 1,000,000)` when present.
