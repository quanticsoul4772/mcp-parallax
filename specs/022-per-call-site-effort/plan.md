# Implementation Plan: Per-Call-Site Reasoning Effort

**Branch**: `022-per-call-site-effort` | **Date**: 2026-07-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/022-per-call-site-effort/spec.md`

## Summary

Every model call runs at the provider's default reasoning effort, `high` —
including `check`'s claim-to-formal-target translation and research's per-source
claim extraction, which are transcription-shaped work. A `PARALLAX_EFFORT_*`
namespace mirrors the `PARALLAX_MODEL_*` namespace 018 established: per-site and
per-tier, most-specific-first, resolved independently of the model. Unset sends
no field, so an upgrade changes nothing.

**Retrofitted plan.** The implementation preceded these artifacts; see the spec's
*Process deviation* section. The Constitution Check below was therefore
performed against finished code rather than gating it, which is the weaker
direction and is recorded as such.

## Technical Context

**Language/Version**: Rust, MSRV 1.94

**Primary Dependencies**: unchanged — no crate added, removed, or bumped.

**Storage**: N/A. No record or schema change; effort is a request property, and
the invocation record already attributes cost per model.

**Testing**: `cargo test`. Resolution and validation are pure functions in
`src/routing.rs`; the wire format is asserted through `wiremock` in
`src/client/anthropic.rs`. No network, no credentials.

**Target Platform**: MCP server over stdio.

**Project Type**: Single Rust library plus binary.

**Performance Goals**: N/A — resolution happens once at construction.

**Constraints**: Off by default (Principle VI) is the binding one, and it is
what makes "unset" a distinct state from `High` rather than a synonym for it.
Clippy `pedantic` + `nursery` denied; `resolve` was already near the 100-line
function limit, so the namespace validation is a free function beside it.

**Scale/Scope**: 12 call sites × 5 levels. Four files touched, none new.

## Constitution Check

*GATE: performed after implementation. Recorded honestly as such.*

| Principle | Verdict | Basis |
|---|---|---|
| **I. Design-Corpus Fidelity** | PASS with a named deviation | The feature traces to 018 research D7, a named deferral. The deferral's *mechanism* changed — `thinking: disabled` now 400s on newer families and `output_config.effort` is the supported control — which is itself a corpus amendment this change makes. **The deviation from the mandated sequence is named in the spec rather than slipped in**, which is what this principle requires of deviations. |
| **II. Constrained-Output Contract** | PASS | `effort` is added *beside* `format` under `output_config`, never replacing it; the wire test asserts both are present together. No mode schema changes. |
| **III. Compiler-Enforced Discipline** | PASS | No new `unwrap`/`expect`, no stdout write. An unparseable level is a startup error, not a silent fallback to a default — the fallback would leave the operator believing a setting took effect. |
| **IV. Seams, Composition, Tests** | PASS | No new external effect and no new seam. Resolution is pure; the client seam is unchanged in signature. Eight tests cover precedence, independence, validation, pooling, and both wire states. |
| **V. Deterministic Over Probabilistic** | PASS | Resolution is a total function of the environment. No model judgment anywhere in it. |
| **VI. Capabilities Off By Default** | PASS — the load-bearing one | Unset sends no field. Asserted on the wire by a test that fails if an `effort` key appears, and by an assertion that `output_config` has exactly one key when unset. |
| **VII. Simplicity and Scope Discipline** | PASS | No new module. The namespace validation is a free function because `resolve` would otherwise cross the 100-line limit — the split seam the principle asks for. No recommended default ships: choosing levels is the operator's, and guessing would be scope the spec does not ask for. |

**Post-implementation**: PASS. The one deviation is procedural, not architectural,
and is named above and in the spec.

### What the retrofit cannot recover

A Constitution Check performed before the code can reject a design. This one
could only describe a design already built and passing. Had it found a violation,
the honest outcome would have been rework — which is the cost of the deviation,
and the reason it is recorded rather than smoothed over.

## Project Structure

### Documentation (this feature)

```text
specs/022-per-call-site-effort/
├── plan.md                    # This file
├── spec.md                    # Feature spec, with the process deviation named
├── contracts/
│   └── config.md              # The environment surface this adds
├── checklists/
│   └── requirements.md        # Spec quality checklist
└── tasks.md                   # Retrofitted, marked complete
```

### Source Code (repository root)

```text
src/
├── routing.rs        # Effort enum; EFFORT_PREFIX; per-route effort +
│                     #   effort_source; independent most-specific-first
│                     #   resolution; validate_effort_namespace;
│                     #   distinct_clients keyed on (model, effort)
├── client/
│   ├── anthropic.rs  # optional effort field; for_model_and_effort;
│   │                 #   output_config.effort emitted only when set
│   └── pool.rs       # pool keyed on (model, effort) rather than model
└── server.rs         # the injected client is reused only when the site is
                      #   on the default model AND at no explicit effort

docs/design/SDK_LANDSCAPE.md   # amended: the thinking-suppression mechanism
specs/018-model-routing/       # amended: research D7's deferral is discharged
CHANGELOG.md                   # Unreleased: Added
```

**Structure Decision**: No new module. Effort belongs in `routing.rs` beside the
model resolution it mirrors — splitting them would mean two files reading the
same environment with the same precedence rule.

## Complexity Tracking

No architectural violations. The single deviation is procedural and named above.
