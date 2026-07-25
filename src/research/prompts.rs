//! The research pipeline's model-hop bundles: prompt templates, constrained
//! output shapes (flat + closed — Principle II), and their boot-time
//! registration. Internal only — never in the tool catalog.

use crate::error::AppError;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::research::extract;
use serde::Deserialize;

/// Registry id of the scope mode.
pub const SCOPE_MODE_ID: &str = "research_scope";
/// Registry id of the claim-extraction mode.
pub const EXTRACT_MODE_ID: &str = "research_extract";
/// Registry id of the synthesis mode.
pub const SYNTH_MODE_ID: &str = "research_synthesize";

const SCOPE_PROMPT_TEMPLATE: &str = "\
You are scoping a web research run. Decompose the question into distinct \
search angles (each a search query a web search engine can answer, no more \
than <<angles_max>>) and the falsifiable sub-questions a good answer must \
settle (no more than 7). Angles must differ materially — different terms, \
different facets — not rephrasings. If a key term in the question has \
multiple well-known referents, every angle must target the referent the \
question intends — never a same-named unrelated subject.<<focus_clause>>\
\n\nQuestion: <<question>>";

/// Refute-biased verification template (research.md 004 D3, amended
/// 2026-07-24): same schema and ensemble machinery as `verify`, adversarial
/// stance, judged against the source excerpts the pipeline places in the
/// context — never the judge's own priors, which systematically refute true
/// post-cutoff facts. The placeholders are the ones `verify::run`'s prompt
/// builder replaces.
const RESEARCH_VERIFY_TEMPLATE: &str = "\
You are an adversarial fact-checker. Attempt to REFUTE the claim below \
against the source excerpts provided in the context. The excerpts are the \
evidence under test: judge from them, not from memory — your knowledge may \
be stale, and a claim your memory cannot confirm is not thereby false. \
Default to refuted when the excerpts cannot establish the claim: \
uncertainty is a refutation, and your refutation must name exactly what \
the excerpts could not establish or what contradicts the claim. Return \
supported only when the claim withstands your attempt against the \
excerpts.\n\nClaim: <<claim>>\n\nContext: <<context>>";

const SYNTH_PROMPT_TEMPLATE: &str = "\
You are writing the executive synthesis of a completed research run. Use \
ONLY the verified findings below — cite them inline with their source \
tokens exactly as given (for example [s3]). Never cite a source not listed. \
Surface uncertainty honestly; never state refuted or unverified content as \
fact. List what remains unanswered as gaps (short phrases). For EVERY gap, \
give the matching entry in gap_targets IN THE SAME ORDER: gap_targets[i] is \
the 1-based number of the sub-question that gaps[i] concerns, counting the \
sub-questions listed below from 1, or 0 when the gap concerns no single one \
of them. The two arrays MUST be the same length.\
<<retry_clause>>\n\nQuestion: <<question>>\n\nSub-questions a good answer \
settles:\n<<sub_questions>>\n\nVerified findings (cite by token):\n\
<<findings>>\n\nRefuted during verification (do NOT assert; you may note \
the refutation):\n<<refuted>>";

/// The scope call's constrained output (flat + closed).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ScopeOut {
    /// Search angles.
    pub(crate) angles: Vec<String>,
    /// Falsifiable sub-questions.
    pub(crate) sub_questions: Vec<String>,
}

/// The synthesis call's constrained output (flat + closed; local validator
/// enforces the length bounds the provider grammar drops).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SynthOut {
    /// The answer prose with inline `[sN]` citations.
    #[schemars(length(max = 8000))]
    pub(crate) answer: String,
    /// Unanswered sub-questions / honest gaps.
    #[schemars(length(max = 10), inner(length(max = 500)))]
    pub(crate) gaps: Vec<String>,
    /// Index-aligned with [`Self::gaps`]: entry *i* is the 1-based
    /// sub-question that `gaps[i]` concerns, or `0` for none (021 FR-003).
    ///
    /// Parallel arrays rather than a list of objects because Principle II
    /// requires mode schemas to stay flat and closed — the same idiom
    /// `decide` uses between `option_scores` and `option_rationales`. Arity
    /// and range are checked at assembly, since JSON Schema cannot state a
    /// relation between two arrays.
    #[schemars(length(max = 10))]
    pub(crate) gap_targets: Vec<u32>,
}

/// Register the research-internal modes (boot-time; enforces flat+closed).
/// These never appear in the tool catalog — they are prompt+schema bundles
/// for the pipeline's model hops.
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry) -> Result<(), AppError> {
    let scope_schema = serde_json::to_value(schemars::schema_for!(ScopeOut))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        SCOPE_MODE_ID,
        "internal: research scope decomposition",
        SCOPE_PROMPT_TEMPLATE,
        scope_schema,
        1,
    )?;
    let extract_schema = serde_json::to_value(schemars::schema_for!(extract::ExtractOut))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        EXTRACT_MODE_ID,
        "internal: research claim extraction",
        extract::EXTRACT_PROMPT_TEMPLATE,
        extract_schema,
        1,
    )?;
    let synth_schema = serde_json::to_value(schemars::schema_for!(SynthOut))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        SYNTH_MODE_ID,
        "internal: research synthesis",
        SYNTH_PROMPT_TEMPLATE,
        synth_schema,
        1,
    )
}

/// Build the refute-biased verify mode from the registered verify mode:
/// same schema, adversarial template (research.md 004 D3).
#[must_use]
pub fn research_verify_mode(verify_mode: &CorrectiveMode) -> CorrectiveMode {
    let mut mode = verify_mode.clone();
    mode.prompt_template = RESEARCH_VERIFY_TEMPLATE;
    mode
}
