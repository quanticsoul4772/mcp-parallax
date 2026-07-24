//! The pre-action gate (D4) — pure decision functions.
//!
//! Risk matching selects which actions get evaluated at all (FR-013);
//! constraint relevance decides the hold: a verified, constraint-kind
//! stored memory whose relevance to the pending action clears τ. No model
//! pass — the user judges actual contradiction at the hold (FR-011 makes
//! that honest).

use crate::checkpoint::{Signal, SignalKind, GATE_RELEVANCE_TAU, GATE_RISK_PATTERNS};
use crate::memory::ranking::cosine;
use crate::memory::{Kind, Memory};

/// Whether a pending action matches the risk-pattern set (FR-013):
/// case-insensitive substring match over `tool_name + " " + tool_input`,
/// built-in patterns plus the `CHECKPOINT_GATE_PATTERNS` extras.
#[must_use]
pub fn risk_matched(tool_name: &str, tool_input: &str, extra_patterns: &[String]) -> bool {
    let haystack = format!("{tool_name} {tool_input}").to_lowercase();
    GATE_RISK_PATTERNS
        .iter()
        .any(|pattern| haystack.contains(pattern))
        || extra_patterns
            .iter()
            .any(|pattern| !pattern.is_empty() && haystack.contains(&pattern.to_lowercase()))
}

/// A memory qualifies as a gate constraint when it is durable knowledge or
/// a lesson (not a skill), trusted, and active.
///
/// Trust is earned, never claimed; active-ness is 017 FR-011 — a superseded
/// constraint stops holding, exactly as it stops being enforced at turn end.
#[must_use]
pub fn is_constraint(memory: &Memory) -> bool {
    matches!(memory.kind, Kind::Lesson | Kind::Fact)
        && memory.trust.is_trusted()
        && memory.status.is_active()
}

/// The deterministic hold decision (D4).
///
/// Holds on the most relevant constraint-kind memory at or above
/// [`GATE_RELEVANCE_TAU`], if any. Returns the memory's content (quoted
/// verbatim in the hold reason) and the fired signal.
#[must_use]
pub fn constraint_hold(query: &[f32], memories: &[Memory]) -> Option<(Signal, String)> {
    let mut best: Option<(f32, &Memory)> = None;
    for memory in memories.iter().filter(|m| is_constraint(m)) {
        let score = cosine(query, &memory.embedding);
        if score >= GATE_RELEVANCE_TAU && best.is_none_or(|(top, _)| score > top) {
            best = Some((score, memory));
        }
    }
    best.map(|(score, memory)| {
        let signal = Signal::new(
            SignalKind::MemoryConflict,
            format!(
                "the pending action is highly relevant (score {score:.2}) to the stored \
                 {} \"{}\"",
                memory.kind.as_str(),
                memory.content
            ),
            &memory.id,
        );
        (signal, memory.content.clone())
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::memory::Trust;
    use chrono::{DateTime, Utc};

    // FR-013 pattern table: defaults, extras, and non-risk passes.
    #[test]
    fn risk_pattern_ground_truth() {
        let cases = [
            ("bash", "git push origin main --force", true),
            ("bash", "kubectl apply -f prod.yaml", true),
            ("bash", "rm -rf ./build", true),
            ("bash", "terraform apply", true),
            ("bash", "cargo deploy-docs", true), // substring: over-selects, never over-holds
            ("write", "DELETE FROM users", true),
            ("bash", "cargo test", false),
            ("read", "src/main.rs", false),
            ("grep", "pattern in files", false),
        ];
        for (tool, input, expected) in cases {
            assert_eq!(risk_matched(tool, input, &[]), expected, "{tool} {input}");
        }
    }

    #[test]
    fn extra_patterns_extend_the_builtin_set_case_insensitively() {
        assert!(!risk_matched("bash", "systemctl restart nginx", &[]));
        assert!(risk_matched(
            "bash",
            "systemctl restart nginx",
            &["SYSTEMCTL".to_string()]
        ));
        // Empty extras never match everything.
        assert!(!risk_matched("read", "a.rs", &[String::new()]));
    }

    fn memory(id: &str, kind: Kind, trust: Trust, embedding: Vec<f32>) -> Memory {
        Memory {
            id: id.to_string(),
            content: format!("constraint {id}: deployments go through staging first"),
            kind,
            origin: "test".into(),
            external: false,
            trust,
            tags: vec![],
            embedding,
            embedding_model: "voyage-4".into(),
            created_at: DateTime::parse_from_rfc3339("2026-06-12T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            status: crate::memory::Status::Active,
            replaced_by: None,
            last_reinforced_at: DateTime::parse_from_rfc3339("2026-06-12T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    // 017 FR-011: a superseded constraint stops holding.
    #[test]
    fn superseded_constraints_never_hold() {
        let mut superseded = memory("m1", Kind::Lesson, Trust::FirstHand, vec![1.0, 0.0]);
        superseded.status = crate::memory::Status::Superseded;
        assert!(!is_constraint(&superseded));
        assert!(constraint_hold(&[1.0, 0.0], &[superseded]).is_none());
    }

    // D4 threshold edges: ≥ τ holds, < τ silent, non-constraint kinds and
    // untrusted memories never hold, empty store silent.
    #[test]
    fn relevance_threshold_edges() {
        let query = vec![1.0, 0.0];
        // cosine(query, [1,0]) = 1.0 ≥ τ.
        let relevant = memory("m1", Kind::Lesson, Trust::FirstHand, vec![1.0, 0.0]);
        let (signal, content) = constraint_hold(&query, std::slice::from_ref(&relevant)).unwrap();
        assert_eq!(signal.kind, SignalKind::MemoryConflict);
        assert!(content.contains("staging"));
        assert!(signal.evidence.contains("staging"));

        // Below τ: cosine = 0.0.
        let irrelevant = memory("m2", Kind::Fact, Trust::Verified, vec![0.0, 1.0]);
        assert!(constraint_hold(&query, &[irrelevant]).is_none());

        // A skill never gates, however relevant.
        let skill = memory("m3", Kind::Skill, Trust::FirstHand, vec![1.0, 0.0]);
        assert!(constraint_hold(&query, &[skill]).is_none());

        // Untrusted memories never gate (verified-before-stored honesty).
        let untrusted = memory("m4", Kind::Fact, Trust::Untrusted, vec![1.0, 0.0]);
        assert!(constraint_hold(&query, &[untrusted]).is_none());

        // Empty store: silent.
        assert!(constraint_hold(&query, &[]).is_none());
    }

    #[test]
    fn the_most_relevant_constraint_wins() {
        let query = vec![1.0, 0.0];
        let weaker = memory("weak", Kind::Fact, Trust::Verified, vec![0.8, 0.6]);
        let stronger = memory("strong", Kind::Lesson, Trust::FirstHand, vec![1.0, 0.1]);
        let (signal, _) = constraint_hold(&query, &[weaker, stronger]).unwrap();
        assert!(signal.evidence.contains("strong"));
    }
}
