//! The research tool's wire types (contract:
//! `specs/004-research-layer/contracts/research.tool.json`).
//!
//! MCP-side only — there is no model hop for this shape, so nesting is legal
//! (003 D6 precedent). `key_findings`/`disagreements`/`sources`/`stats` are
//! server-assembled; the model writes the answer prose, the gaps, and the
//! sub-question each gap concerns (021 `gap_targets`). Every published number
//! is computed by the server from that input — the model never emits a
//! confidence, a coverage figure, or a rate directly.

use crate::research::{Depth, Support};
use serde::{Deserialize, Serialize};

/// `research` input.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct ResearchParams {
    /// The research question, in natural language.
    pub question: String,
    /// Rigor tier; scales angles, sources, and verification votes. Default
    /// standard.
    pub depth: Option<Depth>,
    /// Optional angles to bias the search toward.
    pub focus: Option<Vec<String>>,
    /// Optional hard constraints; explicit values override tier defaults.
    pub constraints: Option<Constraints>,
}

/// Caller constraints (all optional; FR-006).
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
pub struct Constraints {
    /// Hard cap on fetched sources; overrides the tier default.
    pub max_sources: Option<u32>,
    /// Restrict fetching to these registrable domains.
    pub domains_allow: Option<Vec<String>>,
    /// Never fetch these domains. Absolute.
    pub domains_deny: Option<Vec<String>>,
    /// Hard token ceiling; hitting it synthesizes early with stopped_early
    /// set.
    pub budget_tokens: Option<u64>,
    /// Hard wall-clock ceiling; hitting it synthesizes early with
    /// stopped_early set.
    pub deadline_ms: Option<u64>,
}

/// `research` output.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ResearchResult {
    /// Executive synthesis with inline `[sN]` citations.
    pub answer: String,
    /// Support established for the claims the answer asserts (0..=1) — the
    /// mean confidence of the published findings (021 FR-001).
    ///
    /// **Not** reduced by unsettled sub-questions; breadth of resolution is
    /// reported separately. Zero only when no claim was supported.
    pub confidence: f32,
    /// Proportion of verified claims that verification refuted (021 FR-009a).
    ///
    /// Beside `confidence` rather than inside it: the answer does not assert
    /// refuted claims, but without this a run whose evidence largely fell
    /// apart would be indistinguishable from one whose evidence held.
    pub refutation_rate: f32,
    /// Proportion of the run's scoped sub-questions that were settled (021
    /// FR-002) — breadth of resolution, which `confidence` used to be
    /// multiplied by and no longer is.
    ///
    /// Equals the fraction of [`Self::sub_question_status`] marked settled —
    /// or `1.0` when the run scoped no sub-questions, in which case that list
    /// is empty and nothing is unsettled. Stated explicitly because the
    /// fraction is undefined there, and a caller applying the equality to an
    /// empty list would compute 0/0 rather than the 1.0 the server reports.
    pub coverage: f32,
    /// Each sub-question the run scoped, and whether it was settled (021
    /// FR-005). The published basis for [`Self::coverage`].
    pub sub_question_status: Vec<SubQuestionStatus>,
    /// Server-assembled findings, each citing at least one source.
    pub key_findings: Vec<KeyFinding>,
    /// Contested claims with their conflicting positions — surfaced, not
    /// resolved.
    pub disagreements: Vec<Disagreement>,
    /// What could not be answered, plus anything demoted by the grounding
    /// gate.
    pub gaps: Vec<String>,
    /// Identity of every cited source — never page bodies.
    pub sources: Vec<SourceRef>,
    /// Honest accounting (FR-007).
    pub stats: Stats,
}

/// One scoped sub-question and whether the run settled it (021 FR-005).
///
/// Published so `coverage` can be reconciled against the output rather than
/// taken on trust — the sub-questions are not otherwise visible to the caller,
/// so a bare index would reference nothing.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SubQuestionStatus {
    /// The sub-question, verbatim as the scope phase produced it.
    pub sub_question: String,
    /// True when no retained gap targets this sub-question.
    pub settled: bool,
}

/// One verified finding.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct KeyFinding {
    /// The claim.
    pub claim: String,
    /// Support standing (confirmed | contested | refuted | unverified).
    pub support: Support,
    /// Post-verification confidence (0..=1).
    pub confidence: f32,
    /// Citation ids — every id resolves in `sources` (FR-003).
    pub sources: Vec<String>,
}

/// One contested claim with its positions.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Disagreement {
    /// The contested claim.
    pub claim: String,
    /// The conflicting positions with their sources.
    pub positions: Vec<Position>,
}

/// One stance within a disagreement.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct Position {
    /// The stance.
    pub stance: String,
    /// Sources backing it.
    pub sources: Vec<String>,
}

/// Identity of one fetched source.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SourceRef {
    /// Run-scoped id (`s1`, `s2`, …).
    pub id: String,
    /// Final URL after redirects.
    pub url: String,
    /// Page title (extraction metadata; may be the URL when absent).
    pub title: String,
    /// RFC 3339 fetch time.
    pub fetched_at: String,
    /// Heuristic credibility (0..=1) — conservative and explainable.
    pub credibility: f32,
}

/// Honest run accounting (FR-007: no silent truncation).
#[derive(Debug, Clone, Default, Serialize, schemars::JsonSchema)]
pub struct Stats {
    /// Search angles scoped.
    pub angles: u32,
    /// Searches executed (≤ angles when some fail).
    pub searches: u32,
    /// Candidate sources found across angles (post URL dedup).
    pub sources_found: u32,
    /// Sources successfully fetched and extracted.
    pub sources_fetched: u32,
    /// Claims extracted across sources.
    pub claims_extracted: u32,
    /// Claims after semantic dedup.
    pub claims_after_dedup: u32,
    /// Claims that completed verification.
    pub claims_verified: u32,
    /// Claims dropped (refuted, failed verification calls, or unprocessed at
    /// an early stop).
    pub claims_dropped: u32,
    /// Summed LLM token usage (input + output) across every call in the run.
    pub tokens: u64,
    /// Wall-clock elapsed.
    pub elapsed_ms: u64,
    /// True when a ceiling stopped the run before completion.
    pub stopped_early: bool,
    /// Why it stopped early (`budget` | `deadline` | `grounding`), or null.
    pub stop_reason: Option<StopReason>,
}

/// Why a run synthesized early.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StopReason {
    /// The token budget ceiling was hit.
    Budget,
    /// The wall-clock deadline was hit.
    Deadline,
    /// The grounding gate demoted content after its retry.
    Grounding,
    /// The synthesis returned a malformed response twice — its gap list and
    /// the sub-question keys for it disagreed in length — and was demoted
    /// (021).
    ///
    /// Distinct from [`Self::Grounding`] because the grounding gate is never
    /// reached on that path: reporting it as a grounding failure would tell
    /// the caller the answer could not be cited when citation was never
    /// evaluated, and 004 FR-007 requires the accounting be honest.
    MalformedSynthesis,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// The derived schemas and the checked-in contract file share exactly the
    /// same property sets, both directions (003 pattern).
    #[test]
    fn derived_schemas_match_the_contract_file() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../specs/004-research-layer/contracts/research.tool.json"
        ))
        .unwrap();
        let props = |schema: &Value| -> Vec<String> {
            schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect()
        };

        let input = serde_json::to_value(schemars::schema_for!(ResearchParams)).unwrap();
        assert_eq!(props(&input), props(&contract["inputSchema"]));
        // Constraints sub-object property set.
        let derived_constraints = serde_json::to_value(schemars::schema_for!(Constraints)).unwrap();
        assert_eq!(
            props(&derived_constraints),
            props(&contract["inputSchema"]["properties"]["constraints"])
        );

        let output = serde_json::to_value(schemars::schema_for!(ResearchResult)).unwrap();
        assert_eq!(props(&output), props(&contract["outputSchema"]));
        // Stats property set — the honest-accounting contract.
        let derived_stats = serde_json::to_value(schemars::schema_for!(Stats)).unwrap();
        assert_eq!(
            props(&derived_stats),
            props(&contract["outputSchema"]["properties"]["stats"])
        );
    }

    #[test]
    fn stop_reason_and_depth_serialize_lowercase() {
        assert_eq!(
            serde_json::to_value(StopReason::Deadline).unwrap(),
            Value::String("deadline".into())
        );
        let depth: Depth = serde_json::from_value(Value::String("quick".into())).unwrap();
        assert_eq!(depth, Depth::Quick);
        assert_eq!(
            serde_json::to_value(Support::Unverified).unwrap(),
            Value::String("unverified".into())
        );
    }
}
