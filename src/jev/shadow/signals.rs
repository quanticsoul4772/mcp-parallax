//! 068 turn signals: the parts of the shared state each Jev question gets
//! (plan D1, D2).

use super::{questions_json, record_result, redaction_failed, turn_state, TurnIds, TurnState};
use crate::checkpoint::trajectory::TrajectoryWindow;
use crate::error::Outcome;
use crate::jev::exclusion::{claim_exclusion, Exclusion};
use crate::jev::questions::{pushback_reversal_question, unverified_fact_question};
use crate::jev::redact::{RedactedState, Redactor};
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::{Classifier, JevMode, JevQuestion, JevRequest, JevSite};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Jev for the turn-signal site (068).
pub struct SignalsJev {
    /// The classifier (the TypeSafe client in production).
    pub classifier: Arc<dyn Classifier>,
    /// The mandatory redactor.
    pub redactor: Redactor,
    /// The pinned model.
    pub model: String,
    /// This process's session id.
    pub server_session_id: String,
    /// The request deadline.
    pub deadline: Duration,
    /// `PARALLAX_JEV_SEND_USER_TEXT`: that turn's user text goes to
    /// `pushback_reversal` only (plan D1, D9).
    pub send_user_text: bool,
    /// `grounded_verify` is in the catalog (FR-006).
    pub grounded_verify: bool,
    /// `research` is in the catalog (FR-006).
    pub research: bool,
}

/// What the turn-signal shadow sees.
pub struct SignalInputs<'a> {
    /// The turn's window; transcript pairs are mined from redacted text.
    pub window: &'a TrajectoryWindow,
    /// The turn activity summary.
    pub activity: &'a str,
    /// The final assistant message.
    pub final_message: &'a str,
    /// That turn's user text; sent only with the opt-in.
    pub user_text: &'a str,
}

/// Ask Jev both turn-signal questions, one request each (plan D1), and build
/// the two rows, `pushback_reversal` first. Rows carry no label (plan D7).
pub async fn shadow_signals(
    jev: &SignalsJev,
    inputs: SignalInputs<'_>,
    ids: TurnIds<'_>,
) -> Vec<JevRecord> {
    let (pushback, unverified) = tokio::join!(
        pushback_row(jev, &inputs, &ids),
        unverified_row(jev, &inputs, &ids)
    );
    vec![pushback, unverified]
}

async fn pushback_row(jev: &SignalsJev, inputs: &SignalInputs<'_>, ids: &TurnIds<'_>) -> JevRecord {
    let question = pushback_reversal_question();
    let mut row = signal_row(jev, &question, ids);
    let recommends = json!("verify");
    let built = match turn_state(
        &jev.redactor,
        inputs.window,
        inputs.activity,
        inputs.final_message,
    ) {
        Ok(built) => built,
        Err(failed) => {
            row.truncated = failed.truncated;
            return annotate(
                redaction_failed(row, &failed.error),
                &[("recommends", recommends), ("number_diff", Value::Null)],
            );
        }
    };
    let Some(Value::Object(mut map)) = built.state.as_json().cloned() else {
        return annotate(
            state_not_an_object(row),
            &[("recommends", recommends), ("number_diff", Value::Null)],
        );
    };
    let diff = map
        .get("final_message")
        .and_then(Value::as_str)
        .map_or(Value::Null, |final_text| {
            let earlier: Vec<&str> = map
                .get("statement_pairs")
                .and_then(Value::as_array)
                .map(|pairs| {
                    pairs
                        .iter()
                        .filter_map(|p| p.get("earlier").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            Value::Bool(number_diff(final_text, &earlier))
        });
    let built = if jev.send_user_text {
        map.insert("user_text".into(), json!(inputs.user_text));
        match jev.redactor.redact_json(&Value::Object(map)) {
            Ok(state) => TurnState {
                redactions: built
                    .redactions
                    .saturating_add(u32::try_from(state.redactions()).unwrap_or(u32::MAX)),
                hash: state_hash(&state),
                state,
                truncated: built.truncated,
            },
            Err(error) => {
                row.truncated = built.truncated;
                return annotate(
                    redaction_failed(row, &error),
                    &[("recommends", recommends), ("number_diff", diff)],
                );
            }
        }
    } else {
        built
    };
    let row = ask(jev, row, built, question).await;
    annotate(row, &[("recommends", recommends), ("number_diff", diff)])
}

async fn unverified_row(
    jev: &SignalsJev,
    inputs: &SignalInputs<'_>,
    ids: &TurnIds<'_>,
) -> JevRecord {
    let question = unverified_fact_question();
    let mut row = signal_row(jev, &question, ids);
    let recommends = if jev.grounded_verify {
        json!("grounded_verify")
    } else if jev.research {
        json!("research")
    } else {
        Value::Null
    };
    // Redact the whole final message before splitting it into claims, so a
    // secret spread across sentences is removed first (064 review finding C).
    let redacted_final = match jev.redactor.redact(inputs.final_message) {
        Ok(redacted) => redacted,
        Err(error) => {
            return annotate(
                redaction_failed(row, &error),
                &[("recommends", recommends), ("excluded_claims", Value::Null)],
            );
        }
    };
    let (kept, excluded) = strip_number_and_date_claims(redacted_final.as_str());
    let excluded = json!(excluded);
    let mut built = match turn_state(&jev.redactor, inputs.window, inputs.activity, &kept) {
        Ok(built) => built,
        Err(failed) => {
            row.truncated = failed.truncated;
            return annotate(
                redaction_failed(row, &failed.error),
                &[("recommends", recommends), ("excluded_claims", excluded)],
            );
        }
    };
    built.redactions = built
        .redactions
        .saturating_add(u32::try_from(redacted_final.redactions()).unwrap_or(u32::MAX));
    let row = ask(jev, row, built, question).await;
    annotate(
        row,
        &[("recommends", recommends), ("excluded_claims", excluded)],
    )
}

/// A turn-signal row's `cooldown_suppressed` (068 plan D3, FR-007).
///
/// Its own kind when its answer is a yes (0.5 or more, the plan D3 note) and
/// the session already had a yes for that kind inside the cooldown window;
/// otherwise empty.
#[must_use]
pub fn cooldown_suppressed(row: &JevRecord, earlier_yes: &[String]) -> Vec<String> {
    let entry = row.questions_json.get(0);
    let kind = entry.and_then(|e| e.get("id")).and_then(Value::as_str);
    let yes = entry
        .and_then(|e| e.get("answer"))
        .and_then(Value::as_f64)
        .is_some_and(|p| p >= 0.5);
    match kind {
        Some(kind) if yes && earlier_yes.iter().any(|k| k == kind) => vec![kind.to_string()],
        _ => vec![],
    }
}

/// Send one question with its state, once, under the deadline, and fill the
/// row from the result.
async fn ask(
    jev: &SignalsJev,
    mut row: JevRecord,
    built: TurnState,
    question: JevQuestion,
) -> JevRecord {
    row.truncated = built.truncated;
    row.redaction_count = built.redactions;
    row.state_hash = Some(built.hash);
    let request = JevRequest {
        model: jev.model.clone(),
        state: built.state,
        questions: vec![question],
        timeout: jev.deadline,
        retry: false,
    };
    let started = Instant::now();
    let result = tokio::time::timeout(jev.deadline, jev.classifier.classify(&request)).await;
    row.latency_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    record_result(&mut row, result, &request, &jev.model);
    row
}

/// Add fields to the row's single `questions_json` entry.
fn annotate(mut row: JevRecord, fields: &[(&str, Value)]) -> JevRecord {
    if let Some(entry) = row.questions_json.get_mut(0).and_then(Value::as_object_mut) {
        for (key, value) in fields {
            entry.insert((*key).to_string(), value.clone());
        }
    }
    row
}

/// A state that was not a JSON object: nothing is sent.
fn state_not_an_object(mut row: JevRecord) -> JevRecord {
    row.outcome = JevOutcome::Call(Outcome::InvalidInput);
    row.error_class = Some("state:not_an_object".into());
    row
}

/// The fingerprint stored as the row's `state_hash`, as 064's turn state.
fn state_hash(state: &RedactedState) -> String {
    format!(
        "{:016x}",
        crate::checkpoint::fnv1a64(state.as_str().as_bytes())
    )
}

/// The turn-signal row before any request.
fn signal_row(jev: &SignalsJev, question: &JevQuestion, ids: &TurnIds<'_>) -> JevRecord {
    JevRecord {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: ids.now,
        server_session_id: jev.server_session_id.clone(),
        harness_session_id: Some(ids.harness_session_id.to_string()),
        site: JevSite::TurnSignals,
        mode: JevMode::Shadow,
        questions_json: questions_json(std::slice::from_ref(question), &[]),
        model_requested: jev.model.clone(),
        model_answered: None,
        outcome: JevOutcome::Call(Outcome::Success),
        error_class: None,
        request_id: None,
        latency_ms: None,
        input_tokens: None,
        cost_usd: None,
        eval_id: Some(ids.eval_id.to_string()),
        invocation_id: None,
        claim_key: None,
        source_ids: None,
        label_json: None,
        label_source: None,
        label_cost_usd: None,
        cooldown_suppressed: None,
        state_json: None,
        state_hash: None,
        transcript_ref: Some(ids.transcript_path.to_string()),
        redaction_count: 0,
        truncated: false,
        applied: false,
    }
}

/// The final text for `unverified_fact` (068 plan D2, FR-004): each claim
/// (sentence) that [`claim_exclusion`] marks as a number or a date is taken
/// out; returns the kept claims joined by single spaces and how many were
/// taken out.
pub fn strip_number_and_date_claims(text: &str) -> (String, usize) {
    let mut kept: Vec<&str> = Vec::new();
    let mut removed = 0;
    for claim in claims(text) {
        match claim_exclusion(claim, false) {
            Some(Exclusion::Number | Exclusion::Date) => removed += 1,
            Some(Exclusion::FlaggedSource) | None => kept.push(claim),
        }
    }
    (kept.join(" "), removed)
}

/// The `pushback_reversal` entry's `number_diff` (068 plan D2a, roadmap
/// §3.2): true when the final text and the earlier statement of any
/// transcript pair both hold numbers or dates and those differ.
pub fn number_diff(final_text: &str, earlier_statements: &[&str]) -> bool {
    let finals = number_and_date_tokens(final_text);
    !finals.is_empty()
        && earlier_statements.iter().any(|earlier| {
            let tokens = number_and_date_tokens(earlier);
            !tokens.is_empty() && tokens != finals
        })
}

/// A text's numbers (digit runs, with `.` or `,` between digits), number
/// words, date words and relative dates (`last`/`next` + week, month or
/// year), lowercased; the same word lists as [`claim_exclusion`].
fn number_and_date_tokens(text: &str) -> std::collections::BTreeSet<String> {
    use crate::jev::exclusion::{DATE_WORDS, NUMBER_WORDS};
    let mut tokens: std::collections::BTreeSet<String> = text
        .split(|c: char| !(c.is_numeric() || c == '.' || c == ','))
        .map(|piece| piece.trim_matches(|c| c == '.' || c == ','))
        .filter(|piece| piece.chars().any(char::is_numeric))
        .map(str::to_string)
        .collect();
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    for pair in words.windows(2) {
        if matches!(pair[0].as_str(), "last" | "next")
            && matches!(pair[1].as_str(), "week" | "month" | "year")
        {
            tokens.insert(format!("{} {}", pair[0], pair[1]));
        }
    }
    tokens.extend(
        words
            .into_iter()
            .filter(|w| NUMBER_WORDS.contains(&w.as_str()) || DATE_WORDS.contains(&w.as_str())),
    );
    tokens
}

/// Splits text into claims: a claim ends at `.`, `!` or `?` followed by
/// whitespace or the end of the text, or at a line break.
fn claims(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let end = if c == '\n' {
            Some((i, i + 1))
        } else if matches!(c, '.' | '!' | '?')
            && chars.peek().is_none_or(|&(_, next)| next.is_whitespace())
        {
            Some((i + 1, i + 1))
        } else {
            None
        };
        if let Some((claim_end, next_start)) = end {
            let claim = text[start..claim_end].trim();
            if !claim.is_empty() {
                out.push(claim);
            }
            start = next_start;
        }
    }
    let rest = text[start..].trim();
    if !rest.is_empty() {
        out.push(rest);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::strip_number_and_date_claims;

    // 068 T008 (FR-004, plan D2): the final message is split into claims
    // (sentences ending at `.`, `!` or `?` before whitespace, or at a line
    // break); each claim `claim_exclusion` excludes as a number or a date is
    // taken out of the `unverified_fact` state and counted, and every other
    // claim is kept, joined by single spaces.
    #[test]
    fn claims_with_numbers_or_dates_are_taken_out_and_counted() {
        let cases = [
            (
                "The build passes. It has 4096 entries.",
                "The build passes.",
                1,
            ),
            (
                "The parser is fixed. It failed three times.",
                "The parser is fixed.",
                1,
            ),
            (
                "Shipped in March. The docs are updated.",
                "The docs are updated.",
                1,
            ),
            (
                "The office closes on Tuesday! Tests pass.",
                "Tests pass.",
                1,
            ),
            (
                "Did it break yesterday? The fix is in.",
                "The fix is in.",
                1,
            ),
            (
                "Prices change next month. Nothing else changed.",
                "Nothing else changed.",
                1,
            ),
            (
                "Growth was 3.5% overall. The cache is warm.",
                "The cache is warm.",
                1,
            ),
            ("First line\nIt has 5 items.", "First line", 1),
            ("In 2018 it shipped. On Friday it broke.", "", 2),
        ];
        for (text, kept, count) in cases {
            let (got_kept, got_count) = strip_number_and_date_claims(text);
            assert_eq!(got_kept, kept, "{text}");
            assert_eq!(got_count, count, "{text}");
        }
    }

    #[test]
    fn claims_without_numbers_or_dates_are_all_kept() {
        let text = "Rust has no garbage collector. The tool may fail on large inputs. \
                    Marching bands are loud.";
        assert_eq!(strip_number_and_date_claims(text), (text.to_string(), 0));
    }

    // 068 T009a (plan D2a, roadmap §3.2): `number_diff` is true when the
    // final text and the earlier statement of any transcript pair both hold
    // numbers or dates and those differ; matching numbers, or none on either
    // side, give false.
    #[test]
    fn number_diff_is_true_only_when_numbers_or_dates_differ() {
        let cases: [(&str, &[&str], bool); 9] = [
            ("It has 4096 entries.", &["It has 4000 entries."], true),
            ("Growth was 3.6%.", &["Growth was 3.5%."], true),
            ("It failed three times.", &["It failed four times."], true),
            ("It shipped in March.", &["It shipped in April."], true),
            (
                "It has 4096 entries.",
                &["No numbers here.", "It had 12."],
                true,
            ),
            ("It has 4096 entries.", &["It has 4096 entries."], false),
            ("The build passes.", &["It has 4000 entries."], false),
            ("It has 4096 entries.", &["The build passes."], false),
            ("It has 4096 entries.", &[], false),
        ];
        for (final_text, earlier, expected) in cases {
            assert_eq!(
                super::number_diff(final_text, earlier),
                expected,
                "{final_text} vs {earlier:?}"
            );
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod wire_tests {
    use super::{shadow_signals, SignalInputs, SignalsJev};
    use crate::checkpoint::review::FINAL_MESSAGE_EXCERPT_CHARS;
    use crate::checkpoint::trajectory::{TrajectoryEntry, TrajectoryWindow};
    use crate::error::Outcome;
    use crate::jev::redact::Redactor;
    use crate::jev::shadow::TurnIds;
    use crate::storage::jev::JevOutcome;
    use crate::traits::classifier::{JevRequest, JevResponse, MockClassifier};
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    const USER_TEXT: &str = "That is wrong, it was 4000 entries.";
    const FINAL: &str = "The tests pass. It has 4096 entries.";

    /// Records `(first question id, sent state)` for each request.
    fn recording(sent: Arc<Mutex<Vec<(String, String)>>>) -> MockClassifier {
        let mut mock = MockClassifier::new();
        mock.expect_model_id()
            .return_const("jev-1.14.0".to_string());
        mock.expect_classify().returning(
            move |request: &JevRequest| -> Result<JevResponse, crate::error::AppError> {
                sent.lock().unwrap().push((
                    request.questions[0].id.clone(),
                    request.state.as_str().to_string(),
                ));
                Err(crate::error::AppError::ClassifierProvider("test".into()))
            },
        );
        mock
    }

    fn jev(classifier: MockClassifier, send_user_text: bool) -> SignalsJev {
        SignalsJev {
            classifier: Arc::new(classifier),
            redactor: Redactor::new(vec![]),
            model: "jev-1.14.0".into(),
            server_session_id: "s".into(),
            deadline: std::time::Duration::from_millis(500),
            send_user_text,
            grounded_verify: false,
            research: false,
        }
    }

    fn window() -> TrajectoryWindow {
        TrajectoryWindow {
            session_id: "s1".into(),
            entries: vec![TrajectoryEntry::Assistant {
                text: "It has 4000 entries.".into(),
            }],
        }
    }

    fn ids() -> TurnIds<'static> {
        TurnIds {
            eval_id: "e1",
            harness_session_id: "s1",
            transcript_path: "t.jsonl",
            now: chrono::Utc::now(),
        }
    }

    async fn sent_bodies(
        send_user_text: bool,
        activity: &str,
        final_message: &str,
    ) -> (Vec<(String, Value)>, Vec<crate::storage::jev::JevRecord>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let jev = jev(recording(Arc::clone(&sent)), send_user_text);
        let window = window();
        let rows = shadow_signals(
            &jev,
            SignalInputs {
                window: &window,
                activity,
                final_message,
                user_text: USER_TEXT,
            },
            ids(),
        )
        .await;
        let bodies = sent
            .lock()
            .unwrap()
            .iter()
            .map(|(id, state)| (id.clone(), serde_json::from_str(state).unwrap()))
            .collect();
        (bodies, rows)
    }

    fn keys(body: &Value) -> Vec<String> {
        let mut keys: Vec<String> = body.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    }

    // 068 T010 (plan D1, FR-009, SC-002): without the opt-in, each request's
    // whole body is the activity summary, the transcript-only pairs and the
    // final message; unverified_fact's final message has its number claim
    // removed; no user text anywhere.
    #[tokio::test]
    async fn without_the_opt_in_the_bodies_hold_no_user_text() {
        let (bodies, rows) = sent_bodies(false, "ran cargo test", FINAL).await;
        assert_eq!(rows.len(), 2);
        assert_eq!(bodies.len(), 2);
        let (pushback_id, pushback) = &bodies[0];
        let (unverified_id, unverified) = &bodies[1];
        assert_eq!(pushback_id, "pushback_reversal");
        assert_eq!(unverified_id, "unverified_fact");
        let expected = ["activity", "final_message", "statement_pairs"];
        assert_eq!(keys(pushback), expected);
        assert_eq!(keys(unverified), expected);
        assert_eq!(pushback["activity"], "ran cargo test");
        assert_eq!(pushback["final_message"], FINAL);
        assert!(pushback["statement_pairs"].is_array());
        assert_eq!(unverified["activity"], "ran cargo test");
        assert_eq!(unverified["final_message"], "The tests pass.");
        assert!(unverified["statement_pairs"].is_array());
        for (_, body) in &bodies {
            assert!(!body.to_string().contains("That is wrong"), "{body}");
        }
    }

    // 068 T010 (plan D1, D9): with the opt-in, only pushback_reversal's body
    // gains that turn's user text.
    #[tokio::test]
    async fn with_the_opt_in_only_pushback_reversal_gets_the_user_text() {
        let (bodies, _) = sent_bodies(true, "ran cargo test", FINAL).await;
        let (_, pushback) = &bodies[0];
        let (_, unverified) = &bodies[1];
        assert_eq!(
            keys(pushback),
            ["activity", "final_message", "statement_pairs", "user_text"]
        );
        assert_eq!(pushback["user_text"], USER_TEXT);
        assert_eq!(
            keys(unverified),
            ["activity", "final_message", "statement_pairs"]
        );
        assert!(!unverified.to_string().contains("That is wrong"));
    }

    // 068 T010 (plan D1 cap): each request's final message is cut to 2000
    // characters, with `truncated` set on its row.
    #[tokio::test]
    async fn a_long_final_message_is_cut_and_marked_on_both_rows() {
        let long = "word ".repeat(FINAL_MESSAGE_EXCERPT_CHARS);
        let (bodies, rows) = sent_bodies(false, "ran cargo test", &long).await;
        for (id, body) in &bodies {
            let text = body["final_message"].as_str().unwrap();
            assert!(
                text.chars().count() <= FINAL_MESSAGE_EXCERPT_CHARS,
                "{id}: {} chars",
                text.chars().count()
            );
        }
        assert!(rows.iter().all(|r| r.truncated));
    }

    // 068 T010: a redaction failure sends nothing and is recorded on both
    // rows.
    #[tokio::test]
    async fn a_redaction_failure_sends_nothing_and_is_recorded() {
        let mut mock = MockClassifier::new();
        mock.expect_model_id()
            .return_const("jev-1.14.0".to_string());
        mock.expect_classify().times(0);
        let jev = jev(mock, true);
        let window = window();
        let activity = "a".repeat(crate::jev::redact::MAX_INPUT_CHARS + 1);
        let rows = shadow_signals(
            &jev,
            SignalInputs {
                window: &window,
                activity: &activity,
                final_message: FINAL,
                user_text: USER_TEXT,
            },
            ids(),
        )
        .await;
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.outcome, JevOutcome::Call(Outcome::InvalidInput));
            assert_eq!(row.error_class.as_deref(), Some("redaction:too_large"));
        }
    }

    // 068 T011 (plan D2, D2a, D7): each row's questions_json entry carries
    // its added fields: recommends on both, number_diff on pushback_reversal,
    // excluded_claims on unverified_fact.
    #[tokio::test]
    async fn each_row_carries_its_added_fields() {
        let (_, rows) = sent_bodies(false, "ran cargo test", FINAL).await;
        let pushback = &rows[0].questions_json[0];
        let unverified = &rows[1].questions_json[0];
        assert_eq!(pushback["id"], "pushback_reversal");
        assert_eq!(pushback["recommends"], "verify");
        assert!(pushback["number_diff"].is_boolean(), "{pushback}");
        assert!(pushback.get("excluded_claims").is_none(), "{pushback}");
        assert_eq!(unverified["id"], "unverified_fact");
        assert_eq!(unverified["recommends"], Value::Null);
        assert_eq!(unverified["excluded_claims"], 1);
        assert!(unverified.get("number_diff").is_none(), "{unverified}");
        assert!(rows.iter().all(|r| r.label_json.is_none()));
    }

    // 068 T015b (FR-006, §11, roadmap check items 30 and 39): recommends names
    // only correctives in the catalog: verify for pushback_reversal; for
    // unverified_fact, grounded_verify when configured, else research when
    // configured, else none.
    #[tokio::test]
    async fn recommends_names_only_correctives_in_the_catalog() {
        let cases = [
            (false, false, Value::Null),
            (true, false, Value::from("grounded_verify")),
            (false, true, Value::from("research")),
            (true, true, Value::from("grounded_verify")),
        ];
        for (grounded_verify, research, expected) in cases {
            let mut jev = jev(recording(Arc::default()), false);
            jev.grounded_verify = grounded_verify;
            jev.research = research;
            let window = window();
            let rows = shadow_signals(
                &jev,
                SignalInputs {
                    window: &window,
                    activity: "ran cargo test",
                    final_message: FINAL,
                    user_text: USER_TEXT,
                },
                ids(),
            )
            .await;
            let case = format!("grounded_verify={grounded_verify} research={research}");
            assert_eq!(rows[0].questions_json[0]["recommends"], "verify", "{case}");
            assert_eq!(rows[1].questions_json[0]["recommends"], expected, "{case}");
        }
    }
}
