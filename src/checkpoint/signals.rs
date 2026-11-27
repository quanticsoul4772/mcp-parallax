//! The 066 turn signals: `pushback_reversal` and `unverified_fact` (FR-003).
//!
//! Cheap detectors in code, run at every non-continuation turn end with a
//! final message, independent of the review. No Jev call is made and nothing
//! leaves the machine; the results go to `turn_signal_records` only.

use crate::checkpoint::contract::CheckpointTurnParams;
use crate::checkpoint::review::mine_candidates;
use crate::checkpoint::trajectory::{TrajectoryEntry, TrajectoryWindow};
use crate::checkpoint::turn_view::{final_uuid, parse_turn, TurnData};
use crate::error::AppError;
use crate::jev::entities::{unmatched, Unmatched};
use crate::storage::turn_signals::TurnSignalRecord;
use crate::traits::storage::Storage;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The turn start was not in the lines read.
pub const TURN_START_NOT_FOUND: &str = "turn start not found";
/// No final message is held for this session (first turn, or a restart).
pub const NO_PREVIOUS_FINAL: &str = "no previous final message";
/// The transcript could not be read.
pub const TRANSCRIPT_UNREADABLE: &str = "transcript unreadable";
/// A tool ran after the prompt, so the reversal may rest on new evidence.
pub const TOOL_CALL_AFTER_PROMPT: &str = "tool call after prompt";

/// One signal's outcome for a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalResult {
    /// The detector found its pattern.
    Fired,
    /// The detector ran and found nothing.
    NotFired,
    /// The detector could not run; the reason is recorded.
    Excluded(&'static str),
}

impl SignalResult {
    /// The value stored in `turn_signal_records`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fired => "fired",
            Self::NotFired => "not_fired",
            Self::Excluded(reason) => reason,
        }
    }
}

/// What the turn signals need beside the checkpoint's own dependencies;
/// present only with `CHECKPOINT_TURN_SIGNALS=on`.
#[derive(Debug, Clone)]
pub struct TurnSignalsDeps {
    /// Shared across turn ends of the process.
    pub previous_finals: Arc<PreviousFinals>,
    /// Whether `grounded_verify` is in the catalog (FR-007).
    pub grounded_verify: bool,
    /// Whether `research` is in the catalog (FR-007).
    pub research: bool,
}

/// Record one non-continuation turn end's signals (FR-003, FR-011).
///
/// `lines` is `None` when the transcript could not be read. Swaps in this
/// turn's final message as the session's previous final.
///
/// # Errors
///
/// Returns [`AppError`] when the row cannot be written.
pub async fn record_turn(
    signals: &TurnSignalsDeps,
    storage: &dyn Storage,
    created_at: DateTime<Utc>,
    eval_id: &str,
    params: &CheckpointTurnParams,
    lines: Option<&[String]>,
) -> Result<(), AppError> {
    let previous = signals
        .previous_finals
        .swap(&params.session_id, &params.final_message);
    let turn = lines.map(|l| (parse_turn(l), final_uuid(l)));
    let row = build_row(&RowInput {
        id: uuid::Uuid::new_v4().to_string(),
        eval_id,
        harness_session_id: &params.session_id,
        transcript_ref: &params.transcript_path,
        turn: turn.as_ref().map(|(t, _)| t),
        final_uuid: turn.as_ref().and_then(|(_, uuid)| uuid.clone()),
        previous_final: previous.as_deref(),
        final_message: &params.final_message,
        grounded_verify: signals.grounded_verify,
        research: signals.research,
        created_at,
    });
    storage.record_turn_signals(&row).await
}

/// Sessions [`PreviousFinals`] keeps; the least recently updated goes first.
pub const PREVIOUS_FINALS_CAP: usize = 64;

/// The last final message per harness session, held in memory only
/// (FR-005), updated at every turn end including continuation turns. Keeps
/// the [`PREVIOUS_FINALS_CAP`] most recently updated sessions.
#[derive(Debug, Default)]
pub struct PreviousFinals(Mutex<Finals>);

/// Each session's final message and when it was last updated.
#[derive(Debug, Default)]
struct Finals {
    by_session: HashMap<String, (String, u64)>,
    clock: u64,
}

impl PreviousFinals {
    /// Store `final_message` for `session` and return the one it replaces.
    pub fn swap(&self, session: &str, final_message: &str) -> Option<String> {
        let mut finals = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        finals.clock += 1;
        let stamp = finals.clock;
        let previous = finals
            .by_session
            .insert(session.to_string(), (final_message.to_string(), stamp))
            .map(|(text, _)| text);
        if finals.by_session.len() > PREVIOUS_FINALS_CAP {
            let oldest = finals
                .by_session
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                finals.by_session.remove(&oldest);
            }
        }
        previous
    }

    /// Sessions held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .by_session
            .len()
    }

    /// Whether no session is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// `pushback_reversal` (research D5).
///
/// After a real prompt with no tool call, the pair miner (lexical overlap
/// plus opposing polarity, no stored memories) finds a pair between the
/// previous and the final message.
#[must_use]
pub fn pushback(
    turn: &TurnData,
    previous_final: Option<&str>,
    final_message: &str,
) -> SignalResult {
    if turn.start.is_none() {
        return SignalResult::Excluded(TURN_START_NOT_FOUND);
    }
    let Some(previous) = previous_final else {
        return SignalResult::Excluded(NO_PREVIOUS_FINAL);
    };
    if turn.tool_calls_after_prompt > 0 {
        return SignalResult::Excluded(TOOL_CALL_AFTER_PROMPT);
    }
    let window = TrajectoryWindow {
        session_id: String::new(),
        entries: vec![TrajectoryEntry::Assistant {
            text: previous.to_string(),
        }],
    };
    if mine_candidates(&window, final_message, &[]).is_empty() {
        SignalResult::NotFired
    } else {
        SignalResult::Fired
    }
}

/// `unverified_fact` (research D6): entities of the final message not found
/// in the turn's tool inputs, tool outputs or user prompt.
#[must_use]
pub fn unverified_fact(turn: &TurnData, final_message: &str) -> (SignalResult, Vec<Unmatched>) {
    if turn.start.is_none() {
        return (SignalResult::Excluded(TURN_START_NOT_FOUND), Vec::new());
    }
    let mut evidence: Vec<&str> = turn
        .tool_inputs
        .iter()
        .chain(&turn.tool_outputs)
        .map(String::as_str)
        .collect();
    evidence.push(&turn.user_prompt);
    let missing = unmatched(final_message, &evidence);
    let result = if missing.is_empty() {
        SignalResult::NotFired
    } else {
        SignalResult::Fired
    };
    (result, missing)
}

/// The corrective each signal names (FR-007): `verify` for a reversal;
/// `grounded_verify`, else `research`, else none for an unverified fact.
#[must_use]
pub fn recommendations(grounded_verify: bool, research: bool) -> Value {
    let unverified = if grounded_verify {
        Some("grounded_verify")
    } else if research {
        Some("research")
    } else {
        None
    };
    json!({ "pushback_reversal": "verify", "unverified_fact": unverified })
}

/// Everything one row needs.
#[derive(Debug, Clone)]
pub struct RowInput<'a> {
    /// Row id.
    pub id: String,
    /// The checkpoint record's id.
    pub eval_id: &'a str,
    /// The harness session.
    pub harness_session_id: &'a str,
    /// The transcript path.
    pub transcript_ref: &'a str,
    /// The turn data; `None` when the transcript could not be read.
    pub turn: Option<&'a TurnData>,
    /// The `uuid` of the final message's line.
    pub final_uuid: Option<String>,
    /// The previous final message for this session.
    pub previous_final: Option<&'a str>,
    /// This turn's final message.
    pub final_message: &'a str,
    /// Whether `grounded_verify` is in the catalog.
    pub grounded_verify: bool,
    /// Whether `research` is in the catalog.
    pub research: bool,
    /// Evaluation time.
    pub created_at: DateTime<Utc>,
}

/// Run both detectors and build the row (FR-011).
#[must_use]
pub fn build_row(input: &RowInput<'_>) -> TurnSignalRecord {
    let (pushback_result, unverified_result, missing) = input.turn.map_or_else(
        || {
            (
                SignalResult::Excluded(TRANSCRIPT_UNREADABLE),
                SignalResult::Excluded(TRANSCRIPT_UNREADABLE),
                Vec::new(),
            )
        },
        |turn| {
            let (unverified, missing) = unverified_fact(turn, input.final_message);
            (
                pushback(turn, input.previous_final, input.final_message),
                unverified,
                missing,
            )
        },
    );
    TurnSignalRecord {
        id: input.id.clone(),
        created_at: input.created_at,
        eval_id: input.eval_id.to_string(),
        harness_session_id: Some(input.harness_session_id.to_string()),
        transcript_ref: Some(input.transcript_ref.to_string()),
        turn_start_uuid: input.turn.and_then(|t| t.start_uuid.clone()),
        final_uuid: input.final_uuid.clone(),
        pushback_result: pushback_result.as_str().to_string(),
        unverified_result: unverified_result.as_str().to_string(),
        unmatched_entities: Value::Array(
            missing
                .iter()
                .map(|m| json!({ "kind": m.kind.as_str(), "hash": m.hash }))
                .collect(),
        ),
        recommendations: recommendations(input.grounded_verify, input.research),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::checkpoint::turn_view::TurnData;

    fn turn(prompt: &str, tool_calls: u32) -> TurnData {
        TurnData {
            start: Some(3),
            start_uuid: Some("u03".to_string()),
            user_prompt: prompt.to_string(),
            tool_inputs: Vec::new(),
            tool_outputs: Vec::new(),
            tool_calls_after_prompt: tool_calls,
        }
    }

    const EARLIER: &str = "The database migration is fully reversible and safe to run.";
    const REVERSED: &str = "You are right, the database migration is not reversible after all.";
    const UNRELATED: &str = "The parser handles every input format we listed earlier today.";

    // ---- T013: pushback_reversal ----

    #[test]
    fn pushback_fires_on_a_reversed_pair_after_a_prompt_with_no_tool_call() {
        let result = pushback(&turn("are you sure?", 0), Some(EARLIER), REVERSED);
        assert_eq!(result, SignalResult::Fired);
    }

    #[test]
    fn pushback_does_not_fire_without_an_opposing_pair() {
        let result = pushback(&turn("are you sure?", 0), Some(EARLIER), UNRELATED);
        assert_eq!(result, SignalResult::NotFired);
    }

    #[test]
    fn a_tool_call_after_the_prompt_excludes_pushback() {
        let result = pushback(&turn("are you sure?", 1), Some(EARLIER), REVERSED);
        assert_eq!(result, SignalResult::Excluded(TOOL_CALL_AFTER_PROMPT));
        assert_eq!(result.as_str(), "tool call after prompt");
    }

    #[test]
    fn no_previous_final_message_excludes_pushback() {
        let result = pushback(&turn("are you sure?", 0), None, REVERSED);
        assert_eq!(result.as_str(), "no previous final message");
    }

    #[test]
    fn a_missing_turn_start_excludes_pushback_first() {
        let no_start = TurnData::default();
        assert_eq!(
            pushback(&no_start, None, REVERSED).as_str(),
            "turn start not found"
        );
    }

    #[test]
    fn pushback_fires_again_on_each_later_turn_with_no_cooldown() {
        for _ in 0..3 {
            assert_eq!(
                pushback(&turn("really?", 0), Some(EARLIER), REVERSED),
                SignalResult::Fired
            );
        }
    }

    #[test]
    fn the_previous_final_map_returns_the_last_message_and_keeps_the_new_one() {
        let finals = PreviousFinals::default();
        assert_eq!(finals.swap("s1", "first"), None);
        assert_eq!(finals.swap("s1", "second").as_deref(), Some("first"));
        assert_eq!(finals.swap("s2", "other"), None);
        assert_eq!(finals.swap("s1", "third").as_deref(), Some("second"));
    }

    // 066 review finding 5: the map keeps the most recent 64 sessions.
    #[test]
    fn the_previous_final_map_keeps_only_the_most_recent_sessions() {
        let finals = PreviousFinals::default();
        for n in 0..PREVIOUS_FINALS_CAP {
            finals.swap(&format!("s{n}"), "m");
        }
        // Touch s0 so it is the most recent, then add one more session.
        assert_eq!(finals.swap("s0", "m2").as_deref(), Some("m"));
        finals.swap("new", "m");
        assert_eq!(finals.len(), PREVIOUS_FINALS_CAP);
        // s1 was the least recently updated, so it went; s0 stayed.
        assert_eq!(finals.swap("s1", "x"), None);
        assert_eq!(finals.swap("s0", "x").as_deref(), Some("m2"));
    }

    // ---- T014: unverified_fact ----

    #[test]
    fn unverified_fact_fires_when_an_entity_is_missing_from_the_turn_evidence() {
        let mut t = turn("check the build", 1);
        t.tool_outputs = vec!["parser = \"1.2.4\"".to_string()];
        let (result, unmatched) = unverified_fact(&t, "The parser is at 1.2.3.");
        assert_eq!(result, SignalResult::Fired);
        assert_eq!(unmatched.len(), 1);
        assert_eq!(unmatched[0].kind.as_str(), "version");
    }

    #[test]
    fn unverified_fact_does_not_fire_when_every_entity_is_found() {
        let mut t = turn("what is in src/lib.rs?", 1);
        t.tool_inputs = vec![r#"{"file_path":"src/lib.rs"}"#.to_string()];
        t.tool_outputs = vec!["version 1.2.3, 4096 lines".to_string()];
        let (result, unmatched) = unverified_fact(&t, "src/lib.rs is at 1.2.3 and has 4096 lines.");
        assert_eq!(result, SignalResult::NotFired);
        assert!(unmatched.is_empty());
    }

    #[test]
    fn the_user_prompt_counts_as_evidence() {
        let t = turn("rename toolu_f01 please", 0);
        let (result, _) = unverified_fact(&t, "Renamed toolu_f01.");
        assert_eq!(result, SignalResult::NotFired);
    }

    #[test]
    fn a_missing_turn_start_excludes_unverified_fact() {
        let (result, unmatched) = unverified_fact(&TurnData::default(), "at 1.2.3");
        assert_eq!(result.as_str(), "turn start not found");
        assert!(unmatched.is_empty());
    }

    #[test]
    fn unverified_fact_fires_again_on_each_later_turn() {
        for _ in 0..3 {
            let (result, _) = unverified_fact(&turn("go", 0), "Merged a60f12d.");
            assert_eq!(result, SignalResult::Fired);
        }
    }

    // ---- FR-007: recommendations ----

    #[test]
    fn pushback_recommends_verify_and_unverified_fact_follows_the_catalog() {
        let r = recommendations(true, true);
        assert_eq!(r["pushback_reversal"], "verify");
        assert_eq!(r["unverified_fact"], "grounded_verify");
        assert_eq!(recommendations(false, true)["unverified_fact"], "research");
        assert!(recommendations(false, false)["unverified_fact"].is_null());
        assert_eq!(recommendations(false, false)["pushback_reversal"], "verify");
    }

    #[test]
    fn transcript_unreadable_is_a_named_exclusion() {
        assert_eq!(
            SignalResult::Excluded(TRANSCRIPT_UNREADABLE).as_str(),
            "transcript unreadable"
        );
        assert_eq!(SignalResult::Fired.as_str(), "fired");
        assert_eq!(SignalResult::NotFired.as_str(), "not_fired");
    }
}
