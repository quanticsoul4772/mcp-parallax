# Quickstart: Preference Elicitation

No new configuration. `elicit` is always in the catalog (like `verify` / `unstick` /
`diverge` / `decide`). When a Voyage key is set, it also consults your stored verified
preferences.

## Surface the objective before you commit

```jsonc
{ "task": "Add a caching layer to speed up the report endpoint" }
// => (memory configured, with a stored "avoids new services" preference)
{
  "assumed_objective": "Add a caching layer to speed up the endpoint",
  "governing_preferences": [
    { "preference": "Prefers minimal new infrastructure", "signal": "stored memory", "strength": "revealed" }
  ],
  "divergence_points": [
    { "question": "Is a cache the goal, or is the real objective lower p99 — which a query fix could serve without new infra?",
      "signal": "stored 'avoids new services' conflicts with 'add a cache'" }
  ],
  "signal_level": "medium",
  "memory_consulted": true
}
```

The tool names the objective it was about to pursue, the preference that should govern it
(here a *revealed* one from memory, outranking the surface request), and the specific point
where the assumed objective may be wrong — the question worth resolving first.

## Inference, not interrogation

```jsonc
// A task with no preference signal → no fabricated preferences, no questionnaire:
{ "task": "Rename the variable `tmp` to something clearer" }
// => { "assumed_objective": "...", "governing_preferences": [], "divergence_points": [],
//      "signal_level": "low", "memory_consulted": false }
```

With little signal the tool reports `signal_level: low` and returns nothing fabricated.

## What it does not do

- It never blocks, holds, or modifies an action — there is no enforcement field in the
  output. Holding an action that conflicts with a stored preference is `checkpoint_action`'s
  job; `elicit` runs earlier and only surfaces.
- It does not choose among options (that's `decide`), judge a claim (`verify`), or commit to
  a step (`unstick`).

## Validation

- Offline (`cargo test`): the per-pass schema registers flat + closed; arity/strength
  validation is a loud failed pass; a low-signal canned inference → empty preferences and
  divergence; the output carries no enforcement field; `memory_consulted` reflects presence;
  with a seeded trusted memory + mock embedder the recall reaches the prompt (the mock model
  captures the memory content). All deterministic.
- **Live** (dogfood): SC-001 (surfaces the *right* objective), SC-002 (catches a seeded
  stated-vs-revealed conflict as a divergence point), and that the model marks stored
  preferences `revealed` — model-judgment properties a mock cannot produce.
