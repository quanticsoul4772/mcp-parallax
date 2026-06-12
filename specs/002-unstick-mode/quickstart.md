# Quickstart: Unstick Mode

## Invoke

From any connected MCP client, call `unstick`:

```json
{
  "goal": "Get the integration test suite passing on CI",
  "blocked": "The same two tests fail on CI but pass locally; logs show no diff",
  "tried": [
    "Re-running the CI job",
    "Pinning the toolchain version",
    "Adding debug logging to the failing tests"
  ]
}
```

Expected shape:

```json
{
  "next_step": "Run the two failing tests locally with the CI environment variables exported (copy them from the CI job definition) to reproduce the environmental difference.",
  "rationale": "The pass/fail split between local and CI with identical code means the difference is environmental; reproducing the CI environment locally isolates it.",
  "watch_for": "Secrets in CI env vars that cannot be copied locally - substitute dummies and check whether the failure mode changes."
}
```

Exactly one step; never options, never a plan. The step will not restate a
`tried` item.

## Acceptance (SC-002/003/004; manual-run, real spend)

```bash
ANTHROPIC_API_KEY=... cargo run --example acceptance_unstick
```

Runs 10 varied stuck scenarios; asserts structural validity, one-step shape,
zero restatements of tried items, and per-call latency. Results recorded below.

## Records (SC-005)

Same table as verify — the `tool` column distinguishes the correctives:

```bash
sqlite3 ./data/parallax.db "SELECT tool, outcome, latency_ms FROM invocation_records ORDER BY created_at DESC LIMIT 10;"
```
