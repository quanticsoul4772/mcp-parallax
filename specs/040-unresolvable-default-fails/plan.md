# Implementation Plan: An Unresolvable Default Fails Instead of Being Skipped

**Branch**: `029-unresolvable-default-fails` | **Date**: 2026-07-27 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/040-unresolvable-default-fails/spec.md`

## Summary

Replace the two near-duplicate scans that read configuration defaults out of
source with **one shared resolver** that handles all four ways a default is
expressed, refuses to skip anything silently, and is used by every document
check rather than copied into each.

The coverage that follows — named numeric constants, named string constants,
inline string literals — is the consequence. The requirement is FR-001: a
default the resolver cannot resolve fails and names the variable.

## Technical Context

**Language/Version**: Rust, stable pinned via `rust-toolchain.toml`; MSRV 1.94

**Primary Dependencies**: none added. The resolver reads source text through
`include_str!`, as the existing scans do.

**Storage**: N/A

**Testing**: `cargo test`; the resolver and its callers are test-only code

**Target Platform**: stdio MCP server, Windows and Linux

**Project Type**: single Rust binary + library

**Performance Goals**: N/A — runs once per test, over a few thousand lines

**Constraints**: `include_str!` reads files as they sit on disk, which is CRLF
here; 039 shipped a boundary search that matched nothing because of it and
reported every variable missing. Any line- or block-oriented parsing normalises
first.

**Scale/Scope**: 20 configuration variables, 4 default shapes, 2 documents
today and a third whenever one is added

## Constitution Check

*GATE: evaluated before Phase 0 research; re-evaluated after Phase 1 design.*

| Principle | Assessment | Verdict |
| --- | --- | --- |
| **I. Design-Corpus Fidelity** | Implements §10's rule directly — derive the facts, hand-write the reasons — and closes the third instance of partial derivation reported as derivation. The corpus gains nothing new; this is the rule being applied where it was written and then not followed. | PASS |
| **II. Constrained-Output Contract** | No mode schemas touched. | PASS |
| **III. Compiler-Enforced Discipline** | The whole feature *is* this principle: a silent `continue` is a fallback that hides a failure, and FR-001 removes it. The resolver is test-only, so its `panic!`/`assert!` are permitted the way `assert_constraints_agree` already is. | PASS |
| **IV. Seams, Composition, Tests** | One resolver used by every caller rather than copied per document — the divergence between the two current copies is what let 039 inherit 036's blind spot. No network, no disk beyond `include_str!`. | PASS |
| **V. Deterministic Over Probabilistic** | Pure text over source; no model judgment anywhere. | PASS |
| **VI. Capabilities Off By Default** | Adds no capability and no egress. | PASS |
| **VII. Simplicity and Scope Discipline** | The "well under 500 lines" estimate written here before implementation was wrong — one file reached 853 production lines. The pre-merge review named the seam, and it is real: document parsing shares nothing with source resolution except the assertion layer. Split into `config_facts/{mod,source,documents}.rs` at 247 / 476 / 185 lines. | PASS |

No entry requires a Complexity Tracking justification.

## Project Structure

### Documentation (this feature)

```text
specs/040-unresolvable-default-fails/
├── plan.md              # This file
├── spec.md              # Feature specification
├── research.md          # Phase 0 — four decisions
├── data-model.md        # Phase 1 — the resolver's types and rules
├── quickstart.md        # Phase 1 — what a contributor sees when it fails
├── contracts/
│   └── failure-modes.md # Phase 1 — every way the check fails, and its message
└── checklists/
    └── requirements.md  # Spec quality checklist (all passing)
```

### Source Code (repository root)

```text
src/
├── config_facts.rs   # NEW, #[cfg(test)]: the shared resolver — parse defaults
│                     # from source, resolve constants, expose them to callers
├── main.rs           # the two document checks become callers of it
└── config.rs         # unchanged; it is the subject, not a participant
```

**Structure Decision**: a new module rather than more code in `main.rs`, which
already carries the help text and six tests. The resolver is the artifact both
documents depend on, so it belongs somewhere neither owns.

`config_facts.rs` is declared from `main.rs` and therefore belongs to the
**binary** crate — even though its subject, `config.rs`, is a library module
(`src/lib.rs:44`).

**This is forced, not preferred.** A `#[cfg(test)]` item in the library is *not
visible to the binary's tests*: the binary's test build links the library
compiled without `cfg(test)`. Verified rather than assumed — a probe module
added to `lib.rs` under `#[cfg(test)]` fails from `main.rs`'s tests with
`cannot find probe_cfgtest in mcp_parallax`. A library-side resolver could not
serve the `--help` check, and `--help` lives in the binary.

Alternatives and why they lose: making the resolver non-`cfg(test)` ships dead
code in the library and puts assert-based code under Principle III's no-panic
rule; moving `help_text()` into the library relocates production code to suit a
test.

**What it costs.** FR-006 requires every document to be checked by the same
resolution. Both documents that exist are binary-reachable, so it holds today.
A future library-side document would need `help_text()` moved to the library
first — recorded here so that is discovered by reading rather than by compiler
error.

## Phase 0: Research

Complete — see [research.md](research.md). Four decisions:

- **D1**: one shared resolver, not two patched scans — the duplication is the
  transmission mechanism for the defect.
- **D2**: resolve constants from an enumerated file set (settled in clarify).
- **D3**: the reverse direction reads structured markers only (settled in
  clarify).
- **D4**: exclusions as a typed list in the resolver, not a data file.

## Phase 1: Design & Contracts

Complete — [data-model.md](data-model.md),
[contracts/failure-modes.md](contracts/failure-modes.md),
[quickstart.md](quickstart.md).

### Constitution re-check after design

Re-evaluated against the Phase 1 artifacts: all seven principles still PASS.
The design adds one test-only module and removes two copies of a scan; nothing
in the production path changes, and no schema, capability, or dependency moves.

**Two findings from implementation that this plan did not anticipate.**

*There are five default shapes, not four.* `FETCH_ALLOW_PRIVATE` defaults to a
boolean, and a path-qualified constant (`crate::client::anthropic::NAME`) is a
distinct case from a bare one. Both surfaced as `Unresolvable` — the design
working correctly on an unknown shape — and the right response was to handle
them rather than excuse the variables. `EXCLUSIONS` is still empty.

*The first extractor reported confidently wrong values.* Probing what it found
against real configuration showed `ANTHROPIC_API_KEY` — which has no default at
all — resolving to the API base URL, and `INPUT_MAX_CHARS` resolving to
`voyage-4`. The `env::var` window ran past the end of its own statement and
paired variables with later, unrelated closures. That is worse than the silent
skip it replaces: a wrong answer that looks like success is this feature's own
failure mode. Bounding extraction to the statement fixed it. It was found by
printing what the resolver actually resolved, not by the suite passing.

**One risk named rather than designed away.** The resolver parses Rust source as
text. It does not understand the language, so a default expressed in a form it
does not recognise — a function call, a `cfg!` branch, arithmetic — fails rather
than resolving. That is deliberate under FR-001 and will occasionally fail on
something a human can read at a glance. The alternative is a real parser, which
is a dependency and a large amount of machinery for twenty variables. The
mitigation is FR-002: the exclusion list turns such a case into a one-line
recorded decision.

## Complexity Tracking

No principle violations requiring justification.
