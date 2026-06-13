# Quickstart: `grounded-verify`

## Enable

The tool is off by default. Configure a single source root to enable it:

```bash
export GROUNDED_VERIFY_ROOT="/abs/path/to/repo"     # presence enables the tool
# optional ceilings (defaults shown):
export GROUNDED_VERIFY_MAX_BYTES=262144             # 256 KiB total evidence
export GROUNDED_VERIFY_MAX_LOCATORS=64
```

Absent `GROUNDED_VERIFY_ROOT`, `grounded_verify` is not in the catalog and no
file-read path exists. A present-but-unparseable ceiling fails startup, named.

## Call

Verify a claim against verbatim source the model cannot paraphrase:

```jsonc
// tools/call grounded_verify
{
  "claim": "publish() emits both the tracing event and the OTLP telemetry from one record value",
  "locators": [
    { "path": "src/telemetry.rs", "start_line": 105, "end_line": 122 },
    { "path": "src/server/record.rs", "start_line": 55, "end_line": 111 }
  ]
}
```

Expected shape:

```jsonc
{
  "verdict": "supported",
  "confidence": 1.0,
  "findings": ["..."],
  "missing_evidence": [],
  "manifest": { "entries": [
    { "path": "src/telemetry.rs", "start_line": 105, "end_line": 122, "bytes": 612 },
    { "path": "src/server/record.rs", "start_line": 55, "end_line": 111, "bytes": 1840 }
  ]}
}
```

## What it guarantees (and what it doesn't)

- **Guarantees** the verdict rests on the *verbatim* text of the named ranges —
  the caller cannot summarize or bias the evidence (SC-001). The manifest lets a
  reviewer reconstruct the exact evidence set (SC-002).
- **Does not** guarantee the caller named *every* relevant source. When evidence
  is omitted, `missing_evidence` names the gap (SC-006); selecting sources
  remains the caller's judgment.

## Failure is loud and all-or-nothing

A bad locator aborts the whole call with a named error — no verdict over a
partial set:

```jsonc
{ "claim": "...", "locators": [ { "path": "src/telemetry.rs" }, { "path": "src/gone.rs" } ] }
// => error: [invalid_input] source not found: src/gone.rs
```

## Acceptance

`cargo run --example acceptance_grounded_verify` exercises SC-001..006:
verbatim-flips-verdict, manifest fidelity, all-or-nothing errors, root
confinement (traversal + symlink), catalog gating when unset, and the
completeness signal over seeded omissions.
