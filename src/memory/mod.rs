//! The memory layer: durable cross-session memory with verified-before-stored
//! trust (the Recall corrective, `MEMORY_LAYER.md`).
//!
//! Not a registry mode — memory tools have no prompt template and no model
//! hop for their outputs, so the grammar subset and the flat invariant do not
//! apply to them (research.md 003 D6). They share the seams, the error
//! taxonomy, and the recorded execution path.

pub mod consolidate;
pub mod contract;
pub mod push;
pub mod ranking;
pub mod tools;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What a memory is (data-model.md §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A reusable approach that worked (procedural).
    Skill,
    /// What failed and why (episodic → semantic).
    Lesson,
    /// Durable knowledge (semantic).
    Fact,
}

impl Kind {
    /// Stable string form (the `kind` column).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Lesson => "lesson",
            Self::Fact => "fact",
        }
    }

    /// Parse the stable string form (storage read path).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "skill" => Some(Self::Skill),
            "lesson" => Some(Self::Lesson),
            "fact" => Some(Self::Fact),
            _ => None,
        }
    }
}

/// Consolidation status (017 data-model §1): only `Active` records
/// participate in retrieval; all records remain inspectable. Content is
/// never modified by a status change (FR-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Participates in retrieval.
    Active,
    /// Replaced as current truth by a newer admission (`replaced_by`).
    Superseded,
    /// Unified into a canonical record (`replaced_by`).
    Merged,
}

impl Status {
    /// Stable string form (the `status` column).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Superseded => "superseded",
            Self::Merged => "merged",
        }
    }

    /// Parse the stable string form (storage read path).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "superseded" => Some(Self::Superseded),
            "merged" => Some(Self::Merged),
            _ => None,
        }
    }

    /// Whether this record participates in retrieval (FR-011).
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Trust standing — **derived, never caller-set** (research.md 003 D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// First-hand experience of the calling session.
    FirstHand,
    /// External provenance, admitted through an independent verification pass.
    Verified,
    /// External provenance, unverified — stored quarantined and down-ranked.
    Untrusted,
}

impl Trust {
    /// Trusted tiers never rank below `Untrusted` at comparable relevance.
    #[must_use]
    pub const fn is_trusted(self) -> bool {
        matches!(self, Self::FirstHand | Self::Verified)
    }

    /// Stable string form (the `trust` column).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirstHand => "first_hand",
            Self::Verified => "verified",
            Self::Untrusted => "untrusted",
        }
    }

    /// Parse the stable string form (storage read path).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "first_hand" => Some(Self::FirstHand),
            "verified" => Some(Self::Verified),
            "untrusted" => Some(Self::Untrusted),
            _ => None,
        }
    }
}

/// One stored memory (table `memories`, data-model.md §1).
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    /// UUID v4.
    pub id: String,
    /// The memory itself.
    pub content: String,
    /// skill | lesson | fact.
    pub kind: Kind,
    /// Caller-stated provenance.
    pub origin: String,
    /// External content (the poisoning pivot) vs first-hand.
    pub external: bool,
    /// Derived trust standing.
    pub trust: Trust,
    /// Optional tags.
    pub tags: Vec<String>,
    /// Document-type embedding.
    pub embedding: Vec<f32>,
    /// Embedding model id (mismatch detection across model switches).
    pub embedding_model: String,
    /// RFC 3339 via `TimeProvider`.
    pub created_at: DateTime<Utc>,
    /// Consolidation status (017) — only `Active` participates in retrieval.
    pub status: Status,
    /// The superseding / canonical record's id, when status is not active.
    pub replaced_by: Option<String>,
    /// Decay clock (017 research D5): refreshed when returned by recall or
    /// surfaced by push; backfilled to `created_at` at migration.
    pub last_reinforced_at: DateTime<Utc>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn kind_and_trust_round_trip_their_stable_strings() {
        for kind in [Kind::Skill, Kind::Lesson, Kind::Fact] {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        for trust in [Trust::FirstHand, Trust::Verified, Trust::Untrusted] {
            assert_eq!(Trust::parse(trust.as_str()), Some(trust));
        }
        assert!(Trust::FirstHand.is_trusted());
        assert!(Trust::Verified.is_trusted());
        assert!(!Trust::Untrusted.is_trusted());
    }
}
