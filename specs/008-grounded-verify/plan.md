# Implementation Plan: Source-Grounded Verification (`grounded-verify`)

**Branch**: `008-grounded-verify` | **Date**: 2026-06-13 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/008-grounded-verify/spec.md`

## Summary

A new cognitive-corrective tool, `grounded-verify`, that closes `verify`'s
context-trust gap: the caller names source locators (exact file paths and
file/line ranges) within a single configured root, the **server** reads the
verbatim text and assembles it as the verification context, and the existing
stance-blind ensemble runs over that machine-assembled evidence. The caller
chooses which sources are relevant but cannot paraphrase, summarize, or smuggle
a conclusion into the evidence. The result carries the verify verdict plus an
auditable evidence manifest and a completeness signal naming omitted evidence.

Technical approach: reuse the existing `verify` ensemble (pass + aggregate)
unchanged; add (1) an eighth mockable seam `SourceReader` for root-confined
verbatim reads, (2) a deterministic assembly stage that resolves all locators
all-or-nothing and builds the manifest, and (3) a mode whose constrained-output
schema extends verify's with a `missing_evidence` field. Gated on a single
`GROUNDED_VERIFY_ROOT` env var; off by default.

## Technical Context

**Language/Version**: Rust 1.94 (edition 2021) — the existing crate.

**Primary Dependencies**: existing `rmcp` (tool surface), the `ModelClient`
seam (Anthropic native structured outputs), the existing `verify` mode/ensemble,
the schema sanitizer + local validator. New: a `SourceReader` seam over
`std::fs` with path-confinement. No new external crate (glob is deferred, so the
`glob` crate is **not** pulled in for v1).

**Storage**: existing `SqliteStorage` — one `invocation_record` per call. No new
tables, no schema change.

**Testing**: `cargo test` with a mock `SourceReader` (tests never touch disk),
plus an acceptance example mirroring `acceptance_*`. Path-confinement and
all-or-nothing resolution are pure-function tested against ground-truth tables.

**Target Platform**: stdio MCP server (Linux / Windows / macOS), same binary.

**Project Type**: single Rust project (the existing crate); no new project.

**Performance Goals**: file reads are bounded (byte ceiling); latency is
dominated by the K model passes, identical to `verify`. No new perf target
beyond the bounded-bytes ceiling.

**Constraints**: reads confined to one canonicalized root; total assembled bytes
bounded; text-only; off by default; no write capability.

**Scale/Scope**: one new tool, one new trait seam, one new mode, three new
config values, a corpus amendment. Reuses the verify ensemble verbatim.

## Constitution Check

*GATE: evaluated against `.specify/memory/constitution.md`.*

- **I. Design-Corpus Fidelity (NON-NEGOTIABLE)** — ⚠️ **Named deviation, amendment required.** `grounded-verify` is **not** in the original corpus; it was discovered this session when a `verify` pass rubber-stamped a paraphrased conformance claim that the verbatim code refuted. Per Principle I, a feature that the corpus does not yet describe requires the corpus to be **amended in the same change**. Resolution: amend `docs/design/NEW_SERVER_DESIGN.md` (failure-mode catalog + primitives) and `docs/design/CORRECTIVE_SELECTION.md` to register `grounded-verify` as a Verify-family corrective for the "context-curation trust gap," with this spec as the trace. Tracked in Complexity Tracking and as a task in `/speckit-tasks`. (Precedent: the 2026-06-12 watchdog→checkpoint amendment.)
- **II. Constrained-Output Contract** — ✅ the mode declares a flat, closed output schema (`additionalProperties: false`); numeric/length bounds enforced by the local validator. The schema is verify's plus a `missing_evidence` string array.
- **III. Compiler-Enforced Discipline** — ✅ `#![forbid(unsafe_code)]`, no `unwrap`/`expect` in production paths; lints unchanged.
- **IV. Seams, Composition, Tests (NON-NEGOTIABLE)** — ✅ the new `SourceReader` is a mockable seam; the whole feature tests without disk. Composition over inheritance (the mode composes reader + ensemble).
- **V. Deterministic Over Probabilistic** — ✅ locator resolution, root confinement, all-or-nothing failure, the manifest, and aggregation are all deterministic; only the judgment is the model, exactly as in `verify`.
- **VI. Capabilities Off By Default** — ✅ filesystem read is a new capability, gated on `GROUNDED_VERIFY_ROOT`; absent ⇒ the tool is not in the catalog and no read path exists.
- **VII. Simplicity and Scope Discipline** — ✅ reuses the verify ensemble rather than re-implementing; globs deferred; one seam, one mode. The byte/locator ceilings keep the surface bounded.

**Gate result**: PASS with one named deviation (Principle I — corpus amendment), justified and tracked below. No unjustified violations.

## Project Structure

### Documentation (this feature)

```text
specs/008-grounded-verify/
├── plan.md              # This file
├── research.md          # Phase 0 — decisions (seam, confinement, reuse, completeness)
├── data-model.md        # Phase 1 — entities, schemas, validation
├── quickstart.md        # Phase 1 — enable + a worked call
├── contracts/
│   └── grounded-verify.md   # Phase 1 — tool I/O + the model-pass schema
└── tasks.md             # Phase 2 — /speckit-tasks (not this command)
```

### Source Code (repository root)

```text
src/
├── traits/
│   └── source.rs        # NEW — SourceReader seam (8th seam): locator -> verbatim text + size, root-confined
├── grounded/
│   ├── mod.rs           # NEW — module root
│   ├── reader.rs        # NEW — SystemSourceReader: canonicalize + prefix-check confinement, text-only, range + byte bounds
│   └── assemble.rs      # NEW — resolve all locators (all-or-nothing), build AssembledEvidence + EvidenceManifest (pure)
├── modes/
│   ├── grounded_verify.rs   # NEW — mode: prompt + flat/closed schema (+missing_evidence), run = assemble -> verify ensemble -> result
│   ├── verify.rs        # MODIFIED — factor the pass+aggregate so grounded_verify reuses it (no behavior change to verify)
│   └── mod.rs           # MODIFIED — register the mode
├── config.rs            # MODIFIED — grounded_verify_root: Option<String>, _max_bytes, _max_locators (off by default; malformed = startup error)
├── server.rs            # MODIFIED — gate the tool on root presence; inject SourceReader dep
└── traits/mod.rs        # MODIFIED — export the seam

tests/
└── integration.rs       # MODIFIED — 008 block: catalog gating, verbatim-flips-verdict, manifest, all-or-nothing, confinement

examples/
└── acceptance_grounded_verify.rs   # NEW — SC-001..006 acceptance

docs/design/
├── NEW_SERVER_DESIGN.md     # MODIFIED — register grounded-verify (Principle I amendment)
└── CORRECTIVE_SELECTION.md  # MODIFIED — routing note for the context-curation gap
```

**Structure Decision**: single-project Rust crate, extended in place. The new
`grounded/` module mirrors `research/`'s layout (a hygiene-enforcing reader +
pure assembly), and the new `SourceReader` seam sits beside the existing six
traits. The mode reuses the verify ensemble rather than duplicating it.

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| Principle I — feature not in original corpus | The context-curation trust gap was discovered empirically this session and is a real Verify-family failure mode worth a dedicated corrective | "Don't build it / fold into verify" rejected: verify's stance-blindness is the very thing that makes hand-fed context untrustworthy; the corrective is a distinct primitive. The corpus is amended in the same change, as Principle I requires. |
| 8th trait seam (`SourceReader`) | Principle IV requires testing without disk; reading files inline would couple the mode to the real filesystem | Direct `std::fs` in the mode rejected — breaks the test-without-disk guarantee and the confinement logic would be untestable in isolation. |
