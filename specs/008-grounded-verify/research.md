# Research: Source-Grounded Verification

Phase 0 decisions. Each resolves a design unknown for `grounded-verify`; no
`NEEDS CLARIFICATION` remained after `/speckit-clarify` (locators, completeness,
failure mode, root count are all settled in the spec).

## D1 — Reuse the verify ensemble, don't duplicate it

**Decision**: `grounded-verify` is `verify` with a machine-assembled context.
Factor `verify`'s "run K stance-blind passes + aggregate (majority verdict,
agreement-derived confidence, collected findings)" into a shared routine that
both `verify` and `grounded_verify` call. `grounded_verify` prepends the
assembly stage and appends the manifest; the pass logic is identical.

**Rationale**: Principle VII (simplicity); the value is *where the evidence
comes from*, not new reasoning. Keeps one ensemble to maintain and guarantees
identical stance-blindness.

**Alternatives**: a separate ensemble for grounded-verify — rejected (DRY
violation, two aggregation code paths to keep in sync).

## D2 — An 8th mockable seam: `SourceReader`

**Decision**: add `trait SourceReader` (beside the six existing seams) with one
operation: resolve a `SourceLocator` to verbatim text + byte size, or a typed
error. The production impl `SystemSourceReader` wraps `std::fs`; tests inject a
mock so the suite never touches disk.

**Rationale**: Principle IV (test without disk) and the existing composition
convention. The confinement and range/byte logic become unit-testable in
isolation, mirroring how `Fetcher` isolates network hygiene.

**Alternatives**: inline `std::fs` in the mode — rejected (couples to disk,
confinement untestable without real files).

## D3 — Path confinement: canonicalize + prefix-check

**Decision**: at startup, canonicalize the configured root once. For each
locator, join → canonicalize the resolved path → assert the canonical path is
prefixed by the canonical root. This defeats `../` traversal **and** symlink
escape (canonicalization follows symlinks before the check). Reject otherwise,
before any byte is read.

**Rationale**: this is the security core (FR-004, SC-004). Canonicalize-then-
prefix is the same shape as the research layer's SSRF guard (a canonicalized
address check), a known-good pattern already in the codebase.

**Alternatives**: lexical `..` stripping without canonicalization — rejected
(misses symlink escape). Allowing a symlink-following toggle — rejected (no use
case; keep the boundary absolute for v1).

## D4 — All-or-nothing resolution before any model pass

**Decision**: the assembly stage resolves **every** locator first. If any fails
(missing, empty, out of range, non-text, out of root, or the aggregate exceeds
the byte ceiling), it returns a typed error naming the offending locator and
**no model pass runs**. Only a fully-resolved evidence set proceeds to the
ensemble.

**Rationale**: FR-009 / the clarified all-or-nothing decision. A verdict over a
silently-reduced evidence set is the exact failure this feature exists to
prevent. Resolving up front also means the (costly) model passes never run on
doomed input.

**Alternatives**: best-effort with failures noted in the manifest — rejected by
the clarification.

## D5 — Completeness as a field in the same constrained pass

**Decision**: extend the pass output schema with `missing_evidence: string[]`.
Each stance-blind pass may name source classes it would have needed but wasn't
given; the server unions and de-duplicates across passes into the result's
completeness signal. No extra model hop.

**Rationale**: FR-010 in v1 scope with minimal cost — one pass set, not two. The
schema stays flat + closed (Principle II): `{ verdict, findings,
missing_evidence }` per pass; `confidence` and the manifest are server-assembled
(FR-012), never model-authored.

**Alternatives**: a dedicated completeness model call — rejected (extra
cost/latency for a P3 signal). A deterministic completeness heuristic — rejected
(naming "what's missing" is a judgment, not computable).

## D6 — Config and gating

**Decision**: three env vars, following the existing convention (malformed =
startup error, never a silent default):

- `GROUNDED_VERIFY_ROOT` (optional) — the single allowed root. **Presence
  enables the tool**; absent ⇒ not in the catalog (like `VOYAGE_API_KEY` /
  `BRAVE_API_KEY` gating).
- `GROUNDED_VERIFY_MAX_BYTES` (default `262144` = 256 KiB) — total
  assembled-evidence ceiling.
- `GROUNDED_VERIFY_MAX_LOCATORS` (default `64`) — max locators per call.

**Rationale**: Principle VI (off by default) and the established `Config::from_env`
pattern. Defaults are conservative; both ceilings are bounded `usize` with the
loud-malformed convention.

**Alternatives**: reuse `DATABASE_PATH`'s directory as an implicit root —
rejected (implicit, surprising, and not a security boundary the operator
consciously set).

## D7 — Error taxonomy mapping

**Decision**: caller-supplied bad locators (missing/empty/out-of-range/non-text/
out-of-root/over-ceiling) map to `AppError::InvalidInput` → an MCP
`invalid_params` error naming the locator. Root-not-configured is not an error
at call time — the tool simply isn't registered.

**Rationale**: consistent with the existing outcome taxonomy and FR-009's "loud,
named" requirement. `invalid_params` is the right class — the fault is in the
caller's locator set, not the server.

**Alternatives**: a new error class — rejected (the existing taxonomy covers it;
Principle VII).

## D8 — Corpus amendment (Principle I)

**Decision**: register `grounded-verify` in `NEW_SERVER_DESIGN.md` (failure-mode
catalog: "context-curation trust gap"; primitives: a Verify-family corrective)
and add a routing note in `CORRECTIVE_SELECTION.md`, in the implementing change.

**Rationale**: Principle I (NON-NEGOTIABLE) — a feature the corpus doesn't
describe must amend the corpus in the same change. Precedent: the watchdog→
checkpoint amendment (2026-06-12).

**Alternatives**: ship without amending — rejected (constitution violation).
