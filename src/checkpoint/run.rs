//! Per-boundary orchestration: validate → screen → (review) → assemble →
//! record.
//!
//! Fail-open is the spine (FR-008): any evaluation failure becomes a
//! recorded silence-with-`fail_open`, never an error that could block the
//! session. Only the audit write itself propagates — the harness's hooks
//! fail open on errors, so a broken store degrades to no-op there too.

use crate::checkpoint::contract::{
    CheckpointActionParams, CheckpointBatchParams, CheckpointResult, CheckpointTurnParams,
};
use crate::checkpoint::{
    gate, review, screen, Boundary, CheckpointRecord, Signal, SignalKind, COOLDOWN_WINDOW_MS,
    GATE_BUDGET_MS,
};
use crate::error::AppError;
use crate::modes::CorrectiveMode;
use crate::telemetry;
use crate::traits::client::ModelClient;
use crate::traits::clock::TimeProvider;
use crate::traits::embedder::Embedder;
use crate::traits::storage::Storage;
use crate::traits::trajectory::TrajectoryReader;
use chrono::Duration;
use std::sync::Arc;
use std::time::Instant;

/// Everything the three checkpoint boundaries need.
pub struct CheckpointDeps {
    /// Bounded, validated trajectory access (the seventh seam).
    pub reader: Arc<dyn TrajectoryReader>,
    /// Records + cooldown lookups + stored memories.
    pub storage: Arc<dyn Storage>,
    /// Cooldown windows and record timestamps.
    pub clock: Arc<dyn TimeProvider>,
    /// For the end-of-turn review hop only.
    pub model_client: Arc<dyn ModelClient>,
    /// The registered review mode.
    pub review_mode: CorrectiveMode,
    /// Anthropic model id (review-hop cost attribution on checkpoint records).
    pub model: String,
    /// Gate/turn recall — `None` when the memory capability is disabled
    /// (memory-paired signals silently inactive, FR-004/spec edge case).
    pub embedder: Option<Arc<dyn Embedder>>,
    /// `CHECKPOINT_GATE_PATTERNS` extras (FR-013).
    pub gate_extra_patterns: Vec<String>,
}

/// What one evaluation concluded, before recording.
struct Evaluated {
    signal_kinds: Vec<SignalKind>,
    fired: Vec<Signal>,
    /// Keys actually delivered (FR-010 cooldown feed) — the unsuppressed
    /// subset for flags, the held signal for holds, empty otherwise.
    delivered_keys: Vec<String>,
    review_ran: bool,
    result: CheckpointResult,
    cost_usd: f64,
    input_tokens: u64,
    output_tokens: u64,
}

impl Evaluated {
    fn pure(signal_kinds: Vec<SignalKind>, fired: Vec<Signal>, result: CheckpointResult) -> Self {
        Self {
            signal_kinds,
            fired,
            delivered_keys: vec![],
            review_ran: false,
            result,
            cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
        }
    }
}

/// `checkpoint_batch` (US1): deterministic loop/repeated-failure screening.
///
/// # Errors
///
/// Only the audit write propagates; every evaluation failure is a recorded
/// fail-open silence (FR-008).
pub async fn run_batch(
    deps: &CheckpointDeps,
    params: &CheckpointBatchParams,
) -> Result<(CheckpointResult, u64, u64), AppError> {
    let started = Instant::now();
    let evaluated_kinds = vec![SignalKind::Repetition, SignalKind::RepeatedFailure];

    let evaluation = async {
        let window = deps
            .reader
            .read(&params.transcript_path, &params.session_id)
            .await?;
        let fired = screen::screen(&window);
        if fired.is_empty() {
            return Ok(Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            ));
        }
        let remaining = unsuppressed(deps, &params.session_id, &fired).await?;
        let (result, delivered_keys) = if remaining.is_empty() {
            (
                CheckpointResult::suppressed(&fired, elapsed_ms(started)),
                vec![],
            )
        } else {
            (
                CheckpointResult::flag(
                    batch_flag_message(&remaining),
                    &remaining,
                    elapsed_ms(started),
                ),
                remaining.iter().map(|s| s.signal_key.clone()).collect(),
            )
        };
        let mut evaluation = Evaluated::pure(evaluated_kinds.clone(), fired, result);
        evaluation.delivered_keys = delivered_keys;
        Ok::<Evaluated, AppError>(evaluation)
    }
    .await;

    let evaluation = recover(evaluation, evaluated_kinds, started, "checkpoint_batch");
    record(deps, Boundary::Batch, &params.session_id, &evaluation).await?;
    Ok((
        evaluation.result,
        evaluation.input_tokens,
        evaluation.output_tokens,
    ))
}

/// `checkpoint_action` (US2): the risk-matched, deterministic pre-action gate
/// under a hard budget.
///
/// # Errors
///
/// Only the audit write propagates (see [`run_batch`]).
pub async fn run_action(
    deps: &CheckpointDeps,
    params: &CheckpointActionParams,
) -> Result<(CheckpointResult, u64, u64), AppError> {
    let started = Instant::now();

    // FR-013: non-risk-matched actions pass with no evaluation at all.
    if !gate::risk_matched(
        &params.tool_name,
        &params.tool_input,
        &deps.gate_extra_patterns,
    ) {
        let evaluation = Evaluated::pure(
            vec![],
            vec![],
            CheckpointResult::silence(elapsed_ms(started)),
        );
        record(deps, Boundary::Action, &params.session_id, &evaluation).await?;
        return Ok((evaluation.result, 0, 0));
    }

    let evaluated_kinds = vec![SignalKind::MemoryConflict];
    let evaluation = async {
        // Memory capability absent → the signal is silently inactive
        // (spec edge case), not a failure.
        let Some(embedder) = deps.embedder.as_ref() else {
            return Ok(Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            ));
        };
        // The whole gate decision sits under the hard budget (FR-009).
        let work = async {
            let query = embedder
                .embed_query(&format!("{} {}", params.tool_name, params.tool_input))
                .await?;
            let memories = deps.storage.load_memories().await?;
            Ok::<_, AppError>((
                gate::constraint_hold(&query.vector, &memories),
                query.input_tokens,
            ))
        };
        let (held, embed_tokens) =
            tokio::time::timeout(std::time::Duration::from_millis(GATE_BUDGET_MS), work)
                .await
                .map_err(|_| AppError::Timeout {
                    what: "pre-action gate",
                    ms: GATE_BUDGET_MS,
                })??;

        let mut evaluation = match held {
            // FR-011: holds escalate to the user; FR-010: never rate-limited.
            Some((signal, memory_content)) => {
                let delivered_keys = vec![signal.signal_key.clone()];
                let fired = vec![signal];
                let result = CheckpointResult::hold(
                    hold_message(&memory_content),
                    &fired,
                    elapsed_ms(started),
                );
                let mut held_evaluation = Evaluated::pure(evaluated_kinds.clone(), fired, result);
                held_evaluation.delivered_keys = delivered_keys;
                held_evaluation
            }
            None => Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            ),
        };
        evaluation.input_tokens = embed_tokens;
        Ok::<Evaluated, AppError>(evaluation)
    }
    .await;

    let evaluation = recover(evaluation, evaluated_kinds, started, "checkpoint_action");
    record(deps, Boundary::Action, &params.session_id, &evaluation).await?;
    Ok((
        evaluation.result,
        evaluation.input_tokens,
        evaluation.output_tokens,
    ))
}

/// `checkpoint_turn` (US3): deterministic candidate mining gating at most
/// one blind review hop; flags deliver as forced continuation (FR-014).
///
/// # Errors
///
/// Only the audit write propagates (see [`run_batch`]).
pub async fn run_turn(
    deps: &CheckpointDeps,
    params: &CheckpointTurnParams,
) -> Result<(CheckpointResult, u64, u64), AppError> {
    let started = Instant::now();

    // FR-014: a turn end that follows a forced continuation never reviews
    // again — continuation cannot loop.
    if params.continuation {
        let evaluation = Evaluated::pure(
            vec![],
            vec![],
            CheckpointResult::silence(elapsed_ms(started)),
        );
        record(deps, Boundary::Turn, &params.session_id, &evaluation).await?;
        return Ok((evaluation.result, 0, 0));
    }

    let evaluated_kinds = vec![SignalKind::SelfContradiction];
    let evaluation = async {
        if params.final_message.trim().is_empty() {
            return Ok(Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            ));
        }
        let window = deps
            .reader
            .read(&params.transcript_path, &params.session_id)
            .await?;
        let (recall, mut input_tokens) = turn_recall(deps, &params.final_message).await?;
        let candidates = review::mine_candidates(&window, &params.final_message, &recall);
        if candidates.is_empty() {
            // US3-AS2: no candidates → no model pass.
            let mut evaluation = Evaluated::pure(
                evaluated_kinds.clone(),
                vec![],
                CheckpointResult::silence(elapsed_ms(started)),
            );
            evaluation.input_tokens = input_tokens;
            return Ok(evaluation);
        }

        let (flagged, inp, out) =
            review::review_once(deps.model_client.as_ref(), &deps.review_mode, &candidates).await?;
        input_tokens += inp;
        let cost_usd = telemetry::cost_usd(&deps.model, inp, out);

        let mut delivered_keys: Vec<String> = vec![];
        let (fired, result) = match flagged {
            None => (vec![], CheckpointResult::silence(elapsed_ms(started))),
            Some((signal, message)) => {
                let fired = vec![signal];
                let remaining = unsuppressed(deps, &params.session_id, &fired).await?;
                let result = if remaining.is_empty() {
                    CheckpointResult::suppressed(&fired, elapsed_ms(started))
                } else {
                    delivered_keys = remaining.iter().map(|s| s.signal_key.clone()).collect();
                    CheckpointResult::flag(message, &remaining, elapsed_ms(started))
                };
                (fired, result)
            }
        };
        Ok::<Evaluated, AppError>(Evaluated {
            signal_kinds: evaluated_kinds.clone(),
            fired,
            delivered_keys,
            review_ran: true,
            result,
            cost_usd,
            input_tokens,
            output_tokens: out,
        })
    }
    .await;

    let evaluation = recover(evaluation, evaluated_kinds, started, "checkpoint_turn");
    record(deps, Boundary::Turn, &params.session_id, &evaluation).await?;
    Ok((
        evaluation.result,
        evaluation.input_tokens,
        evaluation.output_tokens,
    ))
}

/// Recall for the turn review: final message embedded, memories ranked.
async fn turn_recall(
    deps: &CheckpointDeps,
    final_message: &str,
) -> Result<(Vec<(f32, crate::memory::Memory)>, u64), AppError> {
    let Some(embedder) = deps.embedder.as_ref() else {
        return Ok((vec![], 0));
    };
    let query = embedder.embed_query(final_message).await?;
    let memories = deps.storage.load_memories().await?;
    Ok((
        review::rank_recall(&query.vector, &memories),
        query.input_tokens,
    ))
}

/// FR-008: an evaluation failure becomes a recorded fail-open silence.
fn recover(
    evaluation: Result<Evaluated, AppError>,
    evaluated: Vec<SignalKind>,
    started: Instant,
    boundary: &str,
) -> Evaluated {
    evaluation.unwrap_or_else(|error| {
        tracing::warn!(boundary, %error, "checkpoint evaluation failed open");
        let mut failed = Evaluated::pure(
            evaluated,
            vec![],
            CheckpointResult::fail_open(elapsed_ms(started)),
        );
        failed.review_ran = false;
        failed
    })
}

/// FR-010: drop signals whose key was delivered within the cooldown window.
async fn unsuppressed(
    deps: &CheckpointDeps,
    session_id: &str,
    fired: &[Signal],
) -> Result<Vec<Signal>, AppError> {
    let since = deps.clock.now() - Duration::milliseconds(COOLDOWN_WINDOW_MS);
    let delivered = deps
        .storage
        .delivered_signal_keys_since(session_id, since)
        .await?;
    Ok(fired
        .iter()
        .filter(|signal| !delivered.contains(&signal.signal_key))
        .cloned()
        .collect())
}

/// Exactly one audit row per evaluation (FR-006).
async fn record(
    deps: &CheckpointDeps,
    boundary: Boundary,
    session_id: &str,
    evaluation: &Evaluated,
) -> Result<(), AppError> {
    deps.storage
        .record_checkpoint(&CheckpointRecord {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            boundary,
            signals_evaluated: evaluation.signal_kinds.clone(),
            signals_fired: evaluation.fired.clone(),
            delivered_keys: evaluation.delivered_keys.clone(),
            review_ran: evaluation.review_ran,
            verdict: evaluation.result.verdict,
            suppressed: evaluation.result.suppressed,
            fail_open: evaluation.result.fail_open,
            latency_ms: evaluation.result.latency_ms,
            cost_usd: evaluation.cost_usd,
            created_at: deps.clock.now(),
        })
        .await
}

/// Fixed message templates (FR-005/SC-007): parameterized only by evidence.
fn batch_flag_message(signals: &[Signal]) -> String {
    let evidence: Vec<&str> = signals.iter().map(|s| s.evidence.as_str()).collect();
    format!(
        "Trajectory checkpoint (automated): {}. This pattern suggests the current \
         approach is looping — change the approach (or call the unstick corrective) \
         instead of repeating it.",
        evidence.join("; ")
    )
}

fn hold_message(memory_content: &str) -> String {
    format!(
        "Checkpoint hold (automated): this action appears to conflict with a stored, \
         verified constraint: \"{memory_content}\". Confirm to run it unchanged, or \
         deny it to let the agent course-correct."
    )
}

#[allow(clippy::cast_possible_truncation)] // checkpoint latencies are far below u64::MAX ms
fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::checkpoint::trajectory::{TrajectoryEntry, TrajectoryWindow};
    use crate::checkpoint::Verdict;
    use crate::modes::ModeRegistry;
    use crate::traits::client::{Completion, MockModelClient};
    use crate::traits::clock::MockTimeProvider;
    use crate::traits::embedder::{Embedding, MockEmbedder};
    use crate::traits::storage::MockStorage;
    use crate::traits::trajectory::MockTrajectoryReader;
    use chrono::{DateTime, Utc};
    use serde_json::json;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-06-12T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn review_mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        review::register(&mut registry).unwrap();
        registry.get(review::REVIEW_MODE_ID).unwrap().clone()
    }

    fn looping_window() -> TrajectoryWindow {
        TrajectoryWindow {
            session_id: "s1".into(),
            entries: (1..=4)
                .map(|i| TrajectoryEntry::ToolCall {
                    batch_index: i,
                    tool_name: "bash".into(),
                    normalized_input: "{command=cargo test;}".into(),
                    failed: false,
                })
                .collect(),
        }
    }

    struct DepsBuilder {
        reader: MockTrajectoryReader,
        storage: MockStorage,
        clock: MockTimeProvider,
        client: MockModelClient,
        embedder: Option<Arc<dyn Embedder>>,
        patterns: Vec<String>,
    }

    impl DepsBuilder {
        fn new() -> Self {
            let mut clock = MockTimeProvider::new();
            clock.expect_now().return_const(fixed_now());
            Self {
                reader: MockTrajectoryReader::new(),
                storage: MockStorage::new(),
                clock,
                client: MockModelClient::new(),
                embedder: None,
                patterns: vec![],
            }
        }

        fn build(self) -> CheckpointDeps {
            CheckpointDeps {
                reader: Arc::new(self.reader),
                storage: Arc::new(self.storage),
                clock: Arc::new(self.clock),
                model_client: Arc::new(self.client),
                review_mode: review_mode(),
                model: "claude-opus-4-8".into(),
                embedder: self.embedder,
                gate_extra_patterns: self.patterns,
            }
        }
    }

    fn batch_params() -> CheckpointBatchParams {
        CheckpointBatchParams {
            session_id: "s1".into(),
            transcript_path: "t.jsonl".into(),
        }
    }

    #[tokio::test]
    async fn batch_flags_a_loop_and_records_exactly_once() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok(looping_window()));
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(|_, _| Ok(vec![]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.boundary == Boundary::Batch
                    && r.verdict == Verdict::Flag
                    && !r.suppressed
                    && !r.fail_open
                    && r.signals_fired.len() == 1
            })
            .returning(|_| Ok(()));

        let (result, _, _) = run_batch(&b.build(), &batch_params()).await.unwrap();
        assert_eq!(result.verdict, Verdict::Flag);
        let message = result.message.unwrap();
        assert!(message.contains("cargo test"), "{message}");
        assert!(message.contains("4 times"), "{message}");
    }

    #[tokio::test]
    async fn batch_cooldown_suppresses_an_already_delivered_signal() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok(looping_window()));
        // The same signal key was delivered recently.
        let key = screen::screen(&looping_window())[0].signal_key.clone();
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(move |_, _| Ok(vec![key.clone()]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && r.suppressed)
            .returning(|_| Ok(()));

        let (result, _, _) = run_batch(&b.build(), &batch_params()).await.unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(result.suppressed);
        assert!(result.message.is_none());
    }

    #[tokio::test]
    async fn batch_fails_open_on_a_reader_error_and_still_records() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Err(AppError::ValidationFailure("no transcript".into())));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.fail_open && r.verdict == Verdict::Silence)
            .returning(|_| Ok(()));

        let (result, _, _) = run_batch(&b.build(), &batch_params()).await.unwrap();
        assert!(result.fail_open);
        assert_eq!(result.verdict, Verdict::Silence);
    }

    fn action_params(tool: &str, input: &str) -> CheckpointActionParams {
        CheckpointActionParams {
            session_id: "s1".into(),
            transcript_path: "t.jsonl".into(),
            tool_name: tool.into(),
            tool_input: input.into(),
        }
    }

    fn constraint_memory() -> crate::memory::Memory {
        crate::memory::Memory {
            id: "m1".into(),
            content: "deployments go through staging first — never straight to production".into(),
            kind: crate::memory::Kind::Lesson,
            origin: "test".into(),
            external: false,
            trust: crate::memory::Trust::FirstHand,
            tags: vec![],
            embedding: vec![1.0, 0.0],
            embedding_model: "voyage-4".into(),
            created_at: fixed_now(),
        }
    }

    #[tokio::test]
    async fn gate_holds_a_risky_action_quoting_the_memory() {
        let mut b = DepsBuilder::new();
        let mut embedder = MockEmbedder::new();
        embedder.expect_embed_query().returning(|_| {
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 6,
            })
        });
        b.embedder = Some(Arc::new(embedder));
        b.storage
            .expect_load_memories()
            .returning(|| Ok(vec![constraint_memory()]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.boundary == Boundary::Action && r.verdict == Verdict::Hold)
            .returning(|_| Ok(()));

        let (result, inp, _) =
            run_action(&b.build(), &action_params("bash", "deploy to production"))
                .await
                .unwrap();
        assert_eq!(result.verdict, Verdict::Hold);
        assert!(result.message.unwrap().contains("staging first"));
        assert_eq!(inp, 6); // embed usage attributed
    }

    #[tokio::test]
    async fn gate_passes_non_risk_actions_without_any_evaluation() {
        let mut b = DepsBuilder::new();
        let mut embedder = MockEmbedder::new();
        embedder.expect_embed_query().times(0);
        b.embedder = Some(Arc::new(embedder));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && r.signals_evaluated.is_empty())
            .returning(|_| Ok(()));

        let (result, _, _) = run_action(&b.build(), &action_params("read", "src/main.rs"))
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(!result.fail_open);
    }

    /// An embedder that outlives the gate budget — the timeout test double.
    struct SlowEmbedder;

    #[async_trait::async_trait]
    impl Embedder for SlowEmbedder {
        async fn embed_document(&self, _text: &str) -> Result<Embedding, AppError> {
            unreachable!("the gate never embeds documents")
        }

        async fn embed_query(&self, _text: &str) -> Result<Embedding, AppError> {
            tokio::time::sleep(std::time::Duration::from_millis(GATE_BUDGET_MS + 200)).await;
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 6,
            })
        }

        fn model_id(&self) -> &'static str {
            "voyage-4"
        }
    }

    #[tokio::test]
    async fn gate_fails_open_when_the_budget_elapses() {
        let mut b = DepsBuilder::new();
        b.embedder = Some(Arc::new(SlowEmbedder));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.fail_open)
            .returning(|_| Ok(()));

        let (result, _, _) = run_action(&b.build(), &action_params("bash", "git push --force"))
            .await
            .unwrap();
        assert!(result.fail_open);
        assert_eq!(result.verdict, Verdict::Silence);
    }

    #[tokio::test]
    async fn gate_without_memory_capability_is_silent_not_failed() {
        let mut b = DepsBuilder::new();
        b.embedder = None;
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.verdict == Verdict::Silence
                    && !r.fail_open
                    && r.signals_evaluated == vec![SignalKind::MemoryConflict]
            })
            .returning(|_| Ok(()));

        let (result, _, _) = run_action(&b.build(), &action_params("bash", "git push --force"))
            .await
            .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(!result.fail_open);
    }

    fn turn_params(final_message: &str, continuation: bool) -> CheckpointTurnParams {
        CheckpointTurnParams {
            session_id: "s1".into(),
            transcript_path: "t.jsonl".into(),
            final_message: final_message.into(),
            continuation,
        }
    }

    fn reversal_window() -> TrajectoryWindow {
        TrajectoryWindow {
            session_id: "s1".into(),
            entries: vec![TrajectoryEntry::Assistant {
                text: "The database migration is fully reversible and safe to run.".into(),
            }],
        }
    }

    #[tokio::test]
    async fn turn_flags_a_confirmed_contradiction_citing_both_statements() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok(reversal_window()));
        b.client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: json!({
                    "contradicts": true,
                    "statement_a": "The database migration is fully reversible and safe to run",
                    "statement_b": "the database migration is not reversible",
                    "basis": "Both cannot hold."
                }),
                input_tokens: 80,
                output_tokens: 30,
            })
        });
        b.storage
            .expect_delivered_signal_keys_since()
            .returning(|_, _| Ok(vec![]));
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| {
                r.boundary == Boundary::Turn
                    && r.verdict == Verdict::Flag
                    && r.review_ran
                    && r.cost_usd > 0.0
            })
            .returning(|_| Ok(()));

        let (result, inp, out) = run_turn(
            &b.build(),
            &turn_params(
                "After checking, the database migration is not reversible after all.",
                false,
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Flag);
        let message = result.message.unwrap();
        assert!(message.contains("fully reversible"), "{message}");
        assert!(message.contains("not reversible"), "{message}");
        assert_eq!((inp, out), (80, 30));
    }

    #[tokio::test]
    async fn turn_with_no_candidates_never_invokes_the_hop() {
        let mut b = DepsBuilder::new();
        b.reader.expect_read().returning(|_, _| {
            Ok(TrajectoryWindow {
                session_id: "s1".into(),
                entries: vec![],
            })
        });
        b.client.expect_complete().times(0);
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && !r.review_ran)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("Everything completed without surprises today.", false),
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
    }

    #[tokio::test]
    async fn turn_screening_cleared_records_review_ran_with_silence() {
        let mut b = DepsBuilder::new();
        b.reader
            .expect_read()
            .returning(|_, _| Ok(reversal_window()));
        b.client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: json!({
                    "contradicts": false,
                    "statement_a": "",
                    "statement_b": "",
                    "basis": "The final statement is an evidence-justified update."
                }),
                input_tokens: 60,
                output_tokens: 20,
            })
        });
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && r.review_ran && !r.suppressed)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params(
                "On reflection the database migration is not reversible in practice.",
                false,
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
    }

    #[tokio::test]
    async fn continuation_turns_are_screening_only_and_never_review() {
        let mut b = DepsBuilder::new();
        b.reader.expect_read().times(0);
        b.client.expect_complete().times(0);
        b.storage
            .expect_record_checkpoint()
            .times(1)
            .withf(|r| r.verdict == Verdict::Silence && !r.review_ran)
            .returning(|_| Ok(()));

        let (result, _, _) = run_turn(
            &b.build(),
            &turn_params("Reconciled: the migration is not reversible.", true),
        )
        .await
        .unwrap();
        assert_eq!(result.verdict, Verdict::Silence);
    }
}
