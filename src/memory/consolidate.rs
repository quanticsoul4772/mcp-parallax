//! Admission-time consolidation (017): supersession and merge.
//!
//! A deterministic cosine screen over ACTIVE same-kind memories gates at
//! most ONE budgeted, decline-biased model judgment per admission (the
//! checkpoint layer's screen-gates-judge pattern, research D3). All apply
//! rules are pure: `updates` supersedes, `same_assertion` merges when the
//! merge band and the trust guard allow, everything else — including any
//! judgment failure — keeps both (FR-002). Stored content is never
//! modified; only status columns change (FR-010).

use crate::error::AppError;
use crate::memory::{Memory, Status};
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The consolidation judgment's registry id.
pub const CONSOLIDATION_MODE_ID: &str = "memory_consolidation";

/// Screen floor: pairs below this raw cosine are never judged (research D3).
/// Far above the T015 related-content datum (0.406); moves only with
/// audit-row measurement.
pub const SUPERSEDE_SCREEN_TAU: f32 = 0.75;

/// Merge band: `same_assertion` merges only at or above this raw cosine —
/// near-duplicate territory (research D3/D9).
pub const MERGE_SCREEN_TAU: f32 = 0.90;

/// Budget for the single judgment; overrun ⇒ keep both (fail-open). The
/// save path already tolerates verify-at-save latency (research D9).
pub const CONSOLIDATION_BUDGET_MS: u64 = 5_000;

/// Max capture proposals stored per session (research D6/D9) — the
/// candidate-flood bound; silence remains the default.
pub const CAPTURE_SESSION_CAP: u32 = 2;

/// How the judgment relates the NEW admission to the OLD active memory
/// (contracts/consolidation.hop.json). Uncertain ⇒ `Distinct`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum Relation {
    /// Same knowledge in different words.
    SameAssertion,
    /// The new memory replaces the old as current truth.
    Updates,
    /// Situational; does not displace the standing memory (Berlin/Lisbon).
    ContextSpecific,
    /// Different knowledge (also the uncertain default).
    Distinct,
}

/// The judgment's constrained output (flat + closed — Principle II).
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ConsolidationOut {
    /// The relation of the NEW memory to the OLD.
    pub relation: Relation,
    /// One sentence of grounds.
    pub basis: String,
}

/// The consolidation audit actions (data-model §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationAction {
    /// An older memory was superseded by a newer admission.
    Supersede,
    /// An older memory was merged into a newer canonical admission.
    Merge,
    /// A capture candidate was proposed and stored (quarantined).
    CaptureProposed,
    /// A capture proposal was dropped by the per-session cap.
    CaptureDropped,
}

impl ConsolidationAction {
    /// Stable string form (the `action` column).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supersede => "supersede",
            Self::Merge => "merge",
            Self::CaptureProposed => "capture_proposed",
            Self::CaptureDropped => "capture_dropped",
        }
    }

    /// Parse the stable string form (storage read path).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "supersede" => Some(Self::Supersede),
            "merge" => Some(Self::Merge),
            "capture_proposed" => Some(Self::CaptureProposed),
            "capture_dropped" => Some(Self::CaptureDropped),
            _ => None,
        }
    }
}

/// One consolidation audit row (FR-009; data-model §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidationRecord {
    /// UUID v4.
    pub id: String,
    /// The invoking session, when known (capture rows always carry it).
    pub session_id: Option<String>,
    /// What happened.
    pub action: ConsolidationAction,
    /// The acted-on / proposed memory.
    pub source_id: String,
    /// The superseding / canonical id, when applicable.
    pub target_id: Option<String>,
    /// The judgment's one-sentence grounds.
    pub basis: String,
    /// Via `TimeProvider`.
    pub created_at: DateTime<Utc>,
}

const CONSOLIDATION_PROMPT_TEMPLATE: &str = "\
You are an external memory curator judging how a NEWLY stored memory \
relates to one EXISTING stored memory. You see only the two contents — no \
author, no stakes. Classify the relation:\n\
- same_assertion: the two state the SAME knowledge in different words.\n\
- updates: the NEW memory replaces the OLD as current truth (the old \
statement was true, the world changed).\n\
- context_specific: the NEW statement is situational or temporary and does \
NOT displace the OLD standing statement (e.g. a this-week circumstance \
beside a standing fact).\n\
- distinct: different knowledge.\n\
HARD RULE: when uncertain between any two classifications, answer \
distinct — wrongly replacing or merging destroys knowledge; keeping both \
costs only a duplicate.\n\
Return the relation and a one-sentence basis.\n\
\n\
EXISTING memory:\n<<existing>>\n\
\n\
NEW memory:\n<<new>>";

/// Register the consolidation judgment mode (boot-time; flat+closed).
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(ConsolidationOut))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        CONSOLIDATION_MODE_ID,
        "internal: admission-time memory consolidation",
        CONSOLIDATION_PROMPT_TEMPLATE,
        schema,
        1,
    )
}

/// Pure screen (research D3): the best ACTIVE same-kind pair by raw cosine,
/// if any reaches [`SUPERSEDE_SCREEN_TAU`]. The new memory itself is
/// excluded by id.
#[must_use]
pub fn screen<'a>(new: &Memory, memories: &'a [Memory]) -> Option<(f32, &'a Memory)> {
    let mut best: Option<(f32, &Memory)> = None;
    for memory in memories {
        if memory.id == new.id || memory.kind != new.kind || !memory.status.is_active() {
            continue;
        }
        let score = crate::memory::ranking::cosine(&new.embedding, &memory.embedding);
        if score >= SUPERSEDE_SCREEN_TAU && best.is_none_or(|(top, _)| score > top) {
            best = Some((score, memory));
        }
    }
    best
}

/// A pure apply decision: which status change (if any) the judgment implies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The audit action.
    pub action: ConsolidationAction,
    /// The old memory's new status.
    pub old_status: Status,
}

/// Pure apply rules (contracts/consolidation.hop.json).
///
/// `updates` supersedes; `same_assertion` merges inside the merge band
/// unless the trust guard blocks it (an untrusted admission never merges
/// away a trusted record — research D4); everything else keeps both.
#[must_use]
pub fn apply(relation: Relation, cosine: f32, new: &Memory, old: &Memory) -> Option<Applied> {
    match relation {
        Relation::Updates => Some(Applied {
            action: ConsolidationAction::Supersede,
            old_status: Status::Superseded,
        }),
        Relation::SameAssertion
            if cosine >= MERGE_SCREEN_TAU
                && (new.trust.is_trusted() || !old.trust.is_trusted()) =>
        {
            Some(Applied {
                action: ConsolidationAction::Merge,
                old_status: Status::Merged,
            })
        }
        _ => None,
    }
}

/// Run the single budgeted judgment for a screened pair.
///
/// # Errors
///
/// Provider classes from the model call; schema violations are
/// `ValidationFailure`; budget overrun is `Timeout`. Callers treat every
/// error as keep-both (fail-open).
pub async fn judge(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    new: &Memory,
    old: &Memory,
) -> Result<(ConsolidationOut, u64, u64), AppError> {
    // One-pass substitution on the pristine template (the 005 rule): both
    // placeholders located before any caller text is inserted.
    let Some((head, rest)) = mode.prompt_template.split_once("<<existing>>") else {
        return Err(AppError::ValidationFailure(
            "consolidation template lost its existing placeholder".to_string(),
        ));
    };
    let Some((mid, tail)) = rest.split_once("<<new>>") else {
        return Err(AppError::ValidationFailure(
            "consolidation template lost its new placeholder".to_string(),
        ));
    };
    let prompt = format!("{head}{}{mid}{}{tail}", old.content, new.content);

    let work = async {
        let completion = client.complete(&prompt, &mode.sanitized_schema).await?;
        validate(&mode.output_schema, &completion.value)?;
        let out: ConsolidationOut = serde_json::from_value(completion.value)
            .map_err(|e| AppError::ValidationFailure(format!("consolidation shape: {e}")))?;
        Ok::<_, AppError>((out, completion.input_tokens, completion.output_tokens))
    };
    tokio::time::timeout(
        std::time::Duration::from_millis(CONSOLIDATION_BUDGET_MS),
        work,
    )
    .await
    .map_err(|_| AppError::Timeout {
        what: "memory consolidation judgment",
        ms: CONSOLIDATION_BUDGET_MS,
    })?
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::memory::{Kind, Trust};
    use crate::traits::client::{Completion, MockModelClient};
    use serde_json::json;

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-23T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn memory(id: &str, kind: Kind, trust: Trust, embedding: Vec<f32>, status: Status) -> Memory {
        Memory {
            id: id.into(),
            content: format!("content of {id}"),
            kind,
            origin: "test".into(),
            external: trust == Trust::Untrusted,
            trust,
            tags: vec![],
            embedding,
            embedding_model: "voyage-4".into(),
            created_at: fixed_now(),
            status,
            replaced_by: None,
            last_reinforced_at: fixed_now(),
        }
    }

    // ---- screen -----------------------------------------------------------

    #[test]
    fn screen_selects_best_active_same_kind_pair_at_the_floor() {
        let new = memory(
            "new",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let store = vec![
            memory(
                "close",
                Kind::Fact,
                Trust::FirstHand,
                vec![0.9, 0.436],
                Status::Active,
            ),
            memory(
                "closer",
                Kind::Fact,
                Trust::FirstHand,
                vec![0.99, 0.141],
                Status::Active,
            ),
            memory(
                "wrong-kind",
                Kind::Lesson,
                Trust::FirstHand,
                vec![1.0, 0.0],
                Status::Active,
            ),
            memory(
                "inactive",
                Kind::Fact,
                Trust::FirstHand,
                vec![1.0, 0.0],
                Status::Superseded,
            ),
            memory(
                "far",
                Kind::Fact,
                Trust::FirstHand,
                vec![0.1, 0.995],
                Status::Active,
            ),
        ];
        let (score, best) = screen(&new, &store).unwrap();
        assert_eq!(best.id, "closer");
        assert!(score > 0.98);
    }

    #[test]
    fn screen_is_silent_below_the_floor_and_on_empty() {
        let new = memory(
            "new",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let store = vec![memory(
            "meh",
            Kind::Fact,
            Trust::FirstHand,
            vec![0.7, 0.714],
            Status::Active,
        )];
        assert!(screen(&new, &store).is_none()); // cosine 0.70 < 0.75
        assert!(screen(&new, &[]).is_none());
    }

    // ---- apply ------------------------------------------------------------

    #[test]
    fn updates_supersedes_and_context_or_distinct_keep_both() {
        let new = memory(
            "new",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let old = memory(
            "old",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let applied = apply(Relation::Updates, 0.8, &new, &old).unwrap();
        assert_eq!(applied.action, ConsolidationAction::Supersede);
        assert_eq!(applied.old_status, Status::Superseded);
        assert!(apply(Relation::ContextSpecific, 0.99, &new, &old).is_none());
        assert!(apply(Relation::Distinct, 0.99, &new, &old).is_none());
    }

    #[test]
    fn same_assertion_merges_only_inside_the_merge_band() {
        let new = memory(
            "new",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let old = memory(
            "old",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let applied = apply(Relation::SameAssertion, 0.92, &new, &old).unwrap();
        assert_eq!(applied.action, ConsolidationAction::Merge);
        assert_eq!(applied.old_status, Status::Merged);
        // 0.75–0.90 band: screened, judged same, but NOT merged.
        assert!(apply(Relation::SameAssertion, 0.85, &new, &old).is_none());
    }

    #[test]
    fn trust_guard_blocks_untrusted_merging_away_trusted_but_allows_promotion() {
        let untrusted_new = memory(
            "cand",
            Kind::Fact,
            Trust::Untrusted,
            vec![1.0, 0.0],
            Status::Active,
        );
        let trusted_old = memory(
            "real",
            Kind::Fact,
            Trust::Verified,
            vec![1.0, 0.0],
            Status::Active,
        );
        // Blocking direction: untrusted admission never merges away trusted.
        assert!(apply(Relation::SameAssertion, 0.95, &untrusted_new, &trusted_old).is_none());

        // Promotion direction (research D7 / FR-007): a trusted first-hand
        // admission merges away an untrusted candidate — trusted canonical.
        let trusted_new = memory(
            "save",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let candidate_old = memory(
            "cand",
            Kind::Fact,
            Trust::Untrusted,
            vec![1.0, 0.0],
            Status::Active,
        );
        let applied = apply(Relation::SameAssertion, 0.95, &trusted_new, &candidate_old).unwrap();
        assert_eq!(applied.action, ConsolidationAction::Merge);
    }

    // ---- mode + judge -----------------------------------------------------

    #[test]
    fn the_mode_schema_is_flat_and_registers_with_decline_bias_pinned() {
        let mut registry = ModeRegistry::new();
        register(&mut registry).unwrap();
        let mode = registry.get(CONSOLIDATION_MODE_ID).unwrap();
        assert_eq!(mode.ensemble_k, 1);
        assert!(mode.prompt_template.contains("<<existing>>"));
        assert!(mode.prompt_template.contains("<<new>>"));
        assert!(
            mode.prompt_template.contains("answer \ndistinct")
                || mode.prompt_template.contains("answer distinct")
        );
        let properties = mode.output_schema["properties"].as_object().unwrap();
        assert!(properties.contains_key("relation"));
        assert!(properties.contains_key("basis"));
    }

    fn test_mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        register(&mut registry).unwrap();
        registry.get(CONSOLIDATION_MODE_ID).unwrap().clone()
    }

    #[tokio::test]
    async fn judge_returns_the_classified_relation() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|prompt, _| {
            assert!(prompt.contains("content of old"), "{prompt}");
            assert!(prompt.contains("content of new"), "{prompt}");
            Ok(Completion {
                value: json!({ "relation": "updates", "basis": "The new memory reflects the move." }),
                input_tokens: 40,
                output_tokens: 12,
            })
        });
        let new = memory(
            "new",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let old = memory(
            "old",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let (out, inp, outp) = judge(&client, &test_mode(), &new, &old).await.unwrap();
        assert_eq!(out.relation, Relation::Updates);
        assert_eq!((inp, outp), (40, 12));
    }

    #[tokio::test]
    async fn judge_surfaces_schema_violations_loudly() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|_, _| {
            Ok(Completion {
                value: json!({ "relation": "sideways", "basis": "?" }),
                input_tokens: 10,
                output_tokens: 5,
            })
        });
        let new = memory(
            "new",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let old = memory(
            "old",
            Kind::Fact,
            Trust::FirstHand,
            vec![1.0, 0.0],
            Status::Active,
        );
        let err = judge(&client, &test_mode(), &new, &old).await.unwrap_err();
        assert!(
            err.to_string().contains("relation") || err.to_string().contains("enum"),
            "{err}"
        );
    }
}
