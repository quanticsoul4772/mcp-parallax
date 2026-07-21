//! End-of-turn review (D6).
//!
//! Deterministic candidate mining gates the layer's ONLY model pass — one
//! blind, decline-biased, flat-schema classification. The hop classifies;
//! verdict mapping and flag wording are pure functions (FR-005). Candidates
//! carry verbatim statements stripped of surrounding self-justification
//! (FR-012) plus a between-statements evidence summary so the hop can apply
//! FR-004(d): an evidence-justified reversal is NOT a contradiction.

use crate::checkpoint::preference::{self, PreferenceCandidate};
use crate::checkpoint::trajectory::{TrajectoryEntry, TrajectoryWindow};
use crate::checkpoint::{Signal, SignalKind, REVIEW_CANDIDATES_MAX, REVIEW_RECALL_FLOOR};
use crate::error::AppError;
use crate::memory::ranking::cosine;
use crate::memory::Memory;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use serde::Deserialize;

/// The review mode's registry id.
pub const REVIEW_MODE_ID: &str = "checkpoint_review";

/// Minimum shared content words for a transcript pair to become a candidate.
/// (3, not 4: candidates only gate the decline-biased hop — modest recall
/// here costs one cheap classification, a miss costs the signal.)
const OVERLAP_MIN: usize = 3;

/// Minimum sentence length considered a committed statement.
const SENTENCE_MIN_CHARS: usize = 20;

/// Polarity cues: a pair qualifies only when one side carries a
/// negation/reversal marker the other lacks — cheap screening for explicit
/// opposition, not meaning.
const POLARITY_CUES: &[&str] = &[
    "not ",
    "n't",
    "never",
    "no longer",
    "cannot",
    "instead",
    "actually",
    "however",
    "wrong",
];

const REVIEW_PROMPT_TEMPLATE: &str = "\
You are an independent end-of-turn reviewer making two separate judgments \
about one working session. You see only the material below — no author \
identity, no justification, no stakes. On BOTH judgments, when uncertain \
answer false; a false alarm is worse than a miss.\n\
\n\
JUDGMENT 1 — self-contradiction. Decide whether ANY candidate statement pair \
is a real self-contradiction. A real contradiction is EXPLICIT and MATERIAL: \
the two statements cannot both be true as written. Refinements, added detail, \
tone shifts, and narrowed scope are NOT contradictions. HARD RULE: a reversal \
justified by evidence that appeared between the two statements (see each \
pair's 'observed between') is NOT a contradiction — it is an update, which is \
correct behavior. If exactly one pair contradicts, return contradicts=true \
with statement_a (the earlier statement, verbatim), statement_b (the final \
statement, verbatim), and basis (one sentence: why both cannot hold). If \
several contradict, return the most material one. Otherwise (or when no \
pairs are listed) contradicts=false with empty strings and basis stating the \
strongest reason the candidates are consistent.\n\
\n\
JUDGMENT 2 — stored-preference violation. Decide whether this turn violates \
ANY numbered stored preference — in its final message wording, or in the \
observable turn activity. A preference too vague to check concretely is NOT \
violated; a preference that does not bear on what this turn did is NOT \
violated. If exactly one is violated, return violates=true with \
violated_preference (that preference, verbatim from the list) and \
violation_basis (one sentence: what in the turn violates it). If several are \
violated, return the most material one. Otherwise (or when no preferences \
are listed) violates=false with empty strings.\n\
\n\
Candidate pairs:\n<<candidates>>\n\
\n\
Stored preferences and turn evidence:\n<<preferences>>";

/// The hop's constrained output (flat + closed — Principle II).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ReviewOut {
    /// Whether a real, explicit, material contradiction exists.
    pub contradicts: bool,
    /// The earlier statement, verbatim (empty when contradicts is false).
    pub statement_a: String,
    /// The final statement, verbatim (empty when contradicts is false).
    pub statement_b: String,
    /// One sentence of grounds.
    pub basis: String,
    /// The turn violates a listed stored preference (uncertain ⇒ false —
    /// decline bias, 015 FR-004).
    pub violates: bool,
    /// Verbatim echo of the violated preference — used ONLY for server-side
    /// map-back to the mined candidate (015 research D5); never quoted in
    /// the flag. Empty when violates is false.
    pub violated_preference: String,
    /// One sentence: what in the turn violates it (empty when violates is
    /// false).
    pub violation_basis: String,
}

/// One candidate pair for the hop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The earlier committed statement (or stored decision), verbatim.
    pub earlier: String,
    /// The final-message statement, verbatim.
    pub final_statement: String,
    /// Compact summary of tool outcomes observed between the statements
    /// (FR-004(d) input).
    pub between: String,
}

/// Register the review mode (boot-time; enforces flat+closed).
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(ReviewOut))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        REVIEW_MODE_ID,
        "internal: checkpoint end-of-turn review",
        REVIEW_PROMPT_TEMPLATE,
        schema,
        1,
    )
}

/// Mine candidate pairs: stored decisions relevant to the final message, and
/// transcript pairs with high lexical overlap plus opposing polarity. Pure.
#[must_use]
pub fn mine_candidates(
    window: &TrajectoryWindow,
    final_message: &str,
    recall: &[(f32, Memory)],
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let final_sentences = sentences(final_message);

    // (a) Memory candidates: relevant constraint-kind stored decisions.
    for (score, memory) in recall {
        if *score >= REVIEW_RECALL_FLOOR && crate::checkpoint::gate::is_constraint(memory) {
            let final_statement = final_sentences
                .iter()
                .max_by_key(|s| overlap(&memory.content, s))
                .map_or_else(|| truncate(final_message, 240), |s| (*s).to_string());
            candidates.push(Candidate {
                earlier: memory.content.clone(),
                final_statement,
                between: window_summary(&window.entries),
            });
        }
    }

    // (b) Transcript pairs: earlier assistant sentences vs final sentences,
    // high overlap + opposing polarity.
    for (index, entry) in window.entries.iter().enumerate() {
        let TrajectoryEntry::Assistant { text } = entry else {
            continue;
        };
        if text == final_message {
            continue;
        }
        for earlier_sentence in sentences(text) {
            for final_sentence in &final_sentences {
                if overlap(earlier_sentence, final_sentence) >= OVERLAP_MIN
                    && opposing_polarity(earlier_sentence, final_sentence)
                {
                    candidates.push(Candidate {
                        earlier: earlier_sentence.to_string(),
                        final_statement: (*final_sentence).to_string(),
                        between: between_summary(&window.entries[index + 1..]),
                    });
                }
            }
        }
    }

    // Dedup on the statement pair (non-adjacent repeats across assistant
    // entries differ only in `between` and would burn the cap — review
    // finding 6); first occurrence wins.
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    candidates.retain(|c| seen.insert((c.earlier.clone(), c.final_statement.clone())));
    candidates.truncate(REVIEW_CANDIDATES_MAX);
    candidates
}

/// Rank recall hits for the review (pure cosine; the caller embeds).
#[must_use]
pub fn rank_recall(query: &[f32], memories: &[Memory]) -> Vec<(f32, Memory)> {
    let mut scored: Vec<(f32, Memory)> = memories
        .iter()
        .map(|m| (cosine(query, &m.embedding), m.clone()))
        .filter(|(score, _)| *score >= REVIEW_RECALL_FLOOR)
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

/// Max final-message chars included as violation evidence (015 research
/// D3) — keeps the hop prompt bounded.
const FINAL_MESSAGE_EXCERPT_CHARS: usize = 2000;

/// Deterministic activity summary of the turn's bounded window — the
/// violation judgment's process evidence (015 research D3).
#[must_use]
pub fn turn_activity(window: &TrajectoryWindow) -> String {
    window_summary(&window.entries)
}

/// Run the single review hop — one pass, two judgments (015 FR-010).
///
/// Judges the mined contradiction candidates and preference candidates in
/// a single constrained pass and returns the confirmed flags in delivery
/// order: contradiction first, then violation (015 research D6).
///
/// # Errors
///
/// Provider classes from the model call; schema violations are
/// `ValidationFailure`.
pub async fn review_once(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    candidates: &[Candidate],
    preferences: &[PreferenceCandidate],
    final_message: &str,
    activity: &str,
) -> Result<(Vec<(Signal, String)>, u64, u64), AppError> {
    use std::fmt::Write as _;
    let mut listing = String::new();
    if candidates.is_empty() {
        listing.push_str("(none)\n");
    }
    for (i, candidate) in candidates.iter().enumerate() {
        let _ = writeln!(
            listing,
            "{}. earlier: \"{}\"\n   final: \"{}\"\n   observed between: {}",
            i + 1,
            candidate.earlier,
            candidate.final_statement,
            candidate.between
        );
    }
    let mut preference_block = String::new();
    if preferences.is_empty() {
        preference_block.push_str("(none)\n");
    } else {
        for (i, preference) in preferences.iter().enumerate() {
            let _ = writeln!(preference_block, "{}. \"{}\"", i + 1, preference.content);
        }
        let _ = writeln!(
            preference_block,
            "Turn final message (excerpt): \"{}\"",
            truncate(final_message, FINAL_MESSAGE_EXCERPT_CHARS)
        );
        let _ = writeln!(preference_block, "Turn activity: {activity}");
    }
    // One-pass substitution on the pristine template only (the 005
    // template-injection rule): both placeholders are located on the
    // pristine template before any caller text is inserted, so listed
    // content is never re-scanned for placeholders.
    let Some((head, rest)) = mode.prompt_template.split_once("<<candidates>>") else {
        return Err(AppError::ValidationFailure(
            "review template lost its candidates placeholder".to_string(),
        ));
    };
    let Some((mid, tail)) = rest.split_once("<<preferences>>") else {
        return Err(AppError::ValidationFailure(
            "review template lost its preferences placeholder".to_string(),
        ));
    };
    let prompt = format!("{head}{listing}{mid}{preference_block}{tail}");

    let completion = client.complete(&prompt, &mode.sanitized_schema).await?;
    validate(&mode.output_schema, &completion.value)?;
    let out: ReviewOut = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("review shape: {e}")))?;

    let mut flagged: Vec<(Signal, String)> = Vec::new();
    if out.contradicts && !candidates.is_empty() {
        // Cooldown identity from the MINED pair, not the model's echo —
        // wording drift between turns must not defeat FR-010 suppression
        // (review finding 7). Best-overlap match back to a candidate;
        // the echo is the fallback if nothing matches.
        let identity = candidates
            .iter()
            .max_by_key(|c| {
                overlap(&c.earlier, &out.statement_a)
                    + overlap(&c.final_statement, &out.statement_b)
            })
            .map_or_else(
                || format!("{}|{}", out.statement_a, out.statement_b),
                |c| format!("{}|{}", c.earlier, c.final_statement),
            );
        let signal = Signal::new(
            SignalKind::SelfContradiction,
            format!(
                "earlier: \"{}\" vs final: \"{}\"",
                out.statement_a, out.statement_b
            ),
            &identity,
        );
        flagged.push((signal, assemble_flag(&out)));
    }
    if out.violates && !preferences.is_empty() {
        // Map the echo back to the mined candidate (015 research D5): the
        // flag's quoted content, memory id, and trust come from the
        // server-held candidate, never from the echo — a hallucinated echo
        // cannot mis-attribute, it can only pick the wrong (still real,
        // still recalled) candidate.
        if let Some(matched) = preferences
            .iter()
            .max_by_key(|p| overlap(&p.content, &out.violated_preference))
        {
            flagged.push(preference::violation_signal(matched, &out.violation_basis));
        }
    }
    Ok((flagged, completion.input_tokens, completion.output_tokens))
}

/// The fixed flag template (FR-005/SC-007): parameterized only by evidence.
fn assemble_flag(out: &ReviewOut) -> String {
    format!(
        "End-of-turn review: your conclusion contradicts an earlier committed \
         statement. Earlier: \"{}\" — Final: \"{}\". Basis: {} Reconcile the two \
         explicitly before finishing.",
        out.statement_a, out.statement_b, out.basis
    )
}

fn truncate(text: &str, max: usize) -> String {
    let collected: String = text.chars().take(max).collect();
    collected.trim().to_string()
}

/// Split into trimmed sentences of committed-statement length.
fn sentences(text: &str) -> Vec<&str> {
    text.split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| s.chars().count() >= SENTENCE_MIN_CHARS)
        .collect()
}

/// Shared content words (lowercased, ≥ 4 chars) between two statements.
fn overlap(a: &str, b: &str) -> usize {
    let words = |s: &str| -> std::collections::HashSet<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 4)
            .map(str::to_string)
            .collect()
    };
    words(a).intersection(&words(b)).count()
}

/// One side carries a polarity cue the other lacks.
fn opposing_polarity(a: &str, b: &str) -> bool {
    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();
    POLARITY_CUES
        .iter()
        .any(|cue| a_lower.contains(cue) != b_lower.contains(cue))
}

/// Evidence summary for tool calls after the earlier statement (FR-004(d)).
fn between_summary(entries: &[TrajectoryEntry]) -> String {
    summarize_calls(entries, "since the earlier statement")
}

/// Evidence summary over the whole window (memory candidates).
fn window_summary(entries: &[TrajectoryEntry]) -> String {
    summarize_calls(entries, "in this turn's window")
}

fn summarize_calls(entries: &[TrajectoryEntry], scope: &str) -> String {
    let (mut total, mut failed) = (0_usize, 0_usize);
    let mut tools: Vec<String> = Vec::new();
    for entry in entries {
        if let TrajectoryEntry::ToolCall {
            tool_name,
            failed: f,
            ..
        } = entry
        {
            total += 1;
            failed += usize::from(*f);
            if !tools.contains(tool_name) {
                tools.push(tool_name.clone());
            }
        }
    }
    if total == 0 {
        return format!("no tool activity {scope}");
    }
    format!(
        "{total} tool call(s) {scope} ({failed} failed; tools: {})",
        tools.join(", ")
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::memory::{Kind, Trust};
    use crate::traits::client::{Completion, MockModelClient};
    use chrono::{DateTime, Utc};
    use serde_json::json;

    fn assistant(text: &str) -> TrajectoryEntry {
        TrajectoryEntry::Assistant {
            text: text.to_string(),
        }
    }

    fn call(failed: bool) -> TrajectoryEntry {
        TrajectoryEntry::ToolCall {
            batch_index: 1,
            tool_name: "bash".into(),
            normalized_input: "{command=cargo test;}".into(),
            failed,
        }
    }

    fn window(entries: Vec<TrajectoryEntry>) -> TrajectoryWindow {
        TrajectoryWindow {
            session_id: "s1".into(),
            entries,
        }
    }

    #[test]
    fn seeded_reversal_pair_is_found_with_between_summary() {
        let w = window(vec![
            assistant("The database migration is fully reversible and safe to run."),
            call(false),
            call(true),
        ]);
        let final_message =
            "After review, the database migration is not reversible and cannot be safely run.";
        let candidates = mine_candidates(&w, final_message, &[]);
        assert_eq!(candidates.len(), 1, "{candidates:?}");
        assert!(candidates[0].earlier.contains("fully reversible"));
        assert!(candidates[0].final_statement.contains("not reversible"));
        assert!(candidates[0].between.contains("2 tool call(s)"));
        assert!(candidates[0].between.contains("1 failed"));
    }

    #[test]
    fn paraphrase_without_polarity_opposition_is_not_a_candidate() {
        let w = window(vec![assistant(
            "The database migration is fully reversible and safe to run.",
        )]);
        let final_message = "The database migration is fully reversible and safe to execute.";
        assert!(mine_candidates(&w, final_message, &[]).is_empty());
    }

    #[test]
    fn unrelated_statements_are_not_candidates() {
        let w = window(vec![assistant(
            "The configuration loader reads environment variables at startup.",
        )]);
        let final_message = "The frontend bundle is not minified in development builds.";
        assert!(mine_candidates(&w, final_message, &[]).is_empty());
    }

    fn constraint_memory(content: &str) -> Memory {
        Memory {
            id: "m1".into(),
            content: content.to_string(),
            kind: Kind::Lesson,
            origin: "test".into(),
            external: false,
            trust: Trust::FirstHand,
            tags: vec![],
            embedding: vec![1.0, 0.0],
            embedding_model: "voyage-4".into(),
            created_at: DateTime::parse_from_rfc3339("2026-06-12T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn relevant_stored_decisions_become_candidates() {
        let memory = constraint_memory("Decision: deployments always go through staging first.");
        let w = window(vec![call(false)]);
        let candidates = mine_candidates(
            &w,
            "I will deploy this straight to production now since it is a small change.",
            &[(0.9, memory)],
        );
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].earlier.contains("staging"));
        assert!(candidates[0].between.contains("1 tool call(s)"));
    }

    #[test]
    fn candidate_cap_is_respected_and_empty_inputs_yield_empty() {
        // Many near-identical reversal pairs → capped.
        let earlier = "The cache layer is not enabled for write operations currently.";
        let finals = "The cache layer is enabled for write operations currently. \
                      The cache layer is enabled for write operations in production. \
                      The cache layer is enabled for write operations everywhere now. \
                      The cache layer is enabled for write operations by default here. \
                      The cache layer is enabled for write operations on every node.";
        let w = window(vec![assistant(earlier)]);
        let candidates = mine_candidates(&w, finals, &[]);
        assert!(candidates.len() <= REVIEW_CANDIDATES_MAX);
        assert!(!candidates.is_empty());

        assert!(mine_candidates(&window(vec![]), "", &[]).is_empty());
    }

    #[test]
    fn the_mode_schema_is_flat_and_registers() {
        let mut registry = ModeRegistry::new();
        register(&mut registry).unwrap();
        let mode = registry.get(REVIEW_MODE_ID).unwrap();
        assert_eq!(mode.ensemble_k, 1);
        assert!(mode.prompt_template.contains("<<candidates>>"));
        assert!(mode.prompt_template.contains("<<preferences>>"));
        // Decline bias and the FR-004(d) rule are pinned in the template.
        assert!(mode.prompt_template.contains("contradicts=false"));
        assert!(mode.prompt_template.contains("evidence"));
        assert!(mode.prompt_template.contains("NOT a contradiction"));
        // 015: the violation judgment's decline bias is pinned too.
        assert!(mode.prompt_template.contains("violates=false"));
        assert!(mode.prompt_template.contains("NOT violated"));
        // 015: the extended schema registers (the registry's boot invariant
        // enforces flat+closed) and carries the new violation fields.
        let properties = mode.output_schema["properties"].as_object().unwrap();
        for field in ["violates", "violated_preference", "violation_basis"] {
            assert!(properties.contains_key(field), "{field} missing");
        }
    }

    fn test_mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        register(&mut registry).unwrap();
        registry.get(REVIEW_MODE_ID).unwrap().clone()
    }

    fn candidate() -> Candidate {
        Candidate {
            earlier: "The migration is reversible.".into(),
            final_statement: "The migration is not reversible.".into(),
            between: "no tool activity since the earlier statement".into(),
        }
    }

    fn preference_candidate(id: &str, content: &str) -> PreferenceCandidate {
        PreferenceCandidate {
            memory_id: id.into(),
            content: content.into(),
            trust: Trust::FirstHand,
            score: 0.8,
        }
    }

    #[tokio::test]
    async fn a_confirmed_contradiction_yields_a_flag_citing_both_statements() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|prompt, _| {
            // The hop sees the bare candidates, numbered; with no
            // preferences mined, the preference section is "(none)".
            assert!(prompt.contains("1. earlier:"), "{prompt}");
            assert!(prompt.contains("(none)"), "{prompt}");
            Ok(Completion {
                value: json!({
                    "contradicts": true,
                    "statement_a": "The migration is reversible.",
                    "statement_b": "The migration is not reversible.",
                    "basis": "Both cannot hold as written.",
                    "violates": false,
                    "violated_preference": "",
                    "violation_basis": ""
                }),
                input_tokens: 50,
                output_tokens: 20,
            })
        });
        let (flagged, inp, out) = review_once(
            &client,
            &test_mode(),
            &[candidate()],
            &[],
            "The migration is not reversible.",
            "no tool activity in this turn's window",
        )
        .await
        .unwrap();
        assert_eq!((inp, out), (50, 20));
        assert_eq!(flagged.len(), 1);
        let (signal, message) = &flagged[0];
        assert_eq!(signal.kind, SignalKind::SelfContradiction);
        assert!(message.contains("The migration is reversible."));
        assert!(message.contains("The migration is not reversible."));
        assert!(message.contains("Both cannot hold"));
    }

    #[tokio::test]
    async fn a_cleared_review_yields_no_flag() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: json!({
                    "contradicts": false,
                    "statement_a": "",
                    "statement_b": "",
                    "basis": "The final statement is a refinement, not a reversal.",
                    "violates": false,
                    "violated_preference": "",
                    "violation_basis": ""
                }),
                input_tokens: 40,
                output_tokens: 15,
            })
        });
        let (flagged, _, _) = review_once(
            &client,
            &test_mode(),
            &[candidate()],
            &[],
            "The migration is not reversible in practice.",
            "no tool activity in this turn's window",
        )
        .await
        .unwrap();
        assert!(flagged.is_empty());
    }

    #[tokio::test]
    async fn a_confirmed_violation_maps_back_to_the_matched_candidate() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|prompt, _| {
            // The hop sees numbered preferences + the turn evidence.
            assert!(
                prompt.contains("1. \"never deploy on fridays\""),
                "{prompt}"
            );
            assert!(
                prompt.contains("2. \"always run the test gate before claiming done\""),
                "{prompt}"
            );
            assert!(prompt.contains("Turn final message (excerpt):"), "{prompt}");
            assert!(prompt.contains("Turn activity:"), "{prompt}");
            Ok(Completion {
                value: json!({
                    "contradicts": false,
                    "statement_a": "",
                    "statement_b": "",
                    "basis": "No contradiction among the pairs.",
                    "violates": true,
                    // Slightly reworded echo — map-back must still pick
                    // the test-gate candidate, and the flag must quote the
                    // SERVER-held content, not this echo.
                    "violated_preference": "always run the test gate before you claim done",
                    "violation_basis": "The turn claims completion with no test activity."
                }),
                input_tokens: 60,
                output_tokens: 25,
            })
        });
        let preferences = [
            preference_candidate("mem-friday", "never deploy on fridays"),
            preference_candidate("mem-gate", "always run the test gate before claiming done"),
        ];
        let (flagged, _, _) = review_once(
            &client,
            &test_mode(),
            &[],
            &preferences,
            "All done — everything works.",
            "no tool activity in this turn's window",
        )
        .await
        .unwrap();
        assert_eq!(flagged.len(), 1);
        let (signal, message) = &flagged[0];
        assert_eq!(signal.kind, SignalKind::PreferenceViolation);
        let (expected, _) = preference::violation_signal(
            &preferences[1],
            "The turn claims completion with no test activity.",
        );
        assert_eq!(signal.signal_key, expected.signal_key);
        assert!(
            message.contains("always run the test gate before claiming done"),
            "{message}"
        );
        assert!(message.contains("mem-gate"), "{message}");
        assert!(message.contains("first_hand"), "{message}");
        assert!(!message.contains("before you claim done"), "{message}");
    }

    #[tokio::test]
    async fn both_judgments_firing_yield_two_flags_contradiction_first() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: json!({
                    "contradicts": true,
                    "statement_a": "The migration is reversible.",
                    "statement_b": "The migration is not reversible.",
                    "basis": "Both cannot hold as written.",
                    "violates": true,
                    "violated_preference": "never deploy on fridays",
                    "violation_basis": "The turn deployed on a friday."
                }),
                input_tokens: 70,
                output_tokens: 30,
            })
        });
        let preferences = [preference_candidate(
            "mem-friday",
            "never deploy on fridays",
        )];
        let (flagged, _, _) = review_once(
            &client,
            &test_mode(),
            &[candidate()],
            &preferences,
            "Deployed; the migration is not reversible.",
            "1 tool call(s) in this turn's window (0 failed; tools: bash)",
        )
        .await
        .unwrap();
        assert_eq!(flagged.len(), 2);
        assert_eq!(flagged[0].0.kind, SignalKind::SelfContradiction);
        assert_eq!(flagged[1].0.kind, SignalKind::PreferenceViolation);
    }

    #[tokio::test]
    async fn a_violates_claim_with_no_listed_preferences_is_ignored() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: json!({
                    "contradicts": false,
                    "statement_a": "",
                    "statement_b": "",
                    "basis": "Consistent.",
                    "violates": true,
                    "violated_preference": "a preference that was never listed",
                    "violation_basis": "hallucinated"
                }),
                input_tokens: 30,
                output_tokens: 10,
            })
        });
        let (flagged, _, _) = review_once(
            &client,
            &test_mode(),
            &[candidate()],
            &[],
            "Nothing notable.",
            "no tool activity in this turn's window",
        )
        .await
        .unwrap();
        assert!(flagged.is_empty());
    }
}
