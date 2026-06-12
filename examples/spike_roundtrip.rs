//! Spike 3 — rmcp `Json<T>` structured-output round trip (T005, research.md).
//!
//! Proves, against the pinned rmcp version: (a) a `#[tool]` returning `Json<T>`
//! advertises an `outputSchema` in the tool catalog, and (b) the call result
//! carries the value in `structured_content`. An in-process rmcp client talks
//! to the server over a tokio duplex pipe — no stdio, no network.
//!
//! Run: `cargo run --example spike_roundtrip` (no key, no network)

// Spikes are dev tooling: stdout is fine here (no MCP transport involved).
#![allow(clippy::print_stdout)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{CallToolRequestParams, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, schemars::JsonSchema)]
struct EchoParams {
    /// The text to echo back.
    text: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct EchoOutput {
    /// The echoed text.
    echoed: String,
    /// Length in characters.
    length: u32,
}

#[derive(Clone)]
struct SpikeServer {
    // Read only inside the #[tool_handler]-generated impl; rustc's dead-code
    // pass doesn't see through the macro.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SpikeServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(name = "echo", description = "Echo the input back, structured.")]
    async fn echo(&self, Parameters(p): Parameters<EchoParams>) -> Json<EchoOutput> {
        let length = u32::try_from(p.text.chars().count()).unwrap_or(u32::MAX);
        Json(EchoOutput {
            echoed: p.text,
            length,
        })
    }
}

#[tool_handler]
impl ServerHandler for SpikeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tokio::main]
async fn main() {
    // In-process transport: two ends of a duplex pipe.
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move { SpikeServer::new().serve(server_io).await });
    let client = ().serve(client_io).await.expect("client init");
    let _server = server_task.await.expect("join").expect("server init");

    // (a) The catalog advertises an outputSchema for the Json<T> tool.
    let tools = client.list_all_tools().await.expect("list tools");
    let echo = tools
        .iter()
        .find(|t| t.name == "echo")
        .expect("echo tool listed");
    let output_schema = echo
        .output_schema
        .as_ref()
        .expect("Json<T> tool must advertise an outputSchema");
    println!(
        "outputSchema: {}",
        serde_json::to_string_pretty(output_schema).unwrap()
    );
    assert!(
        serde_json::to_string(output_schema)
            .unwrap()
            .contains("echoed"),
        "outputSchema must describe the EchoOutput fields"
    );

    // (b) The result carries structured_content matching the schema.
    let mut call = CallToolRequestParams::new("echo");
    call.arguments = serde_json::json!({ "text": "parallax" })
        .as_object()
        .cloned();
    let result = client.call_tool(call).await.expect("call echo");
    let structured = result
        .structured_content
        .as_ref()
        .expect("result must carry structured_content");
    println!("structured_content: {structured}");
    assert_eq!(structured["echoed"], "parallax");
    assert_eq!(structured["length"], 8);

    client.cancel().await.expect("shutdown");
    println!(
        "\nSPIKE 3 PASS: rmcp {} emits outputSchema + structured_content for Json<T> tools.",
        rmcp_version()
    );
}

fn rmcp_version() -> &'static str {
    // Reported for the T006 pin decision.
    "1.7.x (workspace-resolved)"
}
