//! Preference-violation enforcement (015): deterministic candidate mining,
//! cooldown identity, and flag assembly — the enforce half of the
//! capture→store→recall→enforce loop (`PREFERENCE_ELICITATION.md`).
//!
//! The single review hop (`review.rs`) judges; everything here is pure. The
//! enforceable population is exactly the gate's constraint population
//! (`gate::is_constraint` — trusted lessons/facts): one definition of
//! "enforceable memory" across both boundaries (research D2). The cooldown
//! identity is the memory id (research D4) — stable across wording drift,
//! so one preference flags at most once per suppression window regardless
//! of how each violation reads.

use crate::checkpoint::{gate, Signal, SignalKind, REVIEW_CANDIDATES_MAX};
use crate::memory::{Memory, Trust};

/// One recalled enforceable memory, mined before the hop (data-model §2).
#[derive(Debug, Clone, PartialEq)]
pub struct PreferenceCandidate {
    /// `Memory.id` — provenance + cooldown identity.
    pub memory_id: String,
    /// Quoted verbatim in the flag.
    pub content: String,
    /// `FirstHand` or `Verified` only — `Untrusted` is structurally
    /// excluded (spec FR-005).
    pub trust: Trust,
    /// Cosine vs the final-message query embedding.
    pub score: f32,
}

/// Mine preference candidates from ranked recall.
///
/// The input is `rank_recall`'s output — already floored at
/// `REVIEW_RECALL_FLOOR` and sorted most-relevant-first — so mining is a
/// trust/kind filter plus the shared cap.
#[must_use]
pub fn mine_preference_candidates(recall: &[(f32, Memory)]) -> Vec<PreferenceCandidate> {
    let mut candidates: Vec<PreferenceCandidate> = recall
        .iter()
        .filter(|(_, memory)| gate::is_constraint(memory))
        .map(|(score, memory)| PreferenceCandidate {
            memory_id: memory.id.clone(),
            content: memory.content.clone(),
            trust: memory.trust,
            score: *score,
        })
        .collect();
    candidates.truncate(REVIEW_CANDIDATES_MAX);
    candidates
}

/// Build the violation signal + fixed flag message (data-model §4).
///
/// The identity is the memory id; the wording is a fixed template
/// parameterized only by server-held candidate evidence plus the hop's
/// one-sentence basis — never the model's free wording (research D5). The
/// closing clause implements FR-002's "fix or push back".
#[must_use]
pub fn violation_signal(candidate: &PreferenceCandidate, basis: &str) -> (Signal, String) {
    let signal = Signal::new(
        SignalKind::PreferenceViolation,
        format!(
            "stored preference \"{}\" (memory {}, {} provenance)",
            candidate.content,
            candidate.memory_id,
            candidate.trust.as_str()
        ),
        &candidate.memory_id,
    );
    let message = format!(
        "End-of-turn review: this turn appears to violate a stored preference: \
         \"{}\" (memory {}, {} provenance). Basis: {} Revise the response to \
         honor it, or state explicitly why it does not apply here.",
        candidate.content,
        candidate.memory_id,
        candidate.trust.as_str(),
        basis
    );
    (signal, message)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::memory::Kind;
    use chrono::{DateTime, Utc};

    fn fixed_now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-07-21T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn memory(id: &str, kind: Kind, trust: Trust) -> Memory {
        Memory {
            id: id.into(),
            content: format!("preference body of {id}"),
            kind,
            origin: "test".into(),
            external: false,
            trust,
            tags: vec![],
            embedding: vec![1.0, 0.0],
            embedding_model: "voyage-4".into(),
            created_at: fixed_now(),
            status: crate::memory::Status::Active,
            replaced_by: None,
            last_reinforced_at: fixed_now(),
        }
    }

    #[test]
    fn mining_excludes_untrusted_and_skills_and_keeps_rank_order() {
        let recall = vec![
            (0.9, memory("m-fact", Kind::Fact, Trust::FirstHand)),
            (0.8, memory("m-untrusted", Kind::Fact, Trust::Untrusted)),
            (0.7, memory("m-skill", Kind::Skill, Trust::Verified)),
            (0.6, memory("m-lesson", Kind::Lesson, Trust::Verified)),
        ];
        let mined = mine_preference_candidates(&recall);
        let ids: Vec<&str> = mined.iter().map(|c| c.memory_id.as_str()).collect();
        assert_eq!(ids, ["m-fact", "m-lesson"]);
        assert!(mined.iter().all(|c| c.trust.is_trusted()));
    }

    #[test]
    fn mining_caps_most_relevant_first() {
        let recall: Vec<(f32, Memory)> = (0..REVIEW_CANDIDATES_MAX + 3)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let score = 0.9 - (i as f32) * 0.01;
                (
                    score,
                    memory(&format!("m{i}"), Kind::Fact, Trust::FirstHand),
                )
            })
            .collect();
        let mined = mine_preference_candidates(&recall);
        assert_eq!(mined.len(), REVIEW_CANDIDATES_MAX);
        assert_eq!(mined[0].memory_id, "m0");
    }

    #[test]
    fn violation_identity_is_the_memory_id_stable_across_basis_wording() {
        let candidate = PreferenceCandidate {
            memory_id: "mem-42".into(),
            content: "never deploy on fridays".into(),
            trust: Trust::Verified,
            score: 0.8,
        };
        let (a, _) = violation_signal(&candidate, "the turn deployed on a friday.");
        let (b, _) = violation_signal(&candidate, "completely different basis wording.");
        assert_eq!(a.signal_key, b.signal_key);
        assert!(a.signal_key.starts_with("preference_violation:"));

        let other = PreferenceCandidate {
            memory_id: "mem-43".into(),
            ..candidate
        };
        let (c, _) = violation_signal(&other, "same basis.");
        assert_ne!(a.signal_key, c.signal_key);
    }

    #[test]
    fn violation_flag_quotes_content_id_trust_and_basis_and_invites_pushback() {
        let candidate = PreferenceCandidate {
            memory_id: "mem-42".into(),
            content: "never deploy on fridays".into(),
            trust: Trust::FirstHand,
            score: 0.8,
        };
        let (signal, message) = violation_signal(&candidate, "the turn deployed on a friday.");
        assert_eq!(signal.kind, SignalKind::PreferenceViolation);
        for text in [&signal.evidence, &message] {
            assert!(text.contains("never deploy on fridays"), "{text}");
            assert!(text.contains("mem-42"), "{text}");
            assert!(text.contains("first_hand"), "{text}");
        }
        assert!(
            message.contains("the turn deployed on a friday."),
            "{message}"
        );
        assert!(message.contains("why it does not apply"), "{message}");
    }
}
