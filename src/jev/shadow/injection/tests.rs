//! Tests for the injection screen (065 T011).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{screen_page, InjectionJev, Screening};
use crate::error::{AppError, Outcome};
use crate::jev::redact::Redactor;
use crate::storage::jev::JevOutcome;
use crate::traits::classifier::{
    Classifier, JevAnswer, JevMode, JevRequest, JevResponse, JevSite, MockClassifier,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MODEL: &str = "jev-1.14.0";
const SECOND: Duration = Duration::from_secs(1);

fn jev(classifier: Arc<dyn Classifier>, mode: JevMode) -> InjectionJev {
    InjectionJev {
        classifier,
        redactor: Redactor::new(vec![]),
        model: MODEL.into(),
        server_session_id: "proc".into(),
        mode,
        threshold: Some(0.35),
        deadline: Duration::from_millis(2000),
    }
}

/// A classifier answering `p` from `model`, keeping every request it saw.
fn answering(p: f64, model: &str) -> (Arc<dyn Classifier>, Arc<Mutex<Vec<JevRequest>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let model = model.to_string();
    let mut mock = MockClassifier::new();
    mock.expect_classify().returning(move |req| {
        log.lock().unwrap().push(req.clone());
        Ok(JevResponse {
            model_answered: model.clone(),
            answers: vec![("directs_ai".into(), JevAnswer::Noul { p })],
            input_tokens: Some(1500),
            request_id: Some("req-1".into()),
            latency_ms: 120,
        })
    });
    (Arc::new(mock), seen)
}

fn failing(error: fn() -> AppError) -> Arc<dyn Classifier> {
    let mut mock = MockClassifier::new();
    mock.expect_classify().returning(move |_| Err(error()));
    Arc::new(mock)
}

async fn screen(jev: &InjectionJev) -> (crate::storage::jev::JevRecord, Screening) {
    screen_page(
        jev,
        "s1",
        "https://a.example/page",
        "Some page text.",
        &["role_marker"],
        SECOND,
        chrono::Utc::now(),
    )
    .await
}

#[tokio::test]
async fn the_request_carries_the_page_the_question_and_the_retry_flag() {
    for (mode, retry) in [(JevMode::Shadow, false), (JevMode::On, true)] {
        let (classifier, seen) = answering(0.1, MODEL);
        screen(&jev(classifier, mode)).await;
        let req = seen.lock().unwrap().pop().unwrap();
        let state = req.state.as_json().unwrap();
        assert_eq!(state["page_url"], "https://a.example/page");
        assert_eq!(state["page_text"], "Some page text.");
        assert_eq!(req.model, MODEL);
        assert_eq!(
            req.questions,
            vec![crate::jev::questions::injection_question()]
        );
        assert_eq!(req.retry, retry, "{mode:?}");
    }
}

#[tokio::test]
async fn a_high_score_warns_and_only_an_on_row_is_applied() {
    for (mode, applied) in [(JevMode::Shadow, false), (JevMode::On, true)] {
        let (classifier, _) = answering(0.9, MODEL);
        let (row, screening) = screen(&jev(classifier, mode)).await;
        assert_eq!(
            screening,
            Screening {
                screened: true,
                probability: Some(0.9),
                warned: true,
                reason: None,
            }
        );
        assert_eq!(row.site, JevSite::ResearchInjection);
        assert_eq!(row.mode, mode);
        assert_eq!(row.applied, applied, "{mode:?}");
        assert_eq!(row.source_ids, Some(vec!["s1".to_string()]));
        assert_eq!(row.questions_json[0]["band"], "warned");
        assert_eq!(
            row.label_json,
            Some(serde_json::json!({ "patterns063": ["role_marker"] }))
        );
        assert!(row.state_json.is_none());
    }
}

#[tokio::test]
async fn a_low_score_is_screened_and_clear() {
    let (classifier, _) = answering(0.1, MODEL);
    let (row, screening) = screen(&jev(classifier, JevMode::On)).await;
    assert!(screening.screened && !screening.warned);
    assert_eq!(screening.probability, Some(0.1));
    assert!(!row.applied);
    assert_eq!(row.questions_json[0]["band"], "clear");
}

#[tokio::test]
async fn every_failure_gives_no_warning_and_names_its_reason() {
    let cases: [(Arc<dyn Classifier>, &str, Outcome); 4] = [
        (
            failing(|| AppError::ClassifierProvider("typesafe 529".into())),
            "classifier_provider",
            Outcome::ClassifierProvider,
        ),
        (
            failing(|| AppError::ProviderRejected {
                status: 400,
                detail: "typesafe 400".into(),
            }),
            "provider_rejected",
            Outcome::ProviderRejected,
        ),
        (
            failing(|| AppError::ProviderRejected {
                status: 401,
                detail: "typesafe disabled after an earlier 401".into(),
            }),
            "disabled",
            Outcome::ProviderRejected,
        ),
        (
            answering(1.5, MODEL).0,
            "validation_failure",
            Outcome::ValidationFailure,
        ),
    ];
    for (classifier, reason, outcome) in cases {
        let (row, screening) = screen(&jev(classifier, JevMode::On)).await;
        assert!(!screening.screened && !screening.warned, "{reason}");
        assert_eq!(screening.reason, Some(reason));
        assert_eq!(row.outcome, JevOutcome::Call(outcome), "{reason}");
        assert!(!row.applied);
    }
}

#[tokio::test]
async fn another_answering_model_never_warns() {
    let (classifier, _) = answering(0.99, "jev-1.13.0");
    let (_, screening) = screen(&jev(classifier, JevMode::On)).await;
    assert!(!screening.warned);
    assert_eq!(screening.reason, Some("validation_failure"));
}

#[tokio::test]
async fn a_call_past_the_deadline_or_with_no_time_left_is_a_timeout() {
    let mut mock = MockClassifier::new();
    mock.expect_classify().returning(|_| {
        std::thread::sleep(Duration::from_millis(5));
        Err(AppError::Timeout {
            what: "jev request",
            ms: 1,
        })
    });
    let (_, screening) = screen(&jev(Arc::new(mock), JevMode::On)).await;
    assert_eq!(screening.reason, Some("timeout"));

    let (classifier, seen) = answering(0.9, MODEL);
    let (row, screening) = screen_page(
        &jev(classifier, JevMode::On),
        "s1",
        "https://a.example",
        "text",
        &[],
        Duration::ZERO,
        chrono::Utc::now(),
    )
    .await;
    assert_eq!(screening.reason, Some("timeout"));
    assert_eq!(row.outcome, JevOutcome::Call(Outcome::Timeout));
    assert!(
        seen.lock().unwrap().is_empty(),
        "nothing is sent with no time left"
    );
}

#[tokio::test]
async fn a_page_that_cannot_be_redacted_is_never_sent() {
    let (classifier, seen) = answering(0.9, MODEL);
    let huge = "x ".repeat(150_000);
    let (row, screening) = screen_page(
        &jev(classifier, JevMode::On),
        "s1",
        "https://a.example",
        &huge,
        &[],
        SECOND,
        chrono::Utc::now(),
    )
    .await;
    assert_eq!(screening.reason, Some("redaction_failed"));
    assert_eq!(row.outcome, JevOutcome::Call(Outcome::InvalidInput));
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_secret_reaches_the_request_body() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "model": MODEL,
            "answers": { "directs_ai": { "type": "noul", "noul": 0.2 } },
            "usage": { "input_tokens": 10, "output_tokens": 1 },
        })))
        .mount(&mock)
        .await;
    let secret = ["sk-", "live-", "9f8e7d6c5b4a3210", "ffee"].concat();
    let token = ["ghp_", "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"].concat();
    let client =
        crate::client::typesafe::TypesafeClient::with_base_url("ts-key", MODEL, &mock.uri())
            .unwrap();
    let mut injection = jev(Arc::new(client), JevMode::On);
    injection.redactor = Redactor::new(vec![secret.clone()]);
    let (_, screening) = screen_page(
        &injection,
        "s1",
        &format!("https://user:{secret}@a.example/?token={token}"),
        &format!("Use the key {secret} and the token {token} here."),
        &[],
        SECOND,
        chrono::Utc::now(),
    )
    .await;
    assert!(screening.screened, "{screening:?}");
    let body = String::from_utf8(mock.received_requests().await.unwrap()[0].body.clone()).unwrap();
    assert!(!body.contains(&secret), "{body}");
    assert!(!body.contains(&token), "{body}");
}
