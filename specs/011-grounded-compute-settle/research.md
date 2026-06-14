# Research: Grounded Compute-Settle

Phase 0 decisions. The clarification settled the user-facing scope (line/byte/match
counts over a single source, numeric threshold; everything else abstains). These
resolve the mechanism against the existing code.

## D1 — How the property + threshold reach the server

**Decision**: extend the per-pass `GroundedPass` schema (010) with **flat nullable
string/integer fields** the model fills when it sets `needs_computation` and the
property is in the supported class:

- `compute_property`: nullable **string** — one of `"lines"`, `"bytes"`, `"matches"`,
  **validated server-side** against that set; any other value is out-of-class → abstain.
- `compute_match_literal`: nullable string (the literal to count; only for `matches`).
- `compute_operator`: nullable **string** — one of `">"`, `">="`, `"<"`, `"<="`,
  `"=="`, `"!="`, **validated server-side**; any other value → abstain.
- `compute_threshold`: nullable integer (the numeric bound the claim asserts).

**Rationale**: the model is the only party that can read the claim and name *what* to
count and *against what bound* — but it must not produce the *value* or the *verdict*
(FR-003). Emitting the property/operator/threshold as structured fields keeps the model
bounded to identification; the server counts and the engine decides. No extra model hop:
the existing passes already run, so the fields ride the pass they already emit.

**Why nullable *string*, not nullable *enum* (analyze H1).** `schemars` encodes an
`Option<SomeEnum>` as an `anyOf`/`allOf` wrapper, and the flat-schema gate
(`modes/mod.rs` `assert_flat`) **rejects `anyOf` at boot** — so a nullable Rust enum
field would fail startup. The only nullable precedent in the codebase is nullable
*scalars* (008 `SourceLocator`'s `Option<String>`/`Option<u32>`), never a nullable enum.
Representing `compute_property`/`compute_operator` as `Option<String>` keeps the
`type: ["string","null"]` shape `assert_flat` already admits, and the closed value set
is enforced by a **server-side validator** (an unrecognized value is treated as
out-of-class → abstain, FR-005), not by the grammar. This trades a grammar-level enum
constraint for a server check — consistent with the project's existing thin-validator
posture, and strictly safer than risking a boot failure.

**Constitution II (flat + closed)**: each field is a nullable scalar (`type:
["string","null"]` / `["integer","null"]`) — exactly the shapes `assert_flat` admits
(verified by the 010 `needs_computation` boolean and the 008 `Option<T>` precedent). No
enum, no `anyOf`, no nested object; the pass schema stays flat + closed.

**Alternatives**: nullable enum fields (rejected — the H1 boot-failure risk above); a
second extraction model-call (rejected — an extra hop to re-derive what the passes
already read); server-side regex parsing of the claim (rejected — brittle, and it
re-implements claim understanding the model already does).

## D2 — What the count runs over

**Decision**: count over the **raw verbatim source content** of the single read unit —
`SourceContent.text` / `.bytes` as the reader returned it — **never** the
header-framed `AssembledEvidence.text` (which interleaves server-generated provenance
headers). `lines` = number of lines in the raw content; `bytes` = `content.bytes`
(already on the manifest); `matches` = count of `compute_match_literal` occurrences in
the raw content.

**Single-source gate**: the compute path engages **only when assembly produced exactly
one read unit** (`units.len() == 1`). A glob expanding to many files, or multiple
locators, is multi-source → abstain (clarification, FR-005). `assemble` must surface the
raw per-unit content (today it frames and discards it); capture it alongside the
manifest rather than re-reading.

**Rationale**: counting over the framed evidence would include header bytes/lines and
corrupt the value — the exact "computed-but-wrong" failure FR-005 forbids. The raw
content is what the claim is about. Single-unit gating is the mechanical form of the
single-source clarification and is unambiguous.

**Line-count convention**: count newline-terminated lines as the reader read them;
fix the off-by-one rule (trailing newline) in the data model and test both an
LF-terminated and a no-trailing-newline file so the convention is pinned, not assumed.

## D3 — How the claim is settled (reuse `check`, skip the model)

**Decision**: construct the comparison string `"{value} {operator} {threshold}"` (e.g.
`"1224 > 1000"`) and call **`crate::deterministic::arithmetic::evaluate`** directly.
`ArithmeticOutcome { holds, result_text }` is the verdict carrier: `holds == true` →
`supported`, `false` → `refuted`; `result_text` is the engine's raw result for audit.

**Rationale**: the `check` tool's normal spine is *translate (model) → execute
(engine)*. Here the server **already holds the value and the operator/threshold**, so
the translation hop is unnecessary — calling `arithmetic::evaluate` is reusing the
exact deterministic engine `check` uses (005 D2, evalexpr), with the model classifier
bypassed because there is nothing left to classify. This is "reuse the engine, not
reimplement" (FR-007) in its strongest form.

**Alternatives**: route through `check::run` with a synthesized claim string (rejected —
re-invokes the model translator to re-derive a comparison the server already has, an
extra hop and a new failure surface); a fresh arithmetic evaluator (rejected — duplicates
`arithmetic::evaluate`).

## D4 — Aggregation: when to settle vs abstain

**Decision**: after the existing 010 aggregation, if a majority of completed passes set
`needs_computation`, examine their compute fields:

1. The agreeing passes must converge on a **single in-class spec**: same
   `compute_property` (validated in `{lines,bytes,matches}`), same `compute_operator`
   (validated in the six comparisons), same `compute_threshold`, and (for `matches`)
   same `compute_match_literal`. Take the spec held by a majority of the
   needs_computation passes.
2. **And** assembly produced exactly one read unit (single-source, D2).
3. **And** the claim is **purely computable** — the agreeing passes carry no substantive
   judgment `findings` (the compound-claim gate, D6). A claim like "X is over 1000 lines
   *and well-structured*" is **not** purely computable and must not be settled on its
   count alone.
4. If all hold → the server counts the property and calls `arithmetic::evaluate`;
   verdict = `supported`/`refuted` from `holds`, with the executed form and result.
5. Otherwise (passes disagree on the spec, property/operator out of class, multi-source,
   any missing field, an `arithmetic::evaluate` error, or a non-empty judgment component)
   → **abstain** with 010's `inconclusive` (route to `check`). No verdict is ever emitted
   over a value the server did not derive **or** over a claim with an unsettled judgment
   half (FR-005, the 010 guarantee).

**Rationale**: requiring an agreed, in-class, single-source, *purely computable* spec is
the conservative gate that makes the compute path provably correct on exactly the
clarified class and defers everything else to the safe 010 behavior. Disagreement among
passes about *what* to count — or a judgment component the count cannot settle — is
itself a signal to abstain.

## D6 — Per-pass verdict on a computable claim, and the compound-claim gate (analyze M1/M2)

**Decision (M1)**: a pass that flags `needs_computation` MUST still emit a valid
`verdict` + `findings` (both are required, 010). The prompt instructs it to emit
`verdict: "supported"` with a one-line note (e.g. "computable: line count needed") and
**empty** `findings` — so it never trips the 010 "refutation with no findings is a
failed pass" guard (`one_pass`) and is never dropped, which would risk quorum loss. The
server ignores this pass-level verdict on the compute path; the engine decides.

**Decision (M2)**: the compound-claim gate (D4 step 3) keys off the agreeing passes'
`findings`: if a `needs_computation` pass also reports a substantive judgment finding,
the claim is compound and the server abstains rather than settling on the count alone.
This prevents the worst regression — `supported` on the count while a false judgment half
is ignored. Pinned by a test (a compound claim → `inconclusive`).

**Rationale**: M1 keeps the compute fields riding a structurally valid pass; M2 keeps the
settle path from answering more than the count actually settles. Both are conservative —
they expand the abstain set, never the confidently-answered set.

## D5 — Output surface

**Decision**: `GroundedVerdict` (010) gains two **optional** server-assembled fields,
present only on a settled compute verdict:

- `executed_form`: the comparison string the engine decided (e.g. `"1224 > 1000"`).
- `engine_result`: the engine's raw result text (`ArithmeticOutcome.result_text`).

The verdict value stays the 010 `GroundedVerdictKind` (`supported`/`refuted` on a settle,
`inconclusive` on abstain). The per-pass schema gains the four D1 fields; nothing else
about the output changes. `verify` is untouched (its pass schema is separate).

**Rationale**: mirrors `check`'s auditable output (formal form + engine result) so a
settled grounded verdict is verifiable the same way (FR-002). Optional fields keep the
judgment-path and abstain-path outputs byte-identical to 010 (no regression).
