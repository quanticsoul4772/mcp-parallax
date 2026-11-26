//! Wire tests for the TypeSafe client against wiremock (064 T009).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{TypesafeClient, MAX_IN_FLIGHT};
use crate::error::AppError;
use crate::jev::questions::research_claim_questions;
use crate::jev::redact::Redactor;
use crate::traits::classifier::{Classifier, JevAnswer, JevQuestion, JevRequest, QuestionKind};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "ts-test-key";
const MODEL: &str = "jev-1.14.0";
/// Planted in error bodies; must never reach an error or a log line.
const BODY_SENTINEL: &str = "BODY-SENTINEL-do-not-log";

fn client(mock: &MockServer) -> TypesafeClient {
    TypesafeClient::with_base_url(KEY, MODEL, &mock.uri()).unwrap()
}

fn request(timeout: Duration) -> JevRequest {
    let state = serde_json::json!({
        "claim": "The sky is blue.",
        "sources": [{ "excerpts": ["The sky appears blue."] }],
    });
    JevRequest {
        model: MODEL.into(),
        state: Redactor::new(vec![]).redact_json(&state).unwrap(),
        questions: research_claim_questions(1),
        timeout,
        retry: false,
    }
}

fn ok_body() -> serde_json::Value {
    serde_json::json!({
        "model": MODEL,
        "answers": {
            "relation_1": {
                "type": "choice",
                "choice": "supports",
                "confidence": 0.6,
                "probabilities": { "says_nothing": 0.1, "contradicts": 0.1, "supports": 0.8 },
            },
            "same_subject_1": { "type": "noul", "noul": 0.9 },
        },
        "usage": { "input_tokens": 120, "output_tokens": 3 },
    })
}

fn ok() -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(ok_body())
        .insert_header("x-typesafe-request-id", "req-123")
}

const SECOND: Duration = Duration::from_secs(5);

#[tokio::test]
async fn the_request_has_bearer_auth_the_state_as_an_object_and_options_in_order() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ok())
        .expect(1)
        .mount(&mock)
        .await;
    client(&mock).classify(&request(SECOND)).await.unwrap();

    let received = mock.received_requests().await.unwrap();
    let raw = String::from_utf8(received[0].body.clone()).unwrap();
    let body: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(body["model"], MODEL);
    assert!(body["state"].is_object(), "state sent as {}", body["state"]);
    assert_eq!(body["questions"]["same_subject_1"]["type"], "noul");
    assert_eq!(body["questions"]["relation_1"]["type"], "choice");
    let order: Vec<usize> = ["\"says_nothing\"", "\"contradicts\"", "\"supports\""]
        .iter()
        .map(|o| raw.find(o).unwrap())
        .collect();
    assert!(
        order[0] < order[1] && order[1] < order[2],
        "options out of order: {raw}"
    );
}

#[tokio::test]
async fn a_yes_no_question_sends_criteria_only_when_it_declares_them() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ok())
        .mount(&mock)
        .await;
    let noul = |id: &str, options: Vec<(String, Option<String>)>| JevQuestion {
        id: id.into(),
        version: 1,
        kind: QuestionKind::Noul,
        instructions: "Is it?".into(),
        options,
    };
    let mut req = request(SECOND);
    req.questions = vec![
        noul(
            "with",
            vec![
                ("true".into(), Some("Yes when so.".into())),
                ("false".into(), Some("No otherwise.".into())),
            ],
        ),
        noul("without", vec![]),
    ];
    // Only the request body matters here, not how the answers validate.
    let _ = client(&mock).classify(&req).await;

    let received = mock.received_requests().await.unwrap();
    let raw = String::from_utf8(received[0].body.clone()).unwrap();
    let body: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        body["questions"]["with"],
        serde_json::json!({
            "type": "noul",
            "instructions": "Is it?",
            "criteria": { "true": "Yes when so.", "false": "No otherwise." },
        })
    );
    assert!(
        raw.find("\"true\"").unwrap() < raw.find("\"false\"").unwrap(),
        "criteria out of order: {raw}"
    );
    assert_eq!(
        body["questions"]["without"],
        serde_json::json!({ "type": "noul", "instructions": "Is it?" })
    );
}

#[tokio::test]
async fn the_answering_model_answers_tokens_and_request_id_are_captured() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok())
        .mount(&mock)
        .await;
    let response = client(&mock).classify(&request(SECOND)).await.unwrap();
    assert_eq!(response.model_answered, MODEL);
    assert_eq!(response.request_id.as_deref(), Some("req-123"));
    assert_eq!(response.input_tokens, Some(120));
    let noul = response
        .answers
        .iter()
        .find(|(id, _)| id == "same_subject_1");
    assert_eq!(noul.map(|(_, a)| a), Some(&JevAnswer::Noul { p: 0.9 }));
    let choice = response.answers.iter().find(|(id, _)| id == "relation_1");
    assert!(matches!(
        choice.map(|(_, a)| a),
        Some(JevAnswer::Choice { choice, probabilities }) if choice == "supports" && probabilities.len() == 3
    ));
}

/// One status against the client: exactly one request, and the error.
async fn error_for(status: u16, body: &str) -> AppError {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(status)
                .set_body_string(body)
                .insert_header("x-typesafe-request-id", "req-err"),
        )
        .expect(1)
        .mount(&mock)
        .await;
    client(&mock).classify(&request(SECOND)).await.unwrap_err()
}

#[tokio::test]
async fn four_hundreds_other_than_429_are_provider_rejected() {
    for status in [400, 401, 413, 415, 422] {
        let err = error_for(status, &format!("{{\"detail\":\"{BODY_SENTINEL}\"}}")).await;
        assert!(
            matches!(&err, AppError::ProviderRejected { status: s, .. } if *s == status),
            "{status}: {err:?}"
        );
    }
}

#[tokio::test]
async fn max_tokens_exceeded_is_named_in_the_error() {
    let err = error_for(
        400,
        &format!("{{\"error\":\"max_tokens_exceeded\",\"message\":\"{BODY_SENTINEL}\"}}"),
    )
    .await;
    assert!(
        matches!(&err, AppError::ProviderRejected { status: 400, detail } if detail.contains("max_tokens_exceeded")),
        "{err:?}"
    );
}

#[tokio::test]
async fn rate_limit_overload_and_server_errors_are_classifier_provider_with_no_retry() {
    for status in [429, 529, 500, 503] {
        let err = error_for(status, BODY_SENTINEL).await;
        assert!(
            matches!(err, AppError::ClassifierProvider(_)),
            "{status}: {err:?}"
        );
    }
}

/// A server that answers `first` once, then `ok()`.
async fn once_then_ok(first: ResponseTemplate) -> MockServer {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(first)
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .respond_with(ok())
        .with_priority(2)
        .mount(&mock)
        .await;
    mock
}

fn retrying(timeout: Duration) -> JevRequest {
    let mut req = request(timeout);
    req.retry = true;
    req
}

#[tokio::test]
async fn a_retrying_request_waits_retry_after_then_succeeds() {
    let mock = once_then_ok(ResponseTemplate::new(429).insert_header("retry-after", "0")).await;
    client(&mock).classify(&retrying(SECOND)).await.unwrap();
    assert_eq!(mock.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_retrying_request_backs_off_without_retry_after() {
    for status in [529, 500, 503] {
        let mock = once_then_ok(ResponseTemplate::new(status)).await;
        let started = Instant::now();
        client(&mock).classify(&retrying(SECOND)).await.unwrap();
        assert_eq!(mock.received_requests().await.unwrap().len(), 2, "{status}");
        assert!(started.elapsed() >= Duration::from_millis(100), "{status}");
    }
}

#[tokio::test]
async fn retries_stop_at_the_deadline() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock)
        .await;
    let started = Instant::now();
    let err = client(&mock)
        .classify(&retrying(Duration::from_millis(350)))
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::ClassifierProvider(_)), "{err:?}");
    // 100 ms then 200 ms fit in 350 ms; the 400 ms wait does not.
    assert_eq!(mock.received_requests().await.unwrap().len(), 3);
    assert!(started.elapsed() < Duration::from_millis(600));
}

#[tokio::test]
async fn a_retry_after_past_the_deadline_is_not_waited_for() {
    let mock = once_then_ok(ResponseTemplate::new(429).insert_header("retry-after", "30")).await;
    let started = Instant::now();
    client(&mock).classify(&retrying(SECOND)).await.unwrap_err();
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn a_huge_retry_after_returns_the_error_without_panicking() {
    let mock = once_then_ok(
        ResponseTemplate::new(429).insert_header("retry-after", "18446744073709551615"),
    )
    .await;
    client(&mock).classify(&retrying(SECOND)).await.unwrap_err();
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_zero_retry_after_still_backs_off_and_stops_after_three_retries() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .mount(&mock)
        .await;
    let started = Instant::now();
    client(&mock).classify(&retrying(SECOND)).await.unwrap_err();
    assert_eq!(mock.received_requests().await.unwrap().len(), 4);
    assert!(started.elapsed() >= Duration::from_millis(700));
}

#[tokio::test]
async fn a_retrying_request_never_retries_a_401_or_other_4xx() {
    for status in [400, 401, 422] {
        let mock = once_then_ok(ResponseTemplate::new(status)).await;
        client(&mock).classify(&retrying(SECOND)).await.unwrap_err();
        assert_eq!(mock.received_requests().await.unwrap().len(), 1, "{status}");
    }
}

#[tokio::test]
async fn a_connection_failure_is_classifier_provider() {
    let address = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let client = TypesafeClient::with_base_url(KEY, MODEL, &format!("http://{address}")).unwrap();
    let err = client.classify(&request(SECOND)).await.unwrap_err();
    assert!(matches!(err, AppError::ClassifierProvider(_)), "{err:?}");
}

#[tokio::test]
async fn a_401_disables_jev_for_the_rest_of_the_process() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&mock)
        .await;
    let client = client(&mock);
    assert!(client.classify(&request(SECOND)).await.is_err());
    let second = client.classify(&request(SECOND)).await.unwrap_err();
    assert!(
        matches!(second, AppError::ProviderRejected { status: 401, .. }),
        "{second:?}"
    );
}

#[tokio::test]
async fn the_deadline_covers_the_whole_response() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok().set_delay(Duration::from_millis(500)))
        .expect(1)
        .mount(&mock)
        .await;
    let err = client(&mock)
        .classify(&request(Duration::from_millis(100)))
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Timeout { .. }), "{err:?}");
}

#[tokio::test]
async fn nothing_is_sent_when_no_time_is_left() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok())
        .expect(0)
        .mount(&mock)
        .await;
    let err = client(&mock)
        .classify(&request(Duration::ZERO))
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Timeout { .. }), "{err:?}");
}

/// A log sink the test can read back.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn error_bodies_never_reach_errors_or_logs() {
    let sink = Captured::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    for status in [400, 422, 429, 500] {
        let err = error_for(status, &format!("{{\"detail\":\"{BODY_SENTINEL}\"}}")).await;
        assert!(
            !format!("{err:?} {err}").contains(BODY_SENTINEL),
            "{status}: {err:?}"
        );
    }
    let logs = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.contains(BODY_SENTINEL), "body logged: {logs}");
}

#[tokio::test]
async fn at_most_four_requests_are_in_flight() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok().set_delay(Duration::from_millis(200)))
        .mount(&mock)
        .await;
    let client = Arc::new(client(&mock));
    let start = Instant::now();
    let calls = (0..10).map(|_| {
        let client = Arc::clone(&client);
        async move { client.classify(&request(SECOND)).await }
    });
    let results = futures::future::join_all(calls).await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(MAX_IN_FLIGHT, 4);
    // Ten 200 ms calls, four at a time, take three waves (about 600 ms);
    // unlimited they would take about 200 ms.
    assert!(
        start.elapsed() >= Duration::from_millis(550),
        "{:?}",
        start.elapsed()
    );
}

/// Review finding P: the client's Debug output never shows the key.
#[test]
fn debug_output_never_shows_the_key() {
    let key = ["ts-", "debug", "-secret-", "4242"].concat();
    let client = TypesafeClient::with_base_url(&key, MODEL, "http://127.0.0.1:9").unwrap();
    let shown = format!("{client:?}");
    assert!(!shown.contains(&key), "{shown}");
    assert!(shown.contains(MODEL), "{shown}");
}

/// Review finding K: a call that times out waiting for a request slot says so.
#[tokio::test]
async fn a_slot_timeout_is_named() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok().set_delay(Duration::from_millis(800)))
        .mount(&mock)
        .await;
    let client = Arc::new(client(&mock));
    let busy: Vec<_> = (0..MAX_IN_FLIGHT)
        .map(|_| {
            let client = Arc::clone(&client);
            tokio::spawn(async move { client.classify(&request(SECOND)).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let err = client
        .classify(&request(Duration::from_millis(100)))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            AppError::Timeout {
                what: "jev request slot",
                ..
            }
        ),
        "{err:?}"
    );
    for call in busy {
        call.await.unwrap().unwrap();
    }
}

/// Review finding O: an answer of a type the client doesn't know makes the
/// response unreadable, with that reason.
#[tokio::test]
async fn an_unknown_answer_type_is_named() {
    let mock = MockServer::start().await;
    let mut body = ok_body();
    body["answers"]["same_subject_1"] = serde_json::json!({ "type": "box", "box": {} });
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&mock)
        .await;
    let err = client(&mock).classify(&request(SECOND)).await.unwrap_err();
    assert!(
        matches!(&err, AppError::ValidationFailure(m) if m.contains("unknown type")),
        "{err:?}"
    );
}
