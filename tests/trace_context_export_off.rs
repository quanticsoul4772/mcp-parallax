//! 058 FR-007: with OTLP export off, a request's trace context changes
//! nothing.
//!
//! Its own process (each `tests/*.rs` file is one), and nothing in it
//! initializes telemetry, so export is off for every call here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::*;

use rmcp::model::RequestMetaObject;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// T012a (pin): the same result and the same stored record with and without
/// `traceparent`, export off. Fields that differ per call by design (id,
/// session, timestamp, latency) are set aside before comparing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_export_off_trace_context_changes_nothing() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(end_turn(&json!({
            "verdict": "supported", "findings": []
        }))))
        .mount(&mock)
        .await;

    let mut outcomes = Vec::new();
    for traced_call in [false, true] {
        let (client, storage, _server) = serve(&mock, 2_000).await;
        let mut params = tool_call("verify", &json!({ "claim": "export off" }));
        if traced_call {
            let mut meta = RequestMetaObject::new();
            meta.set_traceparent("00-0580000000000000000000000000012c-00f067aa0ba902b7-01");
            params.meta = Some(meta);
        }
        let result = client.call_tool(params).await.unwrap();
        let mut record = storage.list_invocations().await.unwrap().remove(0);
        record.id = String::new();
        record.session_id = String::new();
        record.created_at = chrono::DateTime::UNIX_EPOCH;
        record.latency_ms = 0;
        outcomes.push((
            result.structured_content,
            result.is_error,
            format!("{record:?}"),
        ));
    }
    assert_eq!(outcomes[0], outcomes[1]);
}
