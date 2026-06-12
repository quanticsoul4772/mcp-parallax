//! Phase 5: server-assembled findings and the grounded synthesis call
//! (research.md 004 D7; FR-003).
//!
//! The model writes prose and gap phrasing only. Findings, support labels,
//! confidences, disagreements, and sources are deterministic functions of
//! pipeline state assembled here; the grounding gate validates the prose's
//! citation tokens with one violation-fed retry, then demotes — nothing
//! ungrounded ever leaves the server.
//!
//! v1 narrowing (named, research.md D7): disagreement positions reflect the
//! *verification-panel* split, not per-source stances — the per-source stance
//! breakdown is not tracked because claims merge across sources at dedup.

use crate::error::AppError;
use crate::modes::CorrectiveMode;
use crate::research::contract::{Disagreement, KeyFinding, Position, ResearchParams, StopReason};
use crate::research::grounding;
use crate::research::prompts::SynthOut;
use crate::research::{RunMeter, ScopePlan, SourceRecord, Support, VerifiedClaim};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use std::collections::{BTreeMap, BTreeSet};

/// Server-assembled findings and disagreements (D7 — never model-written).
pub struct Assembled {
    pub findings: Vec<KeyFinding>,
    pub disagreements: Vec<Disagreement>,
}

pub fn assemble(surviving: &[VerifiedClaim]) -> Assembled {
    let mut findings = Vec::new();
    let mut disagreements = Vec::new();
    for v in surviving {
        findings.push(KeyFinding {
            claim: v.claim.text.clone(),
            support: v.support,
            confidence: v.confidence,
            sources: v.claim.source_ids.clone(),
        });
        if v.support == Support::Contested {
            // v1 positions reflect the verification split (the per-source
            // stance breakdown is not tracked — claims merge across sources).
            disagreements.push(Disagreement {
                claim: v.claim.text.clone(),
                positions: vec![
                    Position {
                        stance: "supported by part of the verification panel".to_string(),
                        sources: v.claim.source_ids.clone(),
                    },
                    Position {
                        stance: if v.findings.is_empty() {
                            "challenged by part of the verification panel".to_string()
                        } else {
                            format!("challenged: {}", v.findings.join(" | "))
                        },
                        sources: v.claim.source_ids.clone(),
                    },
                ],
            });
        }
    }
    Assembled {
        findings,
        disagreements,
    }
}

/// Synthesis with the grounding gate: one attempt, one violation-fed retry,
/// then demotion (FR-003; never an ungrounded claim). A demotion records
/// `StopReason::Grounding` only when no earlier ceiling reason exists.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // the gate needs exactly this run state
pub async fn synthesize_grounded(
    model_client: &dyn ModelClient,
    synth_mode: &CorrectiveMode,
    params: &ResearchParams,
    plan: &ScopePlan,
    surviving: &[VerifiedClaim],
    refuted: &[VerifiedClaim],
    source_meta: &BTreeMap<String, &SourceRecord>,
    fetched_ids: &BTreeSet<String>,
    meter: &RunMeter,
    stop_reason: &mut Option<StopReason>,
) -> Result<(String, Vec<String>, Vec<String>), AppError> {
    let findings_block = surviving
        .iter()
        .map(|v| {
            let tokens = v
                .claim
                .source_ids
                .iter()
                .fold(String::new(), |mut acc, id| {
                    acc.push('[');
                    acc.push_str(id);
                    acc.push(']');
                    acc
                });
            let titles = v
                .claim
                .source_ids
                .iter()
                .filter_map(|id| source_meta.get(id))
                .map(|s| s.title.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "- ({}, confidence {:.2}) {} {tokens} — {titles}",
                match v.support {
                    Support::Confirmed => "confirmed",
                    Support::Contested => "contested",
                    Support::Unverified => "unverified, single-source",
                    Support::Refuted => "refuted",
                },
                v.confidence,
                v.claim.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let refuted_block = if refuted.is_empty() {
        "(none)".to_string()
    } else {
        refuted
            .iter()
            .map(|v| format!("- {}", v.claim.text))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let sub_questions_block = if plan.sub_questions.is_empty() {
        "(none scoped)".to_string()
    } else {
        plan.sub_questions
            .iter()
            .map(|q| format!("- {q}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let finding_sources: Vec<Vec<String>> = surviving
        .iter()
        .map(|v| v.claim.source_ids.clone())
        .collect();

    let mut retry_clause = String::new();
    for _attempt in 0..2 {
        let prompt = synth_mode
            .prompt_template
            .replace("<<retry_clause>>", &retry_clause)
            .replace("<<question>>", params.question.trim())
            .replace("<<sub_questions>>", &sub_questions_block)
            .replace("<<findings>>", &findings_block)
            .replace("<<refuted>>", &refuted_block);
        let completion = model_client
            .complete(&prompt, &synth_mode.sanitized_schema)
            .await?;
        meter.add(completion.input_tokens, completion.output_tokens);
        validate(&synth_mode.output_schema, &completion.value)?;
        let out: SynthOut = serde_json::from_value(completion.value)
            .map_err(|e| AppError::ValidationFailure(format!("synthesis shape: {e}")))?;

        match grounding::ground(&out.answer, &finding_sources, fetched_ids) {
            Ok(grounded) => {
                return Ok((out.answer, out.gaps, grounded.kept_source_ids));
            }
            Err(violations) => {
                tracing::warn!(?violations, "grounding gate rejected the synthesis");
                retry_clause = format!(
                    " YOUR PREVIOUS ATTEMPT WAS REJECTED for citation violations: {}. Cite \
                     only the listed source tokens.",
                    violations.join("; ")
                );
            }
        }
    }

    // Second failure → demotion: nothing ungrounded leaves the server. An
    // earlier budget/deadline reason is preserved over the grounding one.
    stop_reason.get_or_insert(StopReason::Grounding);
    let mut gaps = vec![
        "the synthesis could not be grounded in the fetched sources and was demoted".to_string(),
    ];
    gaps.extend(plan.sub_questions.clone());
    let grounded = grounding::ground("", &finding_sources, fetched_ids)
        .map_or_else(|_| Vec::new(), |g| g.kept_source_ids);
    Ok((
        "The synthesis could not be grounded after a retry; see key_findings for the \
         verified claims and gaps for what remains."
            .to_string(),
        gaps,
        grounded,
    ))
}
