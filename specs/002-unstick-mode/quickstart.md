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

## Results (acceptance pass — 2026-06-12)

Run live via `cargo run --example acceptance_unstick` against `claude-opus-4-8`
(10 single-pass calls). **All criteria passed:**

| Criterion | Result | Target |
|---|---|---|
| SC-002 valid results | 10/10 | 100% |
| SC-003 menu/plan leakage | 0/10 | 0 |
| SC-003 tried-restatements | 0 (code-enforced; any would have errored) | 0 |
| SC-004 max single-call latency | 6.5 s | < 15 s |

SC-001/SC-005/SC-006 are covered by the test suite: catalog lists both tools
with contract-matching schemas, every invocation leaves one record attributed
to its tool, and the pre-existing suite passes with one knowingly-updated
assertion (the stdio smoke test asserts the *full catalog*, which gaining a
second tool is the feature itself, not a change to verify's behavior).

## Records (SC-005)

Same table as verify — the `tool` column distinguishes the correctives:

```bash
sqlite3 ./data/parallax.db "SELECT tool, outcome, latency_ms FROM invocation_records ORDER BY created_at DESC LIMIT 10;"
```
