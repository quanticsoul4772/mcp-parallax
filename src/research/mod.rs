//! The research layer: offloaded, cited, adversarially-verified answers
//! (the Research primitive, `RESEARCH_PRIMITIVE.md`).
//!
//! Wire types live in [`contract`]; the five-phase orchestration in
//! [`pipeline`]; everything checkable is settled by the pure functions in
//! [`verdict`] and [`grounding`], never by the model — the model writes only
//! the answer prose (research.md 004 D7).

pub mod contract;
pub mod extract;
pub mod fetch;
pub mod grounding;
pub mod pipeline;
pub mod prompts;
pub(crate) mod settings;
pub mod verdict;

use serde::{Deserialize, Serialize};

/// Maximum sub-questions a scope call may produce.
pub const MAX_SUB_QUESTIONS: usize = 7;
/// Maximum claims extracted per source.
pub const MAX_CLAIMS_PER_SOURCE: usize = 12;
/// Maximum synthesis answer length in characters.
pub const MAX_ANSWER_CHARS: usize = 8_000;
/// Maximum gap entries and per-gap length.
pub const MAX_GAPS: usize = 10;
/// Maximum characters per gap entry.
pub const MAX_GAP_CHARS: usize = 500;

/// Rigor tier (contract `depth`; research.md 004 D8). Exhaustive is deferred
/// by spec assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Depth {
    /// 3 angles, 8 sources, 1 vote — a quick look.
    Quick,
    /// 5 angles, 25 sources, 2 votes — the default.
    Standard,
    /// 8 angles, 60 sources, 3 votes — a deep investigation.
    Deep,
}

/// Tier defaults; explicit caller constraints always override (FR-006).
#[derive(Debug, Clone, Copy)]
pub struct DepthTier {
    /// Search angles produced by scope.
    pub angles: u8,
    /// Hard cap on fetched sources.
    pub max_sources: usize,
    /// Verification votes per claim.
    pub verify_k: u8,
    /// Default wall-clock ceiling.
    pub default_deadline_ms: u64,
    /// Default token ceiling.
    pub default_budget_tokens: u64,
}

impl Depth {
    /// The tier table (research.md 004 D8).
    #[must_use]
    pub const fn tier(self) -> DepthTier {
        match self {
            Self::Quick => DepthTier {
                angles: 3,
                max_sources: 8,
                verify_k: 1,
                default_deadline_ms: 90_000,
                default_budget_tokens: 40_000,
            },
            Self::Standard => DepthTier {
                angles: 5,
                max_sources: 25,
                verify_k: 2,
                default_deadline_ms: 240_000,
                default_budget_tokens: 120_000,
            },
            Self::Deep => DepthTier {
                angles: 8,
                max_sources: 60,
                verify_k: 3,
                default_deadline_ms: 480_000,
                default_budget_tokens: 350_000,
            },
        }
    }
}

/// Support standing of a verified claim (FR-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Support {
    /// Supported by the votes with ≥ 2 independent sources.
    Confirmed,
    /// Votes split within the band — surfaced, never resolved.
    Contested,
    /// Refuted by the votes — excluded from the answer body, counted.
    Refuted,
    /// Supported but single-sourced — never stated as fact.
    Unverified,
}

/// The scope phase's output: angles to search, sub-questions a good answer
/// must settle.
#[derive(Debug, Clone)]
pub struct ScopePlan {
    /// Search angles (≤ tier angles).
    pub angles: Vec<String>,
    /// Falsifiable sub-questions (≤ [`MAX_SUB_QUESTIONS`]).
    pub sub_questions: Vec<String>,
}

/// One falsifiable claim with its backing sources (internal — never on the
/// wire raw).
#[derive(Debug, Clone)]
pub struct Claim {
    /// The claim text as extracted.
    pub text: String,
    /// Source ids backing it (grows on dedup merge).
    pub source_ids: Vec<String>,
}

/// A claim after verification.
#[derive(Debug, Clone)]
pub struct VerifiedClaim {
    /// The claim and its sources.
    pub claim: Claim,
    /// Support standing (order-sensitive mapping, `verdict.rs`).
    pub support: Support,
    /// Post-verification confidence (0..=1).
    pub confidence: f32,
    /// Refutation/support findings from the winning side.
    pub findings: Vec<String>,
}

/// Normalized dedup key for claims: lowercase, alphanumeric words joined by
/// single spaces (research.md 004 D6 — deterministic, conservative).
#[must_use]
pub fn claim_key(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalized dedup key for URLs across angles: scheme dropped, host
/// lowercased, fragment and trailing slash stripped.
#[must_use]
pub fn url_key(url: &str) -> String {
    let no_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let no_fragment = no_scheme.split('#').next().unwrap_or(no_scheme);
    let trimmed = no_fragment.trim_end_matches('/');
    // Host is case-insensitive; path is not — lowercase only the host part.
    match trimmed.split_once('/') {
        Some((host, path)) => format!("{}/{}", host.to_lowercase(), path),
        None => trimmed.to_lowercase(),
    }
}

/// The registrable-domain suffix match used by allow/deny lists: `host`
/// matches `domain` when equal or a dot-boundary suffix
/// (`docs.example.com` matches `example.com`, not `notexample.com`).
#[must_use]
pub fn domain_matches(host: &str, domain: &str) -> bool {
    let host = host.to_lowercase();
    let domain = domain.to_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn tier_table_matches_the_design() {
        assert_eq!(Depth::Quick.tier().angles, 3);
        assert_eq!(Depth::Quick.tier().verify_k, 1);
        assert_eq!(Depth::Standard.tier().max_sources, 25);
        assert_eq!(Depth::Deep.tier().angles, 8);
        assert_eq!(Depth::Deep.tier().verify_k, 3);
        // Tiers are strictly increasing in every scaling dimension.
        for (lo, hi) in [
            (Depth::Quick, Depth::Standard),
            (Depth::Standard, Depth::Deep),
        ] {
            assert!(lo.tier().angles < hi.tier().angles);
            assert!(lo.tier().max_sources < hi.tier().max_sources);
            assert!(lo.tier().verify_k <= hi.tier().verify_k);
            assert!(lo.tier().default_budget_tokens < hi.tier().default_budget_tokens);
        }
    }

    #[test]
    fn claim_key_normalizes_case_whitespace_and_punctuation() {
        assert_eq!(
            claim_key("The  Moon landing was in 1969."),
            claim_key("the moon landing was in 1969")
        );
        assert_eq!(claim_key("A—B"), "a b");
        assert_ne!(claim_key("rust is fast"), claim_key("rust is safe"));
    }

    #[test]
    fn url_key_dedups_scheme_fragment_and_trailing_slash() {
        assert_eq!(
            url_key("https://Example.com/Path/"),
            url_key("http://example.com/Path")
        );
        assert_eq!(
            url_key("https://example.com/a#section"),
            url_key("https://example.com/a")
        );
        // Path case is significant; host case is not.
        assert_ne!(
            url_key("https://example.com/Path"),
            url_key("https://example.com/path")
        );
    }

    #[test]
    fn domain_matching_is_suffix_at_dot_boundaries() {
        assert!(domain_matches("example.com", "example.com"));
        assert!(domain_matches("docs.example.com", "example.com"));
        assert!(domain_matches("Docs.Example.COM", "example.com"));
        assert!(!domain_matches("notexample.com", "example.com"));
        assert!(!domain_matches("example.com.evil.net", "example.com"));
    }
}
