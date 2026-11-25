//! The versioned Jev questions (064).
//!
//! Wording and option order live here, in one place. A wording change bumps
//! the question's version and needs a new back-test (plan principle 12).

use crate::traits::classifier::{JevQuestion, QuestionKind};

/// The relation options in their fixed order. Jev leans toward the first
/// option, so `supports` (the answer that would let a claim through) is last.
pub const RELATION_OPTIONS: [&str; 3] = ["says_nothing", "contradicts", "supports"];

const RELATION_DESCRIPTIONS: [&str; 3] = [
    "The excerpts do not address the claim.",
    "The excerpts state or directly imply that the claim is false.",
    "The excerpts state the claim or directly imply that it is true.",
];

fn noul(id: String, instructions: String) -> JevQuestion {
    JevQuestion {
        id,
        version: 1,
        kind: QuestionKind::Noul,
        instructions,
        options: vec![],
    }
}

/// The end-of-turn review questions (064 U2).
#[must_use]
pub fn turn_review_questions() -> Vec<JevQuestion> {
    vec![
        noul(
            "contradiction".into(),
            "Does any pair in `statement_pairs` contain two statements that cannot both be \
             true, where the later one does not justify the reversal with new evidence?"
                .into(),
        ),
        noul(
            "capture_worthy".into(),
            "Does the turn contain a reusable skill or lesson worth saving for future sessions?"
                .into(),
        ),
    ]
}

/// The research claim questions (064 U4): a relation and a same-subject
/// question per source, numbered from 1.
#[must_use]
pub fn research_claim_questions(sources: usize) -> Vec<JevQuestion> {
    (1..=sources)
        .flat_map(|n| {
            [
                JevQuestion {
                    id: format!("relation_{n}"),
                    version: 1,
                    kind: QuestionKind::Choice,
                    instructions: format!(
                        "How do the excerpts in `sources[{n}].excerpts` relate to `claim`?"
                    ),
                    options: RELATION_OPTIONS
                        .iter()
                        .zip(RELATION_DESCRIPTIONS)
                        .map(|(o, d)| ((*o).to_string(), Some(d.to_string())))
                        .collect(),
                },
                noul(
                    format!("same_subject_{n}"),
                    format!(
                        "Are `claim` and the excerpts in `sources[{n}].excerpts` about the same \
                         subject (the same entity, product, version or event)?"
                    ),
                ),
            ]
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{research_claim_questions, turn_review_questions, RELATION_OPTIONS};
    use crate::traits::classifier::QuestionKind;
    use std::collections::BTreeSet;

    #[test]
    fn turn_review_asks_contradiction_and_capture_worthy_as_yes_no() {
        let qs = turn_review_questions();
        let ids: Vec<&str> = qs.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids, ["contradiction", "capture_worthy"]);
        assert!(qs.iter().all(|q| q.kind == QuestionKind::Noul));
    }

    #[test]
    fn research_asks_relation_and_same_subject_per_source() {
        let qs = research_claim_questions(2);
        let ids: Vec<&str> = qs.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "relation_1",
                "same_subject_1",
                "relation_2",
                "same_subject_2"
            ]
        );
        for q in &qs {
            if q.id.starts_with("relation_") {
                assert_eq!(q.kind, QuestionKind::Choice);
                let order: Vec<&str> = q.options.iter().map(|(o, _)| o.as_str()).collect();
                assert_eq!(order, RELATION_OPTIONS);
            } else {
                assert_eq!(q.kind, QuestionKind::Noul);
            }
        }
    }

    #[test]
    fn relation_options_put_supports_last() {
        assert_eq!(
            RELATION_OPTIONS,
            ["says_nothing", "contradicts", "supports"]
        );
    }

    #[test]
    fn every_question_has_a_version_and_ids_are_unique() {
        let mut all = turn_review_questions();
        all.extend(research_claim_questions(3));
        assert!(all.iter().all(|q| q.version >= 1));
        let ids: BTreeSet<&str> = all.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids.len(), all.len());
    }
}
