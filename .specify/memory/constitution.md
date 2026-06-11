<!--
Sync Impact Report
==================
- Version change: template (unversioned) → 1.0.0 (initial ratification)
- Modified principles: n/a — all template placeholders filled for the first time
- Added sections:
  - Core Principles (I–VII)
  - Quality Gates & Tooling (concretizes SECTION_2)
  - Development Workflow (concretizes SECTION_3)
  - Governance
- Removed sections: none
- Templates:
  - ✅ .specify/templates/tasks-template.md — tests changed from OPTIONAL to
    REQUIRED to match Principle IV (updated in the same change as this file)
  - ✅ .specify/templates/plan-template.md — Constitution Check gates are
    resolved from this file at plan time; no structural change required
  - ✅ .specify/templates/spec-template.md — no constitution-mandated section
    changes; success criteria remain technology-agnostic
  - ✅ .specify/templates/checklist-template.md — no change required
- Follow-up TODOs: none
-->

# Parallax Constitution

Parallax is an LLM-augmentation MCP server (Rust, stdio): a catalog of
correctives for the calling model's predictable failure modes — metacognition
the model can't run on itself. This constitution governs how it is built. The
design corpus in `docs/design/` governs *what* is built.

## Core Principles

### I. Design-Corpus Fidelity (NON-NEGOTIABLE)

`docs/design/NEW_SERVER_DESIGN.md` is the source of truth and indexes the rest
of the corpus; `SDK_LANDSCAPE.md` fixes the crate stack per layer. Every
feature MUST trace to the corpus, and every deviation — a different crate, a
dropped layer, a narrowed scope, a skipped spike — MUST be named and justified
in the spec or plan, never slipped in silently. When implementation experience
proves a design section wrong, the corpus MUST be amended in the same change;
drift in either direction is a defect.

**Rationale**: the value model, layer split, and failure-mode catalog were
deliberately researched and decided once. Silent divergence reintroduces
exactly the drift this server exists to catch.

### II. The Constrained-Output Contract

Every mode MUST declare an output JSON Schema. Model output is constrained via
Anthropic's native structured outputs, with the thin schema validator enforcing
what the API grammar drops (numeric/length constraints, recursion bans). Mode
schemas MUST be flat and closed (`additionalProperties: false`). Free-text
parsing, `extract_json`-style scraping, and `tool_choice` hacks are forbidden.

**Rationale**: the contract is what makes a corrective's result machine-usable
and testable; one unconstrained mode reintroduces parsing fragility everywhere.

### III. Compiler-Enforced Discipline

`#![forbid(unsafe_code)]`. Production paths MUST NOT contain
`unwrap`/`expect`/`panic!` (clippy-denied) or write to stdout
(`clippy::print_stdout` denied — stdout is the MCP JSON-RPC channel; logging is
structured `tracing` to stderr only). Lint policy lives in both
`Cargo.toml [lints]` and the `lib.rs` preamble and MUST change in lockstep.
On any error: read it, fix the actual broken thing — fallbacks, swallowed
errors, and graceful-degradation wrappers that hide failures are forbidden.

**Rationale**: every guarantee a machine can enforce MUST NOT depend on review
vigilance. A single stray `println!` corrupts the protocol for every client.

### IV. Seams, Composition, and Tests (NON-NEGOTIABLE)

Every external effect — network, disk, clock, model calls — sits behind a
mockable trait (`ModelClient`, `Storage`, `TimeProvider`, and successors).
Composition over trait inheritance. The entire server MUST remain testable
without network or disk. Tests are REQUIRED for every feature, written through
the trait seams; a feature without tests is incomplete, not minimal.

**Rationale**: the seams exist so correctness never requires live credentials
or wall-clock luck; untested code through those seams wastes the architecture.

### V. Deterministic Over Probabilistic

Anything checkable MUST be settled by a deterministic component — a solver, a
validator, a type system — never by a probabilistic judge. LLM judgment is
reserved for what cannot be checked mechanically.

**Rationale**: the server's thesis is catching model error; using a model to
check what a solver can prove imports the failure mode it exists to correct.

### VI. Capabilities Off By Default

Every new capability with effects beyond the process — network egress, code
execution, shell-out — MUST be env-gated and OFF by default. Enabling one is a
deliberate operator decision, never a side effect of an upgrade.

**Rationale**: an MCP server runs inside other people's sessions; surprise
capability escalation is a security defect even when the capability is useful.

### VII. Simplicity and Scope Discipline

Build only what the spec asks. MVP first; YAGNI. New `.rs` modules target
≤500 lines — crossing it is a signal to find the split seam. Build the full
agreed scope: cutting or deferring is allowed only when named explicitly.

**Rationale**: the failure-mode catalog grows by deliberate addition, not
accretion; small modules keep each corrective independently understandable.

## Quality Gates & Tooling

The full local gate MUST pass before every commit (mirrored by pre-commit
hooks and the `/validate` command, enforced again in CI):

```bash
cargo fmt --all -- --check && cargo clippy --all-features -- -D warnings && cargo test
```

- Clippy runs `all` + `pedantic` + `nursery`; warnings are denied in CI.
- MSRV is declared in `Cargo.toml` (`rust-version`) and verified by a dedicated
  CI job pinned to the same version; the two MUST move in lockstep, by hand —
  the `dtolnay/rust-toolchain` pin is exempt from Dependabot for this reason.
- A weekly `cargo audit` workflow gates known advisories.
- Toolchain is pinned in `rust-toolchain.toml`.

## Development Workflow

- Spec-driven: features flow through the Spec Kit sequence (`/speckit-specify`
  → `clarify` → `plan` → `tasks` → `analyze` → `implement`). The plan's
  Constitution Check gates against this document before research begins.
- Feature branches only; incremental commits with meaningful messages; diff
  before staging.
- Before merge, changes touching the tool surface, layers, schemas, trait
  seams, or dependency stack are reviewed against the design corpus
  (`design-reviewer` agent); all Rust changes are reviewed for convention
  violations the compiler can't see (`code-reviewer` agent).
- Report outcomes straight: if a check fails, show the output; do not minimize
  a real problem. No partial features, TODO stubs, or "not implemented" throws
  on a path a feature claims to deliver.

## Governance

This constitution supersedes other practice documents where they conflict;
`CLAUDE.md` and `README.md` are runtime guidance and MUST stay consistent with
it. Amendments are made by PR that edits this file, updates the Sync Impact
Report comment, propagates changes to the `.specify/templates/` artifacts, and
bumps the version:

- **MAJOR**: removing or redefining a principle in a backward-incompatible way.
- **MINOR**: adding a principle or section, or materially expanding guidance.
- **PATCH**: clarifications and wording that do not change meaning.

Compliance is verified at two points: the Constitution Check gate in every
implementation plan, and review-time checks (the gate commands plus the
reviewer agents) before merge. Complexity that violates Principle VII MUST be
justified in the plan's Complexity Tracking table or rejected.

**Version**: 1.0.0 | **Ratified**: 2026-06-11 | **Last Amended**: 2026-06-11
