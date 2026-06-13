# Data Model: Source-Grounded Verification

Entities are in-memory (no new persistence). The only stored artifact is the
existing `InvocationRecord`, unchanged.

## SourceLocator (input)

A caller-supplied reference to evidence, interpreted within the configured root.

| Field | Type | Notes |
|---|---|---|
| `path` | string | Relative path within the root. Required. |
| `start_line` | integer? | 1-based inclusive start. Optional; with `end_line` forms a range. |
| `end_line` | integer? | 1-based inclusive end. Optional. |

Validation:

- `path` non-empty; resolves (canonicalized) **inside** the configured root — else `InvalidInput`.
- If `start_line`/`end_line` present: `1 ≤ start_line ≤ end_line`; `start_line ≤` file line count — else `InvalidInput` (out of range).
- Both omitted ⇒ the whole file.
- A glob in `path` is **not** interpreted in v1 (deferred); a literal path is expected.

## AssembledEvidence (internal)

Produced by the assembly stage; the only context (besides the claim) the passes see.

| Field | Type | Notes |
|---|---|---|
| `spans` | list of (locator, text, byte_len) | In caller-resolution order; each tagged with provenance. |
| `total_bytes` | integer | Sum of span byte lengths; ≤ `GROUNDED_VERIFY_MAX_BYTES`. |

Rules:

- **All-or-nothing**: assembled only if *every* locator resolves; a single failure aborts with a named `InvalidInput` and no pass runs.
- `total_bytes > GROUNDED_VERIFY_MAX_BYTES` ⇒ `InvalidInput` naming the overflow.
- Locator count `> GROUNDED_VERIFY_MAX_LOCATORS` ⇒ `InvalidInput`.
- Text-only: a span whose bytes are not valid UTF-8 text ⇒ `InvalidInput` (non-text).

## EvidenceManifest (output, server-assembled)

The audit surface — deterministic, never model-authored (FR-012, FR-008).

| Field | Type | Notes |
|---|---|---|
| `entries` | list of manifest entry | One per resolved span, in resolution order. |

Manifest entry: `{ path: string, start_line: int?, end_line: int?, bytes: int }`.

## Pass output (model-authored, per stance-blind pass)

The constrained-output schema for each ensemble pass — **flat and closed**
(`additionalProperties: false`), verify's schema plus `missing_evidence`.

| Field | Type | Notes |
|---|---|---|
| `verdict` | enum `supported` \| `refuted` | The pass's call. |
| `findings` | string[] | Concrete findings; every refutation names the specific gap. |
| `missing_evidence` | string[] | Source classes the pass would have needed but wasn't given; empty when evidence suffices. |

Numeric/length bounds (array caps) are enforced by the local validator, not the
API grammar (per the core contract). The model authors **only** these three
fields.

## GroundedVerdict (result, server-assembled)

The structured tool result returned to the caller.

| Field | Type | Source |
|---|---|---|
| `verdict` | enum `supported` \| `refuted` | Majority across passes (server). |
| `findings` | string[] | Collected across passes (server). |
| `confidence` | number `0.0..=1.0` | Cross-pass agreement (server) — identical to `verify`. |
| `missing_evidence` | string[] | Union/dedup of pass `missing_evidence` (server). |
| `manifest` | EvidenceManifest | Assembled from the reader results (server). |

## Configuration (Config::from_env additions)

| Field | Env | Type | Default | Rule |
|---|---|---|---|---|
| `grounded_verify_root` | `GROUNDED_VERIFY_ROOT` | `Option<String>` | `None` (off) | Presence enables the tool; canonicalized once at startup, must exist as a directory. |
| `grounded_verify_max_bytes` | `GROUNDED_VERIFY_MAX_BYTES` | `usize` | `262144` | Present-but-unparseable ⇒ startup error. |
| `grounded_verify_max_locators` | `GROUNDED_VERIFY_MAX_LOCATORS` | `usize` | `64` | Present-but-unparseable ⇒ startup error. |

## InvocationRecord (unchanged)

One record per `grounded_verify` call (tool=`grounded_verify`, model, tokens,
cost, latency, outcome), exported via OTLP when telemetry is configured — via
the existing `publish()` path. No new fields, no new table.
