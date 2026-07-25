//! Result assembly: the published figures and the wire shape, derived from
//! finished run state.
//!
//! Extracted from [`crate::research::pipeline`] (code review, 021): the block
//! is a pure function of state the five phases have already produced, so it
//! belongs beside the arithmetic it calls rather than inside the phase spine.
//! The extraction also gives the coverage/`sub_question_status` agreement a
//! unit-testable home — the published contract promises `coverage` equals the
//! settled share of that list, and before this the promise held only by both
//! call sites reading the same helper.

use crate::research::contract::{ResearchResult, SourceRef, Stats, SubQuestionStatus};
use crate::research::synthesis::{Assembled, Synthesized};
use crate::research::{verdict, MAX_GAPS};
use crate::research::{ScopePlan, SourceRecord, VerifiedClaim};
use std::collections::BTreeMap;

/// Everything result assembly needs that is not already a published field.
pub struct Outcome<'a> {
    /// The scope phase's plan — sub-questions are the unit coverage measures.
    pub plan: &'a ScopePlan,
    /// Server-assembled findings and disagreements.
    pub assembled: Assembled,
    /// What the synthesis phase produced.
    pub synthesized: Synthesized,
    /// Claims verification refuted — excluded from the answer, counted.
    pub refuted: &'a [VerifiedClaim],
    /// Claims that survived — the findings the answer asserts.
    pub surviving: &'a [VerifiedClaim],
    /// Source lookup for the citation ids the grounding gate kept.
    pub source_meta: &'a BTreeMap<String, &'a SourceRecord>,
}

/// Assemble the published result from finished run state.
///
/// Pure: every figure is counted here from what the phases produced. The model
/// supplies which sub-question each gap concerns and nothing else numeric
/// (021 FR-010).
pub fn assemble_result(outcome: Outcome<'_>, stats: Stats) -> ResearchResult {
    let Outcome {
        plan,
        assembled,
        synthesized,
        refuted,
        surviving,
        source_meta,
    } = outcome;
    let Synthesized {
        answer,
        mut gaps,
        gap_targets,
        grounded_ids,
    } = synthesized;

    // Confidence: the support of what the answer asserts, and nothing else
    // (021 FR-001). The coverage multiplier that used to stand here was the
    // defect — it derived breadth by subtracting the length of the synthesis
    // pass's free-form gap list from the sub-question count, two lists with no
    // correspondence, and drove correct answers to exactly 0.
    let finding_confidences: Vec<f32> = assembled.findings.iter().map(|f| f.confidence).collect();
    let confidence = verdict::overall_confidence(&finding_confidences);

    // Breadth of resolution, published in its own right (021 FR-002), and the
    // per-sub-question basis for it (FR-005) — both from one decode, so the
    // contract's "coverage equals the settled share" holds by construction.
    let settled = verdict::settled_mask(plan.sub_questions.len(), &gap_targets);
    let coverage = verdict::coverage(plan.sub_questions.len(), &gap_targets);
    let sub_question_status: Vec<SubQuestionStatus> = plan
        .sub_questions
        .iter()
        .zip(settled)
        .map(|(sub_question, settled)| SubQuestionStatus {
            sub_question: sub_question.clone(),
            settled,
        })
        .collect();

    // FR-009a: refuted claims are excluded from confidence and surfaced here.
    let refutation_rate = verdict::refutation_rate(refuted.len(), refuted.len() + surviving.len());

    // Sources: only what the grounding kept (uncited pruned).
    let sources: Vec<SourceRef> = grounded_ids
        .iter()
        .filter_map(|id| source_meta.get(id))
        .map(|s| SourceRef {
            id: s.id.clone(),
            url: s.url.clone(),
            title: s.title.clone(),
            fetched_at: s.fetched_at.clone(),
            credibility: s.credibility,
        })
        .collect();

    // 021: this cap acts on the published gap *text* only. Coverage and the
    // statuses were derived above from every target the synthesis returned, so
    // a gap dropped here cannot flip its sub-question to settled — truncating
    // the targets alongside would do exactly that and inflate the figure.
    //
    // The original defect (D3) was that the confidence penalty came from the
    // untruncated list while the caller saw the truncated one, making the
    // number uncheckable. It is fixed by publishing `sub_question_status`, not
    // by reordering statements: the caller reconciles coverage against the
    // statuses, which always agree with it. Gap text is best-effort under the
    // cap.
    gaps.truncate(MAX_GAPS);

    ResearchResult {
        answer,
        confidence,
        refutation_rate,
        coverage,
        sub_question_status,
        key_findings: assembled.findings,
        disagreements: assembled.disagreements,
        gaps,
        sources,
        stats,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::research::{Claim, Support};

    fn claim(text: &str) -> VerifiedClaim {
        VerifiedClaim {
            claim: Claim {
                text: text.to_string(),
                source_ids: vec!["s1".to_string()],
            },
            support: Support::Unverified,
            confidence: 0.8,
            findings: vec![],
        }
    }

    fn plan_of(sub_questions: &[&str]) -> ScopePlan {
        ScopePlan {
            angles: vec![],
            sub_questions: sub_questions.iter().map(|q| (*q).to_string()).collect(),
        }
    }

    fn result_for(sub_questions: &[&str], gaps: &[&str], gap_targets: &[u32]) -> ResearchResult {
        let plan = plan_of(sub_questions);
        let surviving = vec![claim("a claim")];
        let source_meta = BTreeMap::new();
        assemble_result(
            Outcome {
                plan: &plan,
                assembled: crate::research::synthesis::assemble(&surviving),
                synthesized: Synthesized {
                    answer: "answer".to_string(),
                    gaps: gaps.iter().map(|g| (*g).to_string()).collect(),
                    gap_targets: gap_targets.to_vec(),
                    grounded_ids: vec![],
                },
                refuted: &[],
                surviving: &surviving,
                source_meta: &source_meta,
            },
            Stats::default(),
        )
    }

    /// The published contract states `coverage` equals the settled share of
    /// `sub_question_status`. Before the extraction the two were computed by
    /// separate hand-written decodes of the same 1-based keys; the promise now
    /// holds by construction, and this pins it.
    #[test]
    fn coverage_always_equals_the_settled_share_of_the_published_statuses() {
        for (subs, gaps, targets) in [
            (&["a", "b", "c"][..], &[][..], &[][..]),
            (&["a", "b", "c"][..], &["g"][..], &[2][..]),
            (&["a", "b", "c"][..], &["g", "h", "i"][..], &[1, 2, 3][..]),
            // Several gaps on one sub-question: unsettled once (FR-004).
            (&["a", "b"][..], &["g", "h", "i"][..], &[2, 2, 2][..]),
            // Out of range and the no-sub-question sentinel are discarded.
            (&["a", "b"][..], &["g", "h"][..], &[9, 0][..]),
        ] {
            let result = result_for(subs, gaps, targets);
            #[allow(clippy::cast_precision_loss)]
            let share = (result
                .sub_question_status
                .iter()
                .filter(|s| s.settled)
                .count() as f32)
                / (result.sub_question_status.len() as f32);
            assert!(
                (result.coverage - share).abs() < 1e-6,
                "coverage {} vs published share {share} for targets {targets:?}",
                result.coverage
            );
        }
    }

    /// FR-007: a run that scoped nothing has nothing unsettled. The status
    /// list is empty, so the share is undefined — the contract states the 1.0
    /// explicitly for exactly this case.
    #[test]
    fn no_scoped_sub_questions_yields_full_coverage_and_an_empty_status_list() {
        let result = result_for(&[], &["g"], &[0]);
        assert!(result.sub_question_status.is_empty());
        assert!((result.coverage - 1.0).abs() < f32::EPSILON);
    }

    /// The cap acts on published text only: coverage is derived from every
    /// target the synthesis returned, so a dropped gap cannot flip its
    /// sub-question to settled.
    #[test]
    fn the_gap_cap_never_moves_coverage() {
        let subs = ["a", "b"];
        let gaps: Vec<&str> = std::iter::repeat_n("g", MAX_GAPS + 4).collect();
        let mut targets = vec![1_u32; MAX_GAPS + 3];
        targets.push(2);
        let result = result_for(&subs, &gaps, &targets);
        assert_eq!(result.gaps.len(), MAX_GAPS, "published text is capped");
        // Both sub-questions are targeted, including by the entry the cap drops.
        assert!((result.coverage - 0.0).abs() < f32::EPSILON);
        assert!(result.sub_question_status.iter().all(|s| !s.settled));
    }
}
