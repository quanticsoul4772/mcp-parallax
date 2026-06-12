# SDK Usage — Core Layer (rmcp + Anthropic structured outputs)

**Status:** Usage / integration research (2026-06-11). **Parent:**
[`SDK_LANDSCAPE.md`](SDK_LANDSCAPE.md) (which SDKs). **This doc:** *how to wire the two
core SDKs* — the MCP server framework (`rmcp`) and the Anthropic client (native
structured outputs) — into the scaffold's trait seams. Code is illustrative (current
APIs as of June 2026), not final. Verify against the pinned crate versions when wiring.

## The one insight that ties it together

`schemars` derives **one** JSON Schema from a Rust output type, and that single schema
feeds **both hops**:

```
            ┌── rmcp tool `outputSchema`  (MCP client ← server, structured_content)
#[derive(JsonSchema)] OutputType
            └── Anthropic `output_config.format.schema`  (server → model, constrained decode)
```

So a mode is: one input type + one output type, both `JsonSchema`, and the schema is
generated once and reused at both boundaries. This is the concrete form of "modes are
data" and "constrained output is the core contract."

---

## Part 1 — rmcp (the MCP server)

### Deps

```toml
rmcp = { version = "1", features = ["server", "macros", "transport-io"] }
schemars = "1"
serde = { version = "1", features = ["derive"] }
```

(`transport-io` = stdio. mcp-reasoning pins `rmcp` 1.7 with these same features — a
safe starting pin.)

### A tool with structured output

The pattern: `Parameters(InputType)` for the input, return `Json<OutputType>` for
structured output. Both types derive `schemars::JsonSchema`; rmcp derives the tool's
`inputSchema` **and** `outputSchema` from them automatically, and `Json<T>` places the
value in the MCP result's `structured_content` field (not just `content` text).

```rust
use rmcp::{tool, tool_router, handler::server::wrapper::{Parameters, Json}, schemars};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, schemars::JsonSchema)]
struct VerifyParams { claim: String }

#[derive(Serialize, schemars::JsonSchema)]
struct VerifyOutput { verdict: String, confidence: f64 }

#[derive(Clone)]
struct Parallax { /* deps behind the scaffold traits */ }

#[tool_router(server_handler)] // `server_handler` auto-generates #[tool_handler]
impl Parallax {
    #[tool(name = "verify", description = "Independently verify a claim …")]
    async fn verify(&self, Parameters(p): Parameters<VerifyParams>) -> Json<VerifyOutput> {
        // call the model client (Part 2) with VerifyOutput's schema, return Json(out)
        Json(VerifyOutput { verdict: "…".into(), confidence: 0.0 })
    }
}
```

- `Json<T>`'s `IntoCallToolResult` calls `CallToolResult::structured(value)`, which sets
  both `structured_content: Some(value)` and a `content` text fallback (older clients).
- For a hand-built/raw schema, `Tool::with_raw_output_schema(Arc<JsonObject>)` sets the
  output schema directly — useful if a mode's schema is data, not a Rust type.

### Server wiring + stdio

`#[tool_router(server_handler)]` generates the `ServerHandler` glue; implement
`get_info()` for capabilities, then serve over stdio. Logs go to stderr (the scaffold
already configures this); stdout stays the JSON-RPC channel.

```rust
use rmcp::{ServiceExt, transport::stdio, ServerHandler, model::*};

impl ServerHandler for Parallax {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

// in main (async):
let service = Parallax::new(/* … */).serve(stdio()).await?;
service.waiting().await?;
```

### Progress notifications

A long primitive (Research, Search) streams progress via the request context's peer —
`Peer::notify_progress(ProgressNotificationParam)` (async, `Result<(), ServiceError>`).
This is the same milestone stream the dashboard/watchdog consume (design §observability).

### Errors

Tool methods can return `Result<Json<T>, ErrorData>`. Build errors with
`ErrorData::internal_error(msg, None)` (and friends). `ErrorData` is re-exported as
`McpError`. Keep production paths `Result`-based — no `unwrap`/`expect`.

### Open item (flagged in the landscape, now narrowed)

rmcp **does** support `outputSchema` + `structured_content` (via `Json<T>` /
`CallToolResult::structured`) — the earlier open question is **resolved: yes**. Remaining
check: confirm the exact `rmcp` 1.x minor that introduced `Json<T>` and pin to it.

---

## Part 2 — Anthropic native structured outputs (the model client)

No official Anthropic Rust SDK exists (all are community-maintained), so the core path
is a **thin `reqwest` client** that targets the structured-outputs API directly — the
implementation of the scaffold's `ModelClient::complete(prompt, schema)`.

### Request shape (JSON Outputs mode)

Place `output_config.format` on the Messages request:

```jsonc
{
  "model": "claude-opus-4-8",
  "max_tokens": 1024,
  "messages": [{ "role": "user", "content": "…the prompt…" }],
  "output_config": {
    "format": {
      "type": "json_schema",
      "schema": { /* the mode's output schema — flat, closed */ }
    }
  }
}
```

### Response handling

The constrained JSON lands as a **string** in `content[0].text` — parse it yourself
(unlike strict tools, where `content[0].input` is already an object). **Check
`stop_reason` before trusting the body:**

| `stop_reason` | Meaning | Action |
|---|---|---|
| `end_turn` | normal | ✅ JSON matches schema — parse it |
| `tool_use` | a strict tool was called | read `content[].input` (already parsed) |
| `max_tokens` | truncated | ❌ JSON likely invalid — error/retry, don't parse-and-pray |
| `refusal` | safety refusal (200 OK, billed) | ❌ body won't match schema — surface as a refusal, not a parse failure |

```rust
// ModelClient::complete sketch (thin reqwest)
async fn complete(&self, prompt: &str, schema: &Value) -> Result<Value, AppError> {
    let body = json!({
        "model": self.model,
        "max_tokens": self.max_tokens,
        "messages": [{ "role": "user", "content": prompt }],
        "output_config": { "format": { "type": "json_schema", "schema": schema } },
    });
    let resp: MessagesResponse = self.post("/v1/messages", body).await?;
    match resp.stop_reason.as_deref() {
        Some("end_turn") => {
            let text = resp.first_text().ok_or_else(|| AppError::Client("no text".into()))?;
            serde_json::from_str(&text).map_err(|e| AppError::Client(format!("parse: {e}")))
        }
        Some("refusal") => Err(AppError::Client("model refused".into())),
        Some("max_tokens") => Err(AppError::Client("truncated before schema complete".into())),
        other => Err(AppError::Client(format!("unexpected stop_reason: {other:?}"))),
    }
}
```

### Strict-tool alternative

For cases that map to a tool call rather than a free JSON response, put `strict: true`
plus `input_schema` on the tool; the model returns `content[].type == "tool_use"` with an
**already-parsed** `input` and `stop_reason == "tool_use"`. Either mode satisfies the
constrained-output contract; pick per primitive.

### The integration gotcha that needs a sanitizer

`schemars` emits a full JSON Schema, but Anthropic's grammar supports a **subset** and
**rejects/ignores** the rest. A `schemars` type can produce keywords the API doesn't
accept (`minimum`/`maximum`/`minLength`/`maxLength`, `$schema`, `title`, draft-specific
constructs), and by default `schemars` does **not** emit `additionalProperties: false`
(the API **requires** it). So between "derive schema" and "send to Anthropic" there must
be a **schema sanitizer**:

- strip unsupported constraint keywords (keep them for the local validator — that's its
  job, §below),
- force `additionalProperties: false` on every object,
- ensure `required` lists every property the model must emit,
- drop `$schema`/`title`/`description`-only noise the grammar doesn't use.

This is exactly what Anthropic's *own* Python/TS SDKs do silently ("automatically
transform schemas by removing unsupported constraints"); hand-rolling means we own that
transform. **Spike it early** — it's the load-bearing glue.

### Where the validator earns its place

The constraints the sanitizer **strips** (ranges, lengths) are precisely what the
defense-in-depth validator (`jsonschema`/`rsonschema`, §`SDK_LANDSCAPE.md`) re-checks on
the returned JSON. So the full validity story is: **API grammar guarantees shape;
local validator guarantees the value constraints the grammar can't.** Neither is
redundant.

### Other caveats

- **Grammar cache:** first request with a new schema pays ~100–300ms (grammar compile),
  cached 24h. Stable schemas (modes-as-data) benefit; don't churn schemas per call.
- **Limits:** ≤20 strict tools/request, ≤24 optional params, ≤16 union-typed params,
  180s compile timeout. Keep mode schemas small and flat.
- **Extended thinking compatibility:** **verified 2026-06-11** (spike 4,
  `examples/spike_thinking.rs`, live against Opus 4.8): adaptive thinking
  (`thinking: {type: "adaptive"}` + `output_config.effort`) **composes** with
  `output_config.format` — `stop_reason: end_turn`, schema-valid JSON in the text
  block after the thinking block(s). The legacy `thinking: {type: "enabled",
  budget_tokens}` shape is **rejected** by Opus 4.8 with a 400 ("use
  thinking.type.adaptive and output_config.effort"). When reading the response
  with thinking on, find the `text` block — it is not necessarily `content[0]`.
- **Retry/backoff/timeouts:** copy mcp-reasoning's `anthropic/client.rs` pattern (it's a
  solved, lift-and-shift problem); the structured-outputs change is only the request
  body + response parsing above.

---

## How the first tool call flows end-to-end

```
client → rmcp (stdio) → #[tool] verify(Parameters<VerifyParams>)
       → ModelClient::complete(prompt, VerifyOutput-schema)         [Part 2]
       → reqwest POST /v1/messages with output_config.format
       → Anthropic constrains decode → content[0].text = JSON string
       → check stop_reason → parse → validate (ranges) → VerifyOutput
       → return Json(VerifyOutput) → rmcp structured_content        [Part 1]
       → client receives schema-typed result
```

One `schemars` type defines the schema used at both the rmcp boundary and the Anthropic
boundary. That is the whole core contract, concretely.

## What to spike before building the core (small, ordered)

1. **Schema sanitizer** `schemars` → Anthropic-subset (additionalProperties:false; strip
   ranges/lengths/$schema/title). Load-bearing; everything else depends on it.
2. **Thin client happy path** — one real `complete()` call against Opus 4.8 with a tiny
   schema; confirm `content[0].text` parses and `stop_reason` handling.
3. **rmcp `Json<T>` round-trip** — one `verify` tool returning structured_content, hit
   from an in-process rmcp test client; confirm `outputSchema` is emitted.
4. **Thinking + structured output** — confirm they compose (or document that they don't).

## Sources

- rmcp: [docs.rs/rmcp](https://docs.rs/rmcp/latest/rmcp/),
  [official rust-sdk](https://github.com/modelcontextprotocol/rust-sdk),
  `Json<T>` wrapper + `CallToolResult::structured` + `Tool::with_raw_output_schema`
  (docs.rs source), [MCP outputSchema RFC #356](https://github.com/modelcontextprotocol/modelcontextprotocol/pull/356)
- Anthropic structured outputs: [Claude API docs — structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs)
  (request/response shapes, stop_reason table, schema-subset limits, SDK schema-transform note)
