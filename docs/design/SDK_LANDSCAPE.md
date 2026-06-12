# SDK & Framework Landscape

**Status:** Research / decision input (2026-06-11). **Scope:** the external SDKs and
crates worth adopting for Parallax, organized by the four-layer architecture in
[`NEW_SERVER_DESIGN.md`](NEW_SERVER_DESIGN.md). **Method:** web-grounded — versions
and capabilities move fast and are version-sensitive, so every claim below is dated
and sourced (§Sources). Treat versions as "current as of June 2026," re-check before
pinning.

## The one finding that changes the architecture

**Anthropic now ships native structured outputs (GA), and Opus 4.8 supports it.**
Parallax's central bet — "constrained output is the core contract" — was written
assuming we'd *force* it via `tool_use` + `tool_choice`. That hack is no longer
needed: the API now constrains decoding to a JSON Schema directly. This deletes
`extract_json` and its fallbacks from the design *at the API level*, not just by
convention.

- **Two modes:** `output_config.format` (`type: "json_schema"`, pass a `schema`) for
  free-form JSON responses; `strict: true` on a tool object for guaranteed-valid tool
  inputs. GA — no beta header required (the legacy `output_format` + header
  `structured-outputs-2025-11-13` still works during transition).
- **Model support (GA on the Anthropic API):** Fable 5, Opus 4.8 / 4.7 / 4.6 / 4.5,
  Sonnet 4.6 / 4.5, Haiku 4.5. Parallax targets Opus 4.8 → fully covered.
- **The catch — a constrained schema subset.** This directly shapes "modes are data":
  every mode's output schema must live inside what the grammar supports.
  - **Supported:** object/array/string/integer/number/boolean/null, `enum`, `const`,
    `anyOf`/`allOf` (limited), `$ref`/`$def` (no external refs), `required`,
    `additionalProperties: false` (mandatory), string `format`
    (date-time/date/email/uri/uuid/…), array `minItems` (0 or 1 only),
    type arrays for nullable scalars (`"type": ["string","null"]` — verified
    live, feature 002).
  - **NOT supported:** recursive schemas, numeric constraints
    (`minimum`/`maximum`/`multipleOf`), string constraints (`minLength`/`maxLength`),
    richer array bounds, `additionalProperties: true`, regex lookaround.
  - **Limits:** ≤20 strict tools/request, ≤24 optional params, ≤16 union-typed params,
    180s grammar-compile timeout; grammar cached 24h (first call pays 100–300ms).
  - **Implication:** keep schemas small, flat, closed (`additionalProperties:false`),
    and push value-range/length validation into the **thin validator** kept for
    defense-in-depth (§core), since the API grammar won't enforce ranges. This is
    consistent with the design's "keep a thin schema validator" note — it now has a
    concrete job the API *can't* do.

## Summary — what to adopt, by layer

| Area | Pick | Crate / service | Notes |
|---|---|---|---|
| MCP framework | **rmcp** (official Rust SDK) | `rmcp` 1.x | already the keeper; `schemars` feature for tool schemas |
| LLM client | **native structured outputs** over a thin client | `reqwest` (thin) or `adk-anthropic` (typed, unofficial) | see core §2 tradeoff |
| Schema gen / validate | `schemars` + a validator | `schemars` 1.x; `jsonschema` or `rsonschema` | validator enforces what the API grammar can't (ranges/lengths) |
| Memory — embeddings | **Voyage 4** + **rerank-2.5** | Voyage API (`voyage-4`, `rerank-2.5`) | keeper from mcp-reasoning; 200M free tokens |
| Memory — vector store | **brute-force f32 BLOBs** (v1); sqlite-vec at scale | — | spike S1: 3 ms at 5k×1024; see §memory amendment |
| Deterministic — logic | **Z3** | `z3` 0.20 | pure binding, MSRV 1.85; SMT/SAT for the logic/constraint row |
| Deterministic — code exec | **wasmtime** in-proc; microsandbox/E2B for Python | `wasmtime`; E2B/microsandbox (opt-in) | sandbox is non-negotiable; §deterministic |
| Research — search | **Brave** (default) or **Tavily** (answers+cites) | Brave Search API / Tavily | Brave: lowest latency (~669ms); you already have the Brave MCP |
| Research — fetch/extract | **rs-trafilatura** (local) or Firecrawl (managed) | `rs-trafilatura` / `article_scraper` | local extraction avoids a second API |
| Observability | **OpenTelemetry + OTLP** | `opentelemetry`, `opentelemetry-otlp`, `tracing-opentelemetry` | GenAI semantic conventions exist; feeds the watchdog/dashboard |

---

## Core

### 1. MCP framework — `rmcp` (official Rust SDK)

Already the keeper. `rmcp` reached 1.0 in March 2026 and is at 1.x (mcp-reasoning
pins 1.7); it's the official `modelcontextprotocol/rust-sdk` (upstream `4t145/rmcp`),
~4.7M downloads. Procedural macros (`#[tool]`, `#[tool_router]`, `#[tool_handler]`)
generate tool plumbing from Rust types; the optional `schemars` feature generates
JSON Schema for tool definitions. **Open item:** confirm rmcp's support for MCP
*structured tool output* (`outputSchema` + `structuredContent`) in 1.x — the docs
excerpt didn't show it explicitly. If present, Parallax's tools return typed
structured content end-to-end (MCP side) on top of Anthropic structured outputs
(model side) — schema-guaranteed at both hops.

### 2. LLM client — native structured outputs over a thin client

The decision is *how* to call the structured-outputs API, not *whether*.

- **Option A — thin `reqwest` client (mcp-reasoning's earned pattern).** Full control,
  no unofficial dependency, target exactly the `output_config.format` / `strict` API.
  Cost: we maintain the request/response types and retry/backoff (already a solved,
  copyable pattern from mcp-reasoning's `anthropic/`).
- **Option B — a typed community SDK.** `adk-anthropic` advertises "full March 2026
  API parity: adaptive thinking, effort parameter, **structured outputs**, context
  management, fast mode, citations, Files/Skills/Models APIs." `anthropic-sdk-rust`
  claims TS-SDK parity. Upside: less boilerplate, tracks new API features. **Risk:**
  all are **unofficial / community-maintained** — no Anthropic-backed Rust SDK exists.
  A core dependency on an unmaintained crate is the exact brittleness the new design
  is trying to shed.
- **Recommendation:** start with **A (thin client)** for the hot path — it's small,
  controlled, and the structured-outputs API is simple. Evaluate `adk-anthropic`'s
  maintenance health before taking it on for breadth (thinking/effort/citations);
  adopt only if actively maintained. Either way, the *contract* is native structured
  outputs.

### 3. Schema tooling — `schemars` + a validator

`schemars` 1.x derives JSON Schema from Rust types (mode output types → schema, fed to
both rmcp tool defs and the Anthropic `output_config`). For the **defense-in-depth
validator**, `jsonschema` (full Draft 2020-12, mature) or `rsonschema` (2020-12-only,
perf-tuned). Its real job: enforce the constraints the Anthropic grammar **drops**
(numeric ranges, string lengths, richer array bounds) — so the validator isn't
redundant with the API, it covers the API's blind spot.

---

## Memory / experience layer

### Embeddings + rerank — Voyage 4 + rerank-2.5 (keeper)

Voyage 4 family (Jan 2026): `voyage-4-large` (most accurate, supersedes
voyage-3-large), `voyage-4`, `voyage-4-lite`, `voyage-4-nano` (open-weights on HF).
**Shared embedding space across the series → no re-index when switching models**
(directly fixes the model-drift cache-invalidation pain noted in mcp-reasoning's
`voyage-model-env-drift`). Rerankers: `rerank-2.5` / `rerank-2.5-lite`. First 200M
tokens free. Keep mcp-reasoning's thin Voyage client pattern. **The reranker also
doubles as a watchdog signal** (relevance scoring for retrieved memories) — see below.

### Vector store — sqlite-vec (single-store), LanceDB if it outgrows it

- **sqlite-vec** — pure-C, zero-dep SQLite extension; keeps the memory store *inside
  the same SQLite file* as sessions (the SQLite keeper), so no second datastore.
  **Caveat (real):** `sqlx` doesn't expose `sqlite3_auto_extension`; it offers
  `.extension()` / `.extension_with_entrypoint()`. Loading sqlite-vec under sqlx is a
  known rough edge — either load via those, or use `rusqlite` + `sqlite3_auto_extension`
  for the vector path. Resolve this in a spike before committing.
- **LanceDB** — embedded, Rust-native, columnar (Lance), in-process zero-copy; the
  purpose-built embedded option if recall volume outgrows sqlite-vec. Cost: a separate
  store/format alongside SQLite.
- **Qdrant** — Rust-native server (and "Qdrant Edge" in-process); overkill for a
  single-binary dev tool unless it goes multi-tenant.
- **Pick (amended 2026-06-12, feature 003 spike S1):** **brute-force in-process
  cosine over f32 BLOBs** for v1 — no vector extension at all. The spike resolved
  the loading caveat the hard way: every sqlite-vec registration route costs
  either an `unsafe extern` call (the crate forbids unsafe; isolating it means a
  workspace split) or shipping per-platform loadable binaries, while brute force
  at v1 scale measured **3 ms for 5k × 1024-dim vectors** (bit-exact BLOB
  round-trip through the sqlx pool). **sqlite-vec is now the first scale step**
  (revisit near ~50k memories or when recall latency data says so); LanceDB
  remains the step after.

---

## Deterministic / symbolic layer

This is where the SDK choices are hardest, because **executing model-generated code is
non-negotiable to sandbox** (design §safety) and Rust's in-process options can't run
arbitrary Python.

- **Logic / constraints — `z3` 0.20** (prove-rs/z3.rs). High-level Rust bindings over
  Z3, MSRV 1.85, vendored build via the `bundled` feature. Pure in-process, no sandbox
  needed (it's a solver, not arbitrary code) — the cleanest deterministic win and the
  right *first* engine for the logic/constraint and "checkable-ness" rows.
  **Validated (005, 2026-06-12):** clean bundled build is ~5 min (requires cmake —
  on Windows the VS Build Tools' bundled cmake via the `CMAKE` env var works; CI
  runners ship it). Z3 parses SMT-LIB scripts atomically, so an assertion-count
  check detects parse failures without unsafe FFI. Z3's Debug C++ build opens
  `.z3-trace` in the process cwd from a global initializer — `z3-sys` is forced
  to `opt-level = 3` in every cargo profile so `_TRACE` compiles out.
- **Schema/format checks** — the `jsonschema`/`rsonschema` validator above; in-process,
  trivial, already present.
- **Arithmetic / units** — a pure-Rust evaluator covers the bulk of quantitative
  checks without a code sandbox. **Pick (amended 2026-06-12, feature 005):**
  `evalexpr` 13 — the original "`meval`/`fend`-style" wording was proven wrong at
  implementation: `meval` 0.2 is unmaintained and numeric-only (no boolean
  comparisons, so a claim-as-expression target cannot work) and `fend-core` is
  string-in/string-out without boolean results; `evalexpr` natively evaluates
  comparisons into a typed boolean. Its dialect is strict (`^` yields floats,
  int never equals float, `/` on integers is integer division) — the
  translation prompt embeds the whitelist + type rules, and the engine rejects
  exact `==`/`!=` over float-producing arithmetic (tolerance forms enforced).
  `fend-core` remains the upgrade path for the units/dates row.
- **Arbitrary code (PAL-style) — the sandbox question.** Running model-written Python
  is what needs isolation. Options, by isolation strength:
  - **wasmtime** — in-process WASM, memory-isolated by design, runtime formally
    verified for memory safety; low-friction and embeddable. Limitation: running
    *Python* in WASM (pyodide/componentize-py) is heavy; wasmtime shines for
    Rust/WASM-native checks, not drop-in CPython.
  - **microsandbox** (self-host) / **E2B** (managed Firecracker microVM, ~150ms warm
    start) — real microVM isolation for untrusted Python; the 2026 consensus is that
    plain Docker/runc is *insufficient* for untrusted agent code.
  - **Recommendation:** ship the **solver + validator + arithmetic** deterministic
    checks first (no code sandbox required, immediate value). Gate **arbitrary-code
    execution behind an optional, off-by-default sandbox integration** (E2B or
    microsandbox), exactly like every other capability is env-gated. Never run
    generated code in-process unsandboxed.

---

## Research primitive (knowledge / grounding)

Maps directly to [`RESEARCH_PRIMITIVE.md`](RESEARCH_PRIMITIVE.md)'s search→fetch→verify
pipeline.

- **Search provider (pluggable behind a trait — the spec already calls for this):**
  - **Brave Search API** — top agent score and **lowest latency (~669ms)** in 2026
    benchmarks; matters because the pipeline fans out N searches. You already have the
    Brave MCP configured locally.
  - **Tavily** — returns answers-with-citations rather than links; clean for LLM
    consumption, but Advanced/Research tiers can take 5s+.
  - **Exa** — highest relevance / best deep full-page retrieval.
  - **Pick:** **Brave** as the default (latency + you have it), behind the spec's
    provider trait so Tavily/Exa drop in. The benchmarked 2026 default is "Brave +
    an extractor."
- **Fetch / extract:**
  - **rs-trafilatura** — Rust port of Trafilatura; classifies page type (article /
    forum / product / docs / …) and applies type-specific extraction. Local, no API.
  - **article_scraper** / **readability-rust** — Mozilla Readability-style extraction
    (Reader Mode algorithm), local fallback.
  - **Firecrawl** — managed extract API, top-tier quality, but a second paid dependency.
  - **Pick:** **rs-trafilatura** (local, no extra API, page-type-aware) for the extract
    layer; Firecrawl only if local quality proves insufficient.
  - **Validated (004, 2026-06-12):** rs-trafilatura 0.2.2 passed the fixture spike
    (main text extracted, boilerplate excluded; its DEBUG diagnostics are stderr-only
    and debug_assertions-gated — stdout stays protocol-clean). `robotstxt` 0.3
    (Google-parser port) joined the stack for robots.txt enforcement. The Firecrawl
    fallback was not needed.

---

## Watchdog & observability

- **OpenTelemetry (Rust) + OTLP** — `opentelemetry`, `opentelemetry-otlp`,
  `tracing-opentelemetry` (bridges the existing `tracing` to OTel), and
  `opentelemetry-appender-tracing`. **GenAI semantic conventions exist as of 2026**
  (`gen_ai.request.model`, `gen_ai.usage.input_tokens`/`output_tokens`,
  `gen_ai.response.finish_reasons`, message attributes) — so token/model/cost/latency
  are *standard span attributes*, satisfying the design's "observability designed in
  from the first commit" with an industry schema rather than a bespoke event.
- **Why it matters for the watchdog:** the design had the watchdog consuming the same
  activity/event stream the dashboard emits. *Amended 2026-06-12:* that in-process
  stream did not survive MCP — the watchdog's event feed is now the client's hook
  system (see the MCP-reality amendment in `WATCHDOG_LAYER.md`); OTLP remains the
  *export* story (per-checkpoint trace events for catch-rate vs noise measurement),
  not the watchdog's input.
- **NLI / contradiction signal (watchdog):** no clean Rust NLI SDK; the cheap path is
  to reuse **Voyage rerank-2.5** as a relevance/consistency scorer and reserve an LLM
  call (structured output) for the expensive contradiction check — matching the design's
  "cheap signals gate the expensive judge." A local cross-encoder via `candle`/`ort` is
  a later option if the LLM-call cost is too high. *006 implementation note:* the
  shipped checkpoint layer uses no new crates at all — lexical overlap + polarity cues
  mine candidates, existing voyage-4 cosine supplies relevance (query-embed p95
  measured 165 ms, 006 S2), and one decline-biased structured-output hop classifies.

---

## Sequencing — adopt-now vs later

Tied to the scaffold → primitives path (the scaffold's `ModelClient`/`Storage`/`Clock`
traits are already the seams these slot behind):

1. **Now (core, unblocks the first primitive):** `rmcp` wiring; the thin Anthropic
   client using **native structured outputs** (`output_config.format`); `schemars` +
   validator. This makes `ModelClient::complete(prompt, schema)` real.
2. **Memory (next):** Voyage 4 client; spike **sqlite-vec under sqlx** (resolve the
   loading caveat) before building recall.
3. **Research primitive:** Brave provider behind the trait + rs-trafilatura extract.
   **Done (004)** — plus `robotstxt`; named narrowings recorded in the feature's
   research.md and RESEARCH_PRIMITIVE.md's status note.
4. **Deterministic:** `z3` + validator + arithmetic first (no sandbox); the optional
   sandboxed code-exec integration later, off by default.
5. **Observability/watchdog:** OTLP from the first server commit (cheap, and the
   watchdog depends on it).

## Open questions / risks

- **rmcp structured tool output:** confirm `outputSchema`/`structuredContent` support in
  1.x (the doc didn't show it). Affects whether MCP-side output is schema-typed too.
- **No official Anthropic Rust SDK:** every typed option is community-maintained. Thin
  client avoids the dependency risk; re-evaluate `adk-anthropic` health if we want its
  breadth (thinking/effort/citations).
- **Structured-output schema subset:** ranges/lengths/recursion unsupported → the
  validator must cover them, and mode schemas must be designed flat + closed. Recursive
  reasoning structures (graph/tree) can't be expressed as one recursive schema — flatten
  or paginate.
- **sqlite-vec + sqlx loading:** a known rough edge; spike it or use rusqlite for the
  vector path.
- **Sandbox cost/ops:** real microVM isolation (E2B/microsandbox) adds latency and infra;
  keep arbitrary-code-exec optional and off by default, ship solver/validator checks
  first.
- **Search/extract API cost:** Brave/Tavily/Firecrawl are paid; gate egress off by
  default (the Research spec already does), prefer local extraction (rs-trafilatura).

## Sources

- MCP Rust SDK: [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk),
  [rmcp on docs.rs](https://docs.rs/rmcp/latest/rmcp/), [4t145/rmcp](https://github.com/4t145/rmcp)
- Anthropic structured outputs: [Claude API docs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs),
  [Anthropic GA announcement (Tessl)](https://tessl.io/blog/anthropic-brings-structured-outputs-to-claude-developer-platform-making-api-responses-more-reliable/),
  [TDS hands-on guide](https://towardsdatascience.com/hands-on-with-anthropics-new-structured-output-capabilities/)
- Anthropic Rust crates: [anthropic-sdk-rust](https://crates.io/crates/anthropic-sdk-rust),
  [adk-anthropic](https://crates.io/crates/adk-anthropic), [anthropic_rust](https://lib.rs/crates/anthropic_rust)
- Voyage 4: [Voyage 4 family](https://blog.voyageai.com/2026/01/15/voyage-4/),
  [Voyage models overview (MongoDB)](https://www.mongodb.com/docs/voyageai/models/),
  [rerankers](https://docs.voyageai.com/docs/reranker)
- Vector stores: [sqlite-vec](https://github.com/asg017/sqlite-vec),
  [sqlite-vec in Rust](https://alexgarcia.xyz/sqlite-vec/rust.html),
  [sqlx loading issue #198](https://github.com/asg017/sqlite-vec/issues/198),
  [LanceDB / Qdrant benchmarks 2026](https://callsphere.ai/blog/vector-database-benchmarks-2026-pgvector-qdrant-weaviate-milvus-lancedb)
- Sandboxing: [4 ways to sandbox untrusted code 2026](https://dev.to/mohameddiallo/4-ways-to-sandbox-untrusted-code-in-2026-1ffb),
  [AI agent sandboxing guide](https://manveerc.substack.com/p/ai-agent-sandboxing-guide),
  [E2B alternatives (Northflank)](https://northflank.com/blog/best-alternatives-to-e2b-dev-for-running-untrusted-code-in-secure-sandboxes),
  [SandboxEval](https://arxiv.org/pdf/2504.00018)
- Z3: [prove-rs/z3.rs](https://github.com/prove-rs/z3.rs), [z3 on crates.io](https://crates.io/crates/z3)
- Search APIs: [Agentic Search benchmark 2026 (AIMultiple)](https://aimultiple.com/agentic-search),
  [Brave best search API 2026](https://brave.com/learn/best-search-api-2026/),
  [Tavily/Exa/Serper comparison](https://nomadlab.cc/blog/2026/05/best-ai-search-apis-2026-tavily-exa-serper-firecrawl)
- Extraction: [rs-trafilatura](https://dev.to/murroughfoley/rs-trafilatura-page-type-aware-web-content-extraction-in-rust-2ppf),
  [article_scraper](https://crates.io/crates/article_scraper), [readability-rust](https://crates.io/crates/readability-rust)
- Schema validation: [jsonschema](https://docs.rs/jsonschema), [rsonschema](https://lib.rs/crates/rsonschema)
- Observability: [opentelemetry-rust](https://github.com/open-telemetry/opentelemetry-rust),
  [opentelemetry-otlp](https://docs.rs/opentelemetry-otlp/latest/opentelemetry_otlp/),
  [Rust LLM observability with OTel (base14)](https://docs.base14.io/guides/ai-observability/rust-llm-observability/),
  [GenAI observability with OTel](https://opentelemetry.io/blog/2026/genai-observability/)
