# Configuration Contract — the `PARALLAX_MODEL_*` namespace

The operator-facing surface of 018. Anything not listed here is not read by the
routing layer. The namespace is **reserved**: a `PARALLAX_MODEL_*` variable whose
suffix is not in one of the two tables below is a startup error, not an ignored
setting.

## Enablement

| Condition | Effect |
|---|---|
| No `PARALLAX_MODEL_*` variable set | Routing off. Every call site uses `ANTHROPIC_MODEL`, exactly as before this feature (FR-002). |
| One or more set | Each call site resolves independently; the startup routing table reports the outcome. |

Routing is operator-only. No tool argument, prompt content, or caller-supplied value
influences which model answers (FR-003, FR-016).

## Tier settings

| Variable | Applies to | Default |
|---|---|---|
| `PARALLAX_MODEL_BULK` | `research_extract` | `ANTHROPIC_MODEL` |
| `PARALLAX_MODEL_JUDGMENT` | the other eleven call sites | `ANTHROPIC_MODEL` |

## Per-call-site settings

Each overrides its tier for that call site alone.

| Variable | Tier |
|---|---|
| `PARALLAX_MODEL_VERIFY` | judgment |
| `PARALLAX_MODEL_UNSTICK` | judgment |
| `PARALLAX_MODEL_DIVERGE` | judgment |
| `PARALLAX_MODEL_DECIDE` | judgment |
| `PARALLAX_MODEL_ELICIT` | judgment |
| `PARALLAX_MODEL_GROUNDED_VERIFY` | judgment |
| `PARALLAX_MODEL_CHECK_TRANSLATE` | judgment |
| `PARALLAX_MODEL_RESEARCH_SCOPE` | judgment |
| `PARALLAX_MODEL_RESEARCH_EXTRACT` | bulk |
| `PARALLAX_MODEL_RESEARCH_VERIFY` | judgment |
| `PARALLAX_MODEL_RESEARCH_SYNTHESIZE` | judgment |
| `PARALLAX_MODEL_CHECKPOINT_REVIEW` | judgment |

## Resolution order

Most specific wins:

```text
PARALLAX_MODEL_<CALL_SITE>  ->  PARALLAX_MODEL_<TIER>  ->  ANTHROPIC_MODEL
```

## Startup failures (all loud, none silent)

| Condition | Behavior |
|---|---|
| `PARALLAX_MODEL_*` with an unrecognised suffix | `ConfigError` naming the variable. Catches misspelled routes (FR-006a). |
| A recognised variable present but empty or whitespace-only | `ConfigError` naming the variable and value (FR-006). |
| A recognised variable naming a model with no price row | **Not** an error. The run proceeds and prices conservatively, marked `cost_estimated` (FR-012, D8). |

The server refuses to serve on either error; no tool call reaches a provider (SC-005).

## Startup routing table

Emitted once to the diagnostic stream (stderr) after config resolution and before
serving — never to stdout, which is the MCP JSON-RPC channel. Lists every call site,
its resolved model, and which setting supplied it, so an operator can confirm a route
took effect without issuing a call (FR-005, SC-007).

```text
parallax: routing resolved (12 call sites, 2 distinct models)
  research_extract     claude-haiku-4-5  PARALLAX_MODEL_BULK
  verify               claude-opus-5     ANTHROPIC_MODEL
  research_synthesize  claude-sonnet-5   PARALLAX_MODEL_RESEARCH_SYNTHESIZE
  ...
```

Route visibility adds no tool-catalog entry: the audience is the operator, and the
catalog is read by the calling model (FR-005a).

## Related existing variables

| Variable | Relationship |
|---|---|
| `ANTHROPIC_MODEL` | The fall-through for every call site. Unchanged meaning. |
| `REQUEST_TIMEOUT_MS` | Default rises with the output budget (D7). An operator routing to a family that reasons by default may need to raise it further. |
| `VOYAGE_MODEL` | Not routable. Embedding calls are a different provider and already attribute separately. |
