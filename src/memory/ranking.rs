//! Recall ranking — pure, deterministic functions (research.md 003 D4).
//!
//! `effective = cosine + RECENCY_WEIGHT × 2^(−age_days/30) + (ε if trusted)`.
//! Adding ε to *trusted* memories implements the FR-004 band as a clean total
//! order: an untrusted memory outranks a trusted one only when its relevance
//! advantage exceeds ε. The reported `score` stays the raw relevance (cosine).

use crate::memory::Memory;
use chrono::{DateTime, Utc};

/// The trust band: untrusted must beat trusted by more than this much
/// relevance to outrank it (FR-004).
pub const TRUST_EPSILON: f32 = 0.05;

/// Weight of the recency term — small enough that relevance dominates,
/// large enough to break near-ties.
pub const RECENCY_WEIGHT: f32 = 0.02;

/// Recency half-life in days.
pub const RECENCY_HALF_LIFE_DAYS: f32 = 30.0;

/// Cosine similarity; 0.0 for degenerate (zero or mismatched-length) vectors.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= f32::EPSILON {
        0.0
    } else {
        dot / denom
    }
}

/// Exponential recency decay in (0, 1].
#[must_use]
pub fn recency_decay(created_at: DateTime<Utc>, now: DateTime<Utc>) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let age_days = ((now - created_at).num_seconds().max(0) as f32) / 86_400.0;
    (-age_days / RECENCY_HALF_LIFE_DAYS * std::f32::consts::LN_2).exp()
}

/// A ranked memory: the raw relevance reported to the caller, the effective
/// score used for ordering.
#[derive(Debug)]
pub struct Ranked {
    /// The memory.
    pub memory: Memory,
    /// Raw relevance (cosine) — what the contract reports as `score`.
    pub relevance: f32,
}

/// Rank memories against a query embedding: effective-score descending,
/// deterministic tie-break by id.
#[must_use]
pub fn rank(memories: Vec<Memory>, query: &[f32], now: DateTime<Utc>) -> Vec<Ranked> {
    let mut scored: Vec<(f32, Ranked)> = memories
        .into_iter()
        .map(|memory| {
            let relevance = cosine(query, &memory.embedding);
            let mut effective =
                RECENCY_WEIGHT.mul_add(recency_decay(memory.created_at, now), relevance);
            if memory.trust.is_trusted() {
                effective += TRUST_EPSILON;
            }
            (effective, Ranked { memory, relevance })
        })
        .collect();

    scored.sort_by(|(ea, ra), (eb, rb)| {
        eb.total_cmp(ea)
            .then_with(|| ra.memory.id.cmp(&rb.memory.id))
    });
    scored.into_iter().map(|(_, ranked)| ranked).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(clippy::float_cmp)] // degenerate inputs return exactly 0.0 by contract
mod tests {
    use super::*;
    use crate::memory::{Kind, Trust};

    fn memory(id: &str, embedding: Vec<f32>, trust: Trust, age_days: i64) -> Memory {
        Memory {
            id: id.to_string(),
            content: format!("content {id}"),
            kind: Kind::Skill,
            origin: "test".into(),
            external: trust == Trust::Untrusted,
            trust,
            tags: vec![],
            embedding,
            embedding_model: "voyage-4".into(),
            created_at: Utc::now() - chrono::Duration::days(age_days),
        }
    }

    #[test]
    fn cosine_basics_and_degenerate_inputs() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0); // length mismatch
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0); // zero vector
    }

    // FR-004 clause 1: relevance dominates beyond the band.
    #[test]
    fn clearly_more_relevant_untrusted_outranks_trusted() {
        let query = [1.0, 0.0];
        let ranked = rank(
            vec![
                memory("trusted-far", vec![0.5, 0.86], Trust::FirstHand, 0),
                memory("untrusted-close", vec![1.0, 0.05], Trust::Untrusted, 0),
            ],
            &query,
            Utc::now(),
        );
        assert_eq!(ranked[0].memory.id, "untrusted-close");
    }

    // FR-004 clause 3: untrusted never outranks trusted at comparable relevance.
    #[test]
    fn comparable_relevance_puts_trusted_first() {
        let query = [1.0, 0.0];
        let ranked = rank(
            vec![
                memory("untrusted", vec![1.0, 0.0], Trust::Untrusted, 0),
                memory("trusted", vec![0.999, 0.04], Trust::Verified, 0),
            ],
            &query,
            Utc::now(),
        );
        // Untrusted has marginally higher cosine, but within ε — trusted wins.
        assert_eq!(ranked[0].memory.id, "trusted");
        // The reported relevance is still the raw cosine (untrusted's higher).
        assert!(ranked[1].relevance > ranked[0].relevance);
    }

    // FR-004 clause 2: recency breaks near-ties.
    #[test]
    fn recency_breaks_near_ties_within_a_tier() {
        let query = [1.0, 0.0];
        let ranked = rank(
            vec![
                memory("old", vec![1.0, 0.0], Trust::FirstHand, 300),
                memory("new", vec![1.0, 0.0], Trust::FirstHand, 0),
            ],
            &query,
            Utc::now(),
        );
        assert_eq!(ranked[0].memory.id, "new");
    }

    #[test]
    fn ordering_is_deterministic_on_exact_ties() {
        let query = [1.0, 0.0];
        let a = rank(
            vec![
                memory("b", vec![1.0, 0.0], Trust::FirstHand, 0),
                memory("a", vec![1.0, 0.0], Trust::FirstHand, 0),
            ],
            &query,
            Utc::now(),
        );
        assert_eq!(a[0].memory.id, "a"); // id tie-break
    }
}
