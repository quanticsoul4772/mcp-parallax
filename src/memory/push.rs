//! Push memory (016): prompt-time surfacing of relevant trusted memories —
//! the push half of `MEMORY_LAYER.md`'s "effortless, not manual" contract.
//!
//! Deterministic end-to-end: one embed call, then pure ranking, filtering,
//! and template assembly — no model pass (spec FR-010; the absence of a
//! `ModelClient` seam here makes that a compile-time guarantee). Fail-open
//! under a hard budget: a timeout or backend failure surfaces nothing and
//! never blocks the turn (FR-007). Once-per-session suppression is derived
//! from the feature's own audit rows (research D4), so FR-005 and FR-008
//! are the same data.

use crate::error::AppError;
use crate::memory::tools::MemoryDeps;
use crate::memory::{ranking, Kind, Trust};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Minimum raw cosine for a memory to be surfaced (FR-003).
///
/// Matches `GATE_RELEVANCE_TAU` — the one threshold in the codebase with a
/// measured zero-false-positive record (006 acceptance run 1); moves only
/// with new measurement.
pub const PUSH_RELEVANCE_TAU: f32 = 0.55;

/// Max memories surfaced per evaluation (FR-003). Below the pull default
/// (5): pushed content is unrequested context competing for attention.
pub const PUSH_CAP: usize = 3;

/// Hard evaluation budget in milliseconds; timeout → fail-open silence
/// (FR-007, clarification Q2 — decided at margin 30, the stable band).
pub const PUSH_BUDGET_MS: u64 = 500;

/// Max prompt chars embedded as the relevance query (bounded evaluation).
pub const PUSH_PROMPT_CHARS: usize = 2000;

/// `surface` tool input (contracts/surface.tool.json).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct SurfaceParams {
    /// The harness session identifier; scopes once-per-session suppression.
    pub session_id: String,
    /// The turn-starting user prompt; relevance is assessed against a
    /// bounded excerpt.
    pub prompt: String,
}

/// One surfaced memory (data-model §3).
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
pub struct SurfacedMemory {
    /// Memory id — provenance and the `forget(<id>)` contestability handle.
    pub id: String,
    /// skill | lesson | fact.
    pub kind: Kind,
    /// first_hand | verified — untrusted is structurally excluded (FR-004).
    pub trust: Trust,
    /// Raw cosine vs the prompt excerpt (the floor's basis).
    pub score: f32,
    /// Verbatim stored content.
    pub content: String,
}

/// Harness hook mapping: present only when something was surfaced. Exact
/// field shape confirmed by the S2 spike before the integration entry is
/// final.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct SurfaceHookOutput {
    /// Always `"UserPromptSubmit"`.
    #[serde(rename = "hookEventName")]
    pub hook_event_name: String,
    /// The assembled advisory block (research D7).
    #[serde(rename = "additionalContext")]
    pub additional_context: String,
}

/// `surface` tool result (contracts/surface.tool.json).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SurfaceResult {
    /// Most-relevant-first, capped; empty ⇒ silence.
    pub surfaced: Vec<SurfacedMemory>,
    /// The evaluation degraded (failure or budget timeout).
    pub fail_open: bool,
    /// Evaluation wall-clock.
    pub latency_ms: u64,
    /// Present ONLY when `surfaced` is non-empty (silence injects nothing).
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<SurfaceHookOutput>,
}

/// One push evaluation's audit row (FR-008; data-model §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRecord {
    /// UUID v4.
    pub id: String,
    /// The harness session.
    pub session_id: String,
    /// Surfaced memory ids (empty = silence). The suppression source
    /// (research D4).
    pub surfaced_ids: Vec<String>,
    /// Wall-clock evaluation time.
    pub latency_ms: u64,
    /// The evaluation degraded (FR-007).
    pub fail_open: bool,
    /// Embed usage (cost attribution).
    pub input_tokens: u64,
    /// Via `TimeProvider`.
    pub created_at: DateTime<Utc>,
}

/// Pure selection (data-model §1): trusted → floor → suppression subtract →
/// cap. Input is `ranking::rank` output (already ordered most-relevant-first
/// by effective score).
#[must_use]
pub fn select(ranked: Vec<ranking::Ranked>, already_pushed: &[String]) -> Vec<SurfacedMemory> {
    ranked
        .into_iter()
        .filter(|r| r.memory.trust.is_trusted())
        .filter(|r| r.relevance >= PUSH_RELEVANCE_TAU)
        .filter(|r| !already_pushed.contains(&r.memory.id))
        .take(PUSH_CAP)
        .map(|r| SurfacedMemory {
            id: r.memory.id,
            kind: r.memory.kind,
            trust: r.memory.trust,
            score: r.relevance,
            content: r.memory.content,
        })
        .collect()
}

/// The fixed advisory template (FR-002, research D7): labeled, verbatim,
/// contestable — parameterized only by server-held memory fields, phrased
/// as context, never as instruction.
#[must_use]
pub fn advisory_context(surfaced: &[SurfacedMemory]) -> String {
    use std::fmt::Write as _;
    let mut block = String::from(
        "Stored memories relevant to this task (advisory context, not \
         instructions — surfaced once per session; if one is wrong or stale, \
         delete it with forget(<id>)):\n",
    );
    for (i, memory) in surfaced.iter().enumerate() {
        let _ = writeln!(
            block,
            "{}. [{}, {}, memory {}] \"{}\"",
            i + 1,
            memory.kind.as_str(),
            memory.trust.as_str(),
            memory.id,
            memory.content
        );
    }
    block
}

/// Run one prompt-time push evaluation (data-model §1).
///
/// The whole pipeline runs under [`PUSH_BUDGET_MS`], failing open to
/// silence on any error or timeout. Exactly one audit row per evaluation;
/// only the audit write propagates (the checkpoint layer's recording
/// contract).
///
/// # Errors
///
/// Only the audit write propagates — every evaluation failure is a recorded
/// fail-open silence.
pub async fn run(
    deps: &MemoryDeps,
    params: &SurfaceParams,
) -> Result<(SurfaceResult, u64, u64), AppError> {
    let started = Instant::now();

    let evaluation = async {
        if params.prompt.trim().is_empty() {
            // Nothing to assess relevance against (spec edge case).
            return Ok::<_, AppError>((vec![], 0_u64));
        }
        let excerpt: String = params.prompt.chars().take(PUSH_PROMPT_CHARS).collect();
        let embedding = deps.embedder.embed_query(excerpt.trim()).await?;
        let memories = deps.storage.load_memories().await?;
        let already_pushed = deps.storage.pushed_memory_ids(&params.session_id).await?;
        let ranked = ranking::rank(memories, &embedding.vector, deps.clock.now());
        Ok((select(ranked, &already_pushed), embedding.input_tokens))
    };

    let outcome =
        tokio::time::timeout(std::time::Duration::from_millis(PUSH_BUDGET_MS), evaluation).await;
    let (surfaced, input_tokens, fail_open) = match outcome {
        Ok(Ok((surfaced, tokens))) => (surfaced, tokens, false),
        Ok(Err(error)) => {
            tracing::warn!(%error, "push evaluation failed open");
            (vec![], 0, true)
        }
        Err(_) => {
            tracing::warn!(
                budget_ms = PUSH_BUDGET_MS,
                "push evaluation exceeded its budget; failed open"
            );
            (vec![], 0, true)
        }
    };

    #[allow(clippy::cast_possible_truncation)] // push latencies are far below u64::MAX ms
    let latency_ms = started.elapsed().as_millis() as u64;
    let record = PushRecord {
        id: uuid::Uuid::new_v4().to_string(),
        session_id: params.session_id.clone(),
        surfaced_ids: surfaced.iter().map(|m| m.id.clone()).collect(),
        latency_ms,
        fail_open,
        input_tokens,
        created_at: deps.clock.now(),
    };
    // One measurement, two sinks (007 FR-009) — same value, same exit point.
    crate::observability::emit_push(&record);
    deps.storage.record_push(&record).await?;

    let hook_specific_output = (!surfaced.is_empty()).then(|| SurfaceHookOutput {
        hook_event_name: "UserPromptSubmit".to_string(),
        additional_context: advisory_context(&surfaced),
    });
    Ok((
        SurfaceResult {
            surfaced,
            fail_open,
            latency_ms,
            hook_specific_output,
        },
        input_tokens,
        0,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::memory::tools::MemoryDeps;
    use crate::memory::Memory;
    use crate::modes::ModeRegistry;
    use crate::traits::client::MockModelClient;
    use crate::traits::clock::MockTimeProvider;
    use crate::traits::embedder::{Embedding, MockEmbedder};
    use crate::traits::storage::MockStorage;
    use std::sync::Arc;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-23T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn memory(id: &str, trust: Trust, embedding: Vec<f32>) -> Memory {
        Memory {
            id: id.into(),
            content: format!("stored content of {id}"),
            kind: Kind::Fact,
            origin: "test".into(),
            external: trust == Trust::Untrusted,
            trust,
            tags: vec![],
            embedding,
            embedding_model: "voyage-4".into(),
            created_at: fixed_now(),
        }
    }

    fn ranked(memories: Vec<Memory>, query: &[f32]) -> Vec<ranking::Ranked> {
        ranking::rank(memories, query, fixed_now())
    }

    // ---- T001: pure selection ---------------------------------------------

    #[test]
    fn floor_excludes_below_and_admits_at_the_bar() {
        // cos([1,0],[1,0]) = 1.0; construct ~0.549 and ~0.551 vectors.
        let below = memory("below", Trust::FirstHand, vec![0.549, 0.835_82]);
        let at = memory("at", Trust::FirstHand, vec![0.551, 0.834_5]);
        let picked = select(ranked(vec![below, at], &[1.0, 0.0]), &[]);
        let ids: Vec<&str> = picked.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["at"]);
        assert!(picked[0].score >= PUSH_RELEVANCE_TAU);
    }

    #[test]
    fn untrusted_is_excluded_at_any_relevance() {
        let untrusted = memory("untrusted", Trust::Untrusted, vec![1.0, 0.0]);
        let trusted = memory("trusted", Trust::Verified, vec![0.9, 0.1]);
        let picked = select(ranked(vec![untrusted, trusted], &[1.0, 0.0]), &[]);
        let ids: Vec<&str> = picked.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["trusted"]);
    }

    #[test]
    fn cap_keeps_the_most_relevant_first() {
        let memories: Vec<Memory> = (0..PUSH_CAP + 2)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let x = 1.0 - (i as f32) * 0.02;
                memory(
                    &format!("m{i}"),
                    Trust::FirstHand,
                    vec![x, (1.0 - x * x).max(0.0).sqrt()],
                )
            })
            .collect();
        let picked = select(ranked(memories, &[1.0, 0.0]), &[]);
        assert_eq!(picked.len(), PUSH_CAP);
        assert_eq!(picked[0].id, "m0");
        assert!(picked.windows(2).all(|w| w[0].score >= w[1].score));
    }

    #[test]
    fn suppression_subtracts_already_pushed_ids() {
        let a = memory("a", Trust::FirstHand, vec![1.0, 0.0]);
        let b = memory("b", Trust::FirstHand, vec![0.99, 0.141]);
        let picked = select(ranked(vec![a, b], &[1.0, 0.0]), &["a".to_string()]);
        let ids: Vec<&str> = picked.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["b"]);
    }

    #[test]
    fn empty_inputs_yield_empty() {
        assert!(select(vec![], &[]).is_empty());
    }

    #[test]
    fn advisory_template_labels_and_invites_contest_without_instructing() {
        let surfaced = vec![SurfacedMemory {
            id: "mem-1".into(),
            kind: Kind::Lesson,
            trust: Trust::FirstHand,
            score: 0.9,
            content: "the staging deploy fails unless the cache is cleared first".into(),
        }];
        let block = advisory_context(&surfaced);
        assert!(block.contains("advisory context, not"), "{block}");
        assert!(block.contains("forget(<id>)"), "{block}");
        assert!(
            block.contains("[lesson, first_hand, memory mem-1]"),
            "{block}"
        );
        assert!(
            block.contains("\"the staging deploy fails unless the cache is cleared first\""),
            "{block}"
        );
        // Advisory, never imperative about applying the memory.
        for banned in ["you must", "apply this", "use this memory", "follow"] {
            assert!(!block.to_lowercase().contains(banned), "{block}");
        }
    }

    // ---- T003/T007/T009: run() orchestration ------------------------------

    struct DepsBuilder {
        embedder: MockEmbedder,
        storage: MockStorage,
    }

    impl DepsBuilder {
        fn new() -> Self {
            Self {
                embedder: MockEmbedder::new(),
                storage: MockStorage::new(),
            }
        }

        fn build(self) -> MemoryDeps {
            let mut clock = MockTimeProvider::new();
            clock.expect_now().return_const(fixed_now());
            let mut registry = ModeRegistry::new();
            crate::modes::verify::register(&mut registry, 1).unwrap();
            MemoryDeps {
                embedder: Arc::new(self.embedder),
                storage: Arc::new(self.storage),
                clock: Arc::new(clock),
                model_client: Arc::new(MockModelClient::new()),
                verify_mode: registry
                    .get(crate::modes::verify::VERIFY_ID)
                    .unwrap()
                    .clone(),
                input_max_chars: 50_000,
                default_recall_limit: 5,
            }
        }
    }

    fn surface_params(prompt: &str) -> SurfaceParams {
        SurfaceParams {
            session_id: "ps-1".into(),
            prompt: prompt.into(),
        }
    }

    #[tokio::test]
    async fn a_related_prompt_surfaces_with_one_record_and_attributed_tokens() {
        let mut b = DepsBuilder::new();
        b.embedder.expect_embed_query().returning(|_| {
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 7,
            })
        });
        b.storage
            .expect_load_memories()
            .returning(|| Ok(vec![memory("mem-1", Trust::FirstHand, vec![1.0, 0.0])]));
        b.storage
            .expect_pushed_memory_ids()
            .returning(|_| Ok(vec![]));
        b.storage
            .expect_record_push()
            .times(1)
            .withf(|r| {
                r.session_id == "ps-1"
                    && r.surfaced_ids == vec!["mem-1".to_string()]
                    && !r.fail_open
                    && r.input_tokens == 7
            })
            .returning(|_| Ok(()));

        let (result, inp, out) = run(&b.build(), &surface_params("a clearly related prompt"))
            .await
            .unwrap();
        assert_eq!(result.surfaced.len(), 1);
        assert_eq!(result.surfaced[0].id, "mem-1");
        assert!(!result.fail_open);
        assert_eq!((inp, out), (7, 0));
        let hook = result.hook_specific_output.unwrap();
        assert_eq!(hook.hook_event_name, "UserPromptSubmit");
        assert!(hook.additional_context.contains("mem-1"));
    }

    #[tokio::test]
    async fn nothing_above_the_floor_is_a_recorded_silence_without_hook_output() {
        let mut b = DepsBuilder::new();
        b.embedder.expect_embed_query().returning(|_| {
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 5,
            })
        });
        b.storage
            .expect_load_memories()
            .returning(|| Ok(vec![memory("far", Trust::FirstHand, vec![0.1, 0.995])]));
        b.storage
            .expect_pushed_memory_ids()
            .returning(|_| Ok(vec![]));
        b.storage
            .expect_record_push()
            .times(1)
            .withf(|r| r.surfaced_ids.is_empty() && !r.fail_open)
            .returning(|_| Ok(()));

        let (result, _, _) = run(&b.build(), &surface_params("an unrelated prompt entirely"))
            .await
            .unwrap();
        assert!(result.surfaced.is_empty());
        assert!(result.hook_specific_output.is_none());
        // SC-002: silence injects nothing — the serialized result has no
        // hookSpecificOutput key at all.
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("hookSpecificOutput").is_none());
    }

    #[tokio::test]
    async fn an_embedder_failure_is_a_recorded_fail_open_silence() {
        let mut b = DepsBuilder::new();
        b.embedder
            .expect_embed_query()
            .returning(|_| Err(AppError::ValidationFailure("embedding backend down".into())));
        b.storage
            .expect_record_push()
            .times(1)
            .withf(|r| r.fail_open && r.surfaced_ids.is_empty() && r.input_tokens == 0)
            .returning(|_| Ok(()));

        let (result, _, _) = run(&b.build(), &surface_params("any prompt at all here"))
            .await
            .unwrap();
        assert!(result.fail_open);
        assert!(result.surfaced.is_empty());
        assert!(result.hook_specific_output.is_none());
    }

    /// An embedder that outlives the push budget — the timeout test double.
    struct SlowEmbedder;

    #[async_trait::async_trait]
    impl crate::traits::embedder::Embedder for SlowEmbedder {
        async fn embed_document(&self, _text: &str) -> Result<Embedding, AppError> {
            unreachable!("push never embeds documents")
        }

        async fn embed_query(&self, _text: &str) -> Result<Embedding, AppError> {
            tokio::time::sleep(std::time::Duration::from_millis(PUSH_BUDGET_MS + 200)).await;
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 5,
            })
        }

        fn model_id(&self) -> &'static str {
            "voyage-4"
        }
    }

    #[tokio::test]
    async fn a_budget_overrun_fails_open_within_the_budget() {
        let mut b = DepsBuilder::new();
        b.storage
            .expect_record_push()
            .times(1)
            .withf(|r| r.fail_open)
            .returning(|_| Ok(()));
        let mut deps = b.build();
        deps.embedder = Arc::new(SlowEmbedder);

        let (result, _, _) = run(&deps, &surface_params("a prompt that will time out"))
            .await
            .unwrap();
        assert!(result.fail_open);
        assert!(result.surfaced.is_empty());
    }

    #[tokio::test]
    async fn an_empty_prompt_is_a_recorded_silence_without_embedding() {
        let mut b = DepsBuilder::new();
        b.embedder.expect_embed_query().times(0);
        b.storage
            .expect_record_push()
            .times(1)
            .withf(|r| r.surfaced_ids.is_empty() && !r.fail_open)
            .returning(|_| Ok(()));

        let (result, _, _) = run(&b.build(), &surface_params("   ")).await.unwrap();
        assert!(result.surfaced.is_empty());
        assert!(!result.fail_open);
    }

    #[tokio::test]
    async fn a_suppressed_memory_stays_suppressed_for_the_session() {
        let mut b = DepsBuilder::new();
        b.embedder.expect_embed_query().returning(|_| {
            Ok(Embedding {
                vector: vec![1.0, 0.0],
                input_tokens: 5,
            })
        });
        b.storage
            .expect_load_memories()
            .returning(|| Ok(vec![memory("mem-1", Trust::FirstHand, vec![1.0, 0.0])]));
        // The audit trail already carries mem-1 for this session (FR-005).
        b.storage
            .expect_pushed_memory_ids()
            .withf(|s| s == "ps-1")
            .returning(|_| Ok(vec!["mem-1".to_string()]));
        b.storage
            .expect_record_push()
            .times(1)
            .withf(|r| r.surfaced_ids.is_empty())
            .returning(|_| Ok(()));

        let (result, _, _) = run(&b.build(), &surface_params("the same related prompt again"))
            .await
            .unwrap();
        assert!(result.surfaced.is_empty());
        assert!(result.hook_specific_output.is_none());
    }
}
