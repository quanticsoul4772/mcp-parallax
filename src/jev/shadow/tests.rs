//! Tests for the Jev shadow glue (064).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{shadow_turn, TurnIds, TurnInputs, TurnJev};
use crate::checkpoint::trajectory::{TrajectoryEntry, TrajectoryWindow};
use crate::jev::redact::Redactor;
use crate::traits::classifier::{JevRequest, JevResponse, MockClassifier};
use std::sync::{Arc, Mutex};

fn recording(sent: Arc<Mutex<Vec<String>>>) -> MockClassifier {
    let mut mock = MockClassifier::new();
    mock.expect_model_id()
        .return_const("jev-1.14.0".to_string());
    mock.expect_classify().returning(
        move |request: &JevRequest| -> Result<JevResponse, crate::error::AppError> {
            sent.lock()
                .unwrap()
                .push(request.state.as_str().to_string());
            Err(crate::error::AppError::ClassifierProvider("test".into()))
        },
    );
    mock
}

/// Review findings J and K: calls that may be billed say the cost is
/// unknown; calls that sent nothing say why.
#[test]
fn error_classes_name_unsent_calls_and_unknown_costs() {
    use crate::error::AppError;
    let disabled = AppError::ProviderRejected {
        status: 401,
        detail: "typesafe disabled after an earlier 401".into(),
    };
    assert_eq!(super::error_class(&disabled), "disabled_after_401");
    let rejected = AppError::ProviderRejected {
        status: 401,
        detail: "typesafe 401".into(),
    };
    assert_eq!(super::error_class(&rejected), "http_401");
    let no_slot = AppError::Timeout {
        what: "jev request slot",
        ms: 10,
    };
    assert_eq!(super::error_class(&no_slot), "no_slot");
    let deadline = AppError::Timeout {
        what: "jev request",
        ms: 10,
    };
    assert_eq!(super::error_class(&deadline), "deadline; cost unknown");
    let row = super::abandoned(super::tests_support::claim_row());
    assert_eq!(
        row.error_class.as_deref(),
        Some("verify_finished; cost unknown")
    );
}

/// Review finding C: redaction runs on the whole texts, before the final
/// message is cut and before transcript statements are split.
#[tokio::test]
async fn redaction_runs_before_the_cut_and_the_sentence_split() {
    let secret = ["sk-", "straddle", "Secret", "Value", "99"].concat();
    let jwt = [
        "eyJ",
        "hbGciOiJIUzI1NiJ9",
        ".",
        "eyJ",
        "zdWIiOiIxIn0",
        ".",
        "c2lnbmF0",
        "dXJl",
    ]
    .concat();
    let sent = Arc::new(Mutex::new(Vec::new()));
    let jev = TurnJev {
        classifier: Arc::new(recording(Arc::clone(&sent))),
        redactor: Redactor::new(vec![secret.clone()]),
        model: "jev-1.14.0".into(),
        server_session_id: "s".into(),
        deadline: std::time::Duration::from_millis(500),
    };
    let window = TrajectoryWindow {
        session_id: "s1".into(),
        entries: vec![TrajectoryEntry::Assistant {
            text: format!("The session token {jwt} is fully valid and safe to use."),
        }],
    };
    // The secret straddles the 2,000-character cut.
    let final_message = format!(
        "{} The session token is not valid and not safe to use. {}{secret}",
        "word ".repeat(380),
        "x".repeat(2000 - 5 * 380 - 54 - 10)
    ) + &" tail".repeat(20);
    let row = shadow_turn(
        &jev,
        TurnInputs {
            window: &window,
            activity: "no tool activity",
            final_message: &final_message,
            capture_enabled: false,
        },
        TurnIds {
            eval_id: "e1",
            harness_session_id: "s1",
            transcript_path: "t.jsonl",
            now: chrono::Utc::now(),
        },
    )
    .await;
    assert!(row.truncated);
    let sent = sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert!(
        !sent[0].contains(&secret[..10]),
        "secret fragment sent: {}",
        sent[0]
    );
    assert!(!sent[0].contains("eyJ"), "JWT fragment sent: {}", sent[0]);
}
