//! The research layer: offloaded, cited, adversarially-verified answers
//! (the Research primitive, `RESEARCH_PRIMITIVE.md`).
//!
//! Wire types live in [`contract`]; the five-phase orchestration in
//! [`pipeline`]; everything checkable is settled by the pure functions in
//! [`verdict`] and [`grounding`], never by the model — the model writes only
//! the answer prose (research.md 004 D7).

pub mod contract;
pub mod evidence;
pub mod extract;
pub mod fetch;
pub mod grounding;
pub(crate) mod outcome;
pub mod pipeline;
pub mod prompts;
pub(crate) mod settings;
pub(crate) mod synthesis;
pub mod verdict;

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

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
    // Budget defaults were re-tuned against live runs (2026-06-12): the
    // corpus's 40k/120k/350k estimates starved real runs mid-verification.
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

/// Tokens one claim costs per verification pass, measured across three live
/// standard-tier runs (2 678 / 2 565 / 2 280; mean 2 508).
///
/// This is the one empirical input to the tier budgets. Everything else in the
/// derivation is a cap the tier already declares.
const TOKENS_PER_CLAIM_PASS: u64 = 2_508;

impl Depth {
    /// The token budget this tier's own caps imply: every source fetched, every
    /// source yielding the maximum claims, every claim verified by the full
    /// ensemble.
    ///
    /// **The budget is sized to this, not below it (024).** A default below the
    /// structural maximum is not a safety net — it is a silent quality cap. The
    /// three standard runs that produced [`TOKENS_PER_CLAIM_PASS`] each tripped
    /// their ceiling and dropped 20–45% of the claims they had already paid to
    /// extract, reporting only `stopped_early`. Sizing from the caps means the
    /// ceiling fires on genuine anomaly rather than on the tier working as
    /// specified.
    ///
    /// A caller who wants to spend less passes `budget_tokens` explicitly; the
    /// contract has always supported that, and it is the honest place for a
    /// cost decision because the caller sees the trade it is making.
    ///
    /// **The ceiling is soft, and sizing it depends on knowing that** (033).
    /// The budget is probed before and inside each unit of work, so exhaustion
    /// stops *new* tasks while in-flight ones finish. The three standard runs
    /// recorded 503k, 643k and 949k against a 450_000 cap — 1.1x to 2.1x over
    /// it. A nominal budget is therefore an instruction to stop starting, not
    /// a hard spend limit, and any figure here should be read as the point at
    /// which the run begins winding down rather than the most it can cost.
    ///
    /// That cuts both ways and is why the raise is defensible: the old 450_000
    /// was already yielding 949k runs, so the gap between nominal and actual
    /// was doing the work a larger nominal value does honestly.
    #[must_use]
    pub const fn structural_max_tokens(self) -> u64 {
        let tier = self.tier();
        let claims = tier.max_sources as u64 * MAX_CLAIMS_PER_SOURCE as u64;
        claims * tier.verify_k as u64 * TOKENS_PER_CLAIM_PASS
    }

    /// The wire spelling, matching the contract's `depth` enum. Used to stamp
    /// the invocation record so a recorded run can be attributed to its tier
    /// (019 — without this, no historical run tells you which ceiling it ran
    /// under, and the standard/deep budgets cannot be sized from data).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
            Self::Deep => "deep",
        }
    }

    /// The tier table (research.md 004 D8).
    #[must_use]
    pub const fn tier(self) -> DepthTier {
        match self {
            Self::Quick => DepthTier {
                angles: 3,
                max_sources: 8,
                verify_k: 1,
                default_deadline_ms: 120_000,
                // Raised 150k -> 350k (2026-07-24), derived from this tier's
                // own history rather than picked. The 004 evidence-grounding
                // fix gave each verification hop a real source excerpt instead
                // of a title, and the ceiling never moved, so every quick run
                // tripped it and dropped ~40% of its claims.
                //
                // The first attempt at this number was 250_000, from
                // 1.43 (the tier's original 150_000 / 104_783 headroom ratio)
                // times a post-004 measurement of 174,952. That measurement
                // was taken from a run that had ITSELF stopped early and
                // dropped 43 of 89 claims, so it was the cost of an incomplete
                // run and understated the real figure. A run that actually
                // completes measures 239,371 (77 claims extracted, 77
                // verified, 8/8 sources — near this tier's structural
                // maximum), which is 95.7% of 250_000. Applying 1.43 to the
                // complete-run cost gives ~342_000, rounded to 350_000.
                //
                // Sizing a ceiling from a run that hit that ceiling is
                // circular; the number it produces is always too low.
                //
                // 024 replaced that method entirely for standard and deep, and
                // this value is left alone because it already clears the same
                // bar: quick's structural maximum is 8 x 12 x 1 x 2_508 =
                // ~241_000, and 350_000 sits above it.
                //
                // That reasoning held until the record carried the depth.
                // 019 added it; 033 then read the history back and found
                // standard tripping 3 of 3 while quick trips 0 of 7 with a
                // median of 239k. Both were raised in 033 — standard on that
                // evidence, deep on the ordering invariant. Quick is the tier
                // this method was proven on and needs no change.
                default_budget_tokens: 350_000,
            },
            Self::Standard => DepthTier {
                angles: 5,
                max_sources: 25,
                verify_k: 2,
                default_deadline_ms: 240_000,
                // 450_000 -> 1_600_000 (033). The tier's own structural
                // maximum: 25 sources x 12 claims x 2 passes x 2_508 tokens
                // per claim per pass — the caps this tier already declares,
                // multiplied out.
                //
                // **Measured, then derived.** Every standard run on record
                // hit the old ceiling — 3 of 3, spending 503k, 643k and 949k
                // against a 450k cap. That is the base rate, not an anecdote:
                // there is no standard run that finished inside 450_000.
                //
                // The number is still structural rather than fitted to those
                // three, deliberately. Fitting to runs that *stopped early*
                // is how the quick tier got 250_000 — derived from a run that
                // had itself been truncated, and corrected to 350_000 once
                // that circularity was noticed. Quick now sits at 0 of 7
                // trips with a median of 239k, which is what a budget with
                // real headroom looks like.
                default_budget_tokens: 1_600_000,
            },
            Self::Deep => DepthTier {
                angles: 8,
                max_sources: 60,
                verify_k: 3,
                default_deadline_ms: 480_000,
                // 1_000_000 -> 5_500_000 (033): 60 x 12 x 3 x 2_508 =
                // 5_417_000, rounded up. The same formula as standard, over
                // caps this tier already declares.
                //
                // **This is the weaker of the two raises and should be read
                // that way.** No deep run has ever executed — zero rows in
                // `invocation_records` carry `depth='deep'` — so unlike
                // standard, which tripped 3 of 3, there is no observation
                // here at all. It was nearly held back for exactly that
                // reason.
                //
                // What forces it is the tier ordering. Deep declares strictly
                // more than standard in every dimension (60 sources vs 25,
                // verify_k 3 vs 2), so its structural maximum is necessarily
                // larger, and a deep budget *below* standard's would make the
                // more thorough tier truncate harder — incoherent, and caught
                // by `tier_table_matches_the_design` when this was first
                // written the other way. Accepting the formula for standard,
                // which the trip data justifies, means accepting it here.
                //
                // Revisit when a deep run exists to measure against. Until
                // then this is arithmetic over declared caps, not evidence.
                default_budget_tokens: 5_500_000,
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

/// Shared run accounting: token sums double as the budget meter.
#[derive(Default)]
pub(crate) struct RunMeter {
    input_tokens: AtomicU64,
    output_tokens: AtomicU64,
    /// 018 T032: the same tokens, kept per model so the invocation record can
    /// price each at its own rate. The atomics above remain the budget's hot
    /// path — ceilings are denominated in tokens, not currency, so budget and
    /// deadline behavior is unchanged by routing.
    by_model: std::sync::Mutex<crate::telemetry::ModelUsage>,
}

impl RunMeter {
    pub(crate) fn add(&self, model: &str, input: u64, output: u64) {
        self.input_tokens.fetch_add(input, Ordering::Relaxed);
        self.output_tokens.fetch_add(output, Ordering::Relaxed);
        if let Ok(mut usage) = self.by_model.lock() {
            usage.add(model, input, output);
        }
    }
    pub(crate) fn total(&self) -> u64 {
        self.input_tokens() + self.output_tokens()
    }
    pub(crate) fn input_tokens(&self) -> u64 {
        self.input_tokens.load(Ordering::Relaxed)
    }
    pub(crate) fn output_tokens(&self) -> u64 {
        self.output_tokens.load(Ordering::Relaxed)
    }
    /// Per-model usage accumulated so far.
    pub(crate) fn usage(&self) -> crate::telemetry::ModelUsage {
        self.by_model
            .lock()
            .map(|usage| usage.clone())
            .unwrap_or_default()
    }
}

/// One fetched-and-extracted source (internal).
pub(crate) struct SourceRecord {
    pub(crate) id: String,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) fetched_at: String,
    pub(crate) credibility: f32,
    pub(crate) claims: Vec<String>,
    /// Readable text retained for the verification hop's evidence context
    /// (004 D3 amendment). Internal only — never on the wire (FR-012).
    pub(crate) text: String,
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
        // 019: the quick ceiling is pinned to the value derived from the
        // tier's own measured consumption (see the comment on the table). A
        // silent drift back below the post-004 cost is what dropped ~40% of a
        // run's claims, so the number is asserted, not merely commented.
        assert_eq!(Depth::Quick.tier().default_budget_tokens, 350_000);
        // The tier a record is stamped with must match the contract spelling.
        assert_eq!(Depth::Quick.as_str(), "quick");
        assert_eq!(Depth::Standard.as_str(), "standard");
        assert_eq!(Depth::Deep.as_str(), "deep");
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
