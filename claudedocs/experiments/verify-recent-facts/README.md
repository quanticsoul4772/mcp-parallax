# verify on recent facts without context (2026-11)

Evidence for the 2026-11-19 change to `verify`'s description, which now tells
callers to pass evidence as context for anything newer than the model's
training.

## What was measured

`verify` was called with a claim and **no context**, on `claude-opus-5`
(`PARALLAX_MODEL_VERIFY`), with the configured 3 passes, at two reasoning
effort levels:

- **medium**: first through the `PARALLAX_EFFORT_VERIFY=medium` setting (the
  easy claims; the record's `effort` column is empty because it holds only
  per-call overrides), then through the per-call `effort` argument (the sharper
  claims);
- **high**: the provider's default, through the per-call `effort` argument.

Each claim was run once at each level.

## Easy claims

| Claim | Correct answer | How it was established | medium | high |
|---|---|---|---|---|
| Rust 1.100.0 was released on 12 November 2026. | true | the Rust blog post `blog.rust-lang.org/2026/11/12/Rust-1.100.0/`, fetched by a research run the same day | supported, 1.0 | supported, 1.0 |
| MCP SEP-2577 deprecates Roots, Sampling and Logging. | true | the SEP text, fetched by research runs the same day | supported, 1.0 | supported, 1.0 |
| num-rational 0.4.2 works with num-bigint 0.5. | false | crates.io: num-rational 0.4.2 requires `num-bigint ^0.4.0`; Dependabot PR #140 failed to compile for this reason | refuted, 1.0 | refuted, 1.0 |

All verdicts were right, but the reasons were not evidence: the Rust date was
supported by counting the six-week release calendar, the SEP claim because no
error was found, and the num-rational refutation stated that no num-bigint 0.5
exists, which is false.

## Sharper claims

Each has a confirmed answer and a plausible guess that lands on the wrong side.

| Claim | Correct answer | How it was established | medium | high |
|---|---|---|---|---|
| num-bigint 0.5.1 has been published on crates.io. | true | Dependabot PR #140 bumped to it; its CI compiled num-bigint 0.5.1 | refuted, 1.0 (wrong) | refuted, 1.0 (wrong) |
| MCP SEP-2577 deprecates the Roots, Sampling, Logging and Elicitation features. | false | the SEP text lists Roots, Sampling and Logging only | supported, 0.67 (wrong) | refuted, 0.67 (right) |
| Rust 1.100.0 stabilized the never type. | true | the Rust 1.100.0 release announcement, fetched by a research run the same day | refuted, 1.0 (wrong) | refuted, 1.0 (wrong) |

medium: 0 of 3 right. high: 1 of 3 right, and its correct refutation of the
SEP claim also rested on stale facts (that no SEP-2577 exists and that none of
the four features is deprecated).

## Time, tokens and cost

From `invocation_records` in the Parallax database. The calls in each group ran
in parallel, so a record cannot be tied to a particular claim; ranges are given
per group.

| Group | Time | Output tokens | Cost per call |
|---|---|---|---|
| easy, medium | 8.5–10.2 s | 1,140–1,725 | $0.039–$0.054 |
| easy, high | 15.2–25.0 s | 2,303–4,295 | $0.068–$0.118 |
| sharper, medium | 8.8–18.3 s | 1,385–2,916 | $0.045–$0.084 |
| sharper, high | 12.3–33.5 s | 2,102–6,437 | $0.063–$0.172 |

Earlier default-effort verify calls on other claims took 35–70 s and wrote
4,400–11,700 output tokens.

## What it shows

- With no context, verify judges from the model's training knowledge. At both
  levels it refuted two true facts newer than that training at confidence 1.
- high caught one false claim that medium supported, so medium was not kept.
- Both levels stated stale facts as findings, for example that no num-bigint
  0.5 release exists.

## Limits

Six claims, one run each, one model (`claude-opus-5`). The easy claims cannot
separate the two levels: both gave the same verdicts and the same reasoning.
