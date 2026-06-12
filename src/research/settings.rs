//! Per-run effective settings: tier defaults with caller overrides
//! (FR-006), validated before any provider call (FR-010).

use crate::error::AppError;
use crate::research::contract::ResearchParams;
use crate::research::pipeline::ResearchDeps;
use crate::research::Depth;

/// Brave caps `count` at 20 per request.
pub const SEARCH_COUNT_MAX: u8 = 20;

/// Effective per-run settings: tier defaults with caller overrides (FR-006).
#[derive(Debug, Clone)]
pub struct RunSettings {
    pub(crate) angles: u8,
    pub(crate) max_sources: usize,
    pub(crate) verify_k: u8,
    pub(crate) deadline_ms: u64,
    pub(crate) budget_tokens: u64,
    pub(crate) domains_allow: Vec<String>,
    pub(crate) domains_deny: Vec<String>,
}

pub fn validate_params(
    deps: &ResearchDeps,
    params: &ResearchParams,
) -> Result<RunSettings, AppError> {
    let question = params.question.trim();
    if question.is_empty() {
        return Err(AppError::InvalidInput(
            "question is empty or whitespace-only".to_string(),
        ));
    }
    let len = params.question.chars().count();
    if len > deps.input_max_chars {
        return Err(AppError::InvalidInput(format!(
            "question is {len} characters; the configured maximum is {} (INPUT_MAX_CHARS); \
             it was not trimmed",
            deps.input_max_chars
        )));
    }
    if let Some(focus) = &params.focus {
        if focus.len() > 8 {
            return Err(AppError::InvalidInput(format!(
                "{} focus entries exceed the maximum of 8",
                focus.len()
            )));
        }
        if focus
            .iter()
            .any(|f| f.trim().is_empty() || f.chars().count() > 200)
        {
            return Err(AppError::InvalidInput(
                "every focus entry must be non-empty and at most 200 characters".to_string(),
            ));
        }
    }

    let tier = params.depth.unwrap_or(Depth::Standard).tier();
    let constraints = params.constraints.clone().unwrap_or_default();
    if let Some(n) = constraints.max_sources {
        if !(1..=60).contains(&n) {
            return Err(AppError::InvalidInput(format!(
                "max_sources {n} is out of range 1..=60"
            )));
        }
    }
    if let Some(b) = constraints.budget_tokens {
        if b < 1_000 {
            return Err(AppError::InvalidInput(format!(
                "budget_tokens {b} is below the minimum of 1000"
            )));
        }
    }
    if let Some(d) = constraints.deadline_ms {
        if d < 5_000 {
            return Err(AppError::InvalidInput(format!(
                "deadline_ms {d} is below the minimum of 5000"
            )));
        }
    }
    for (name, list) in [
        ("domains_allow", &constraints.domains_allow),
        ("domains_deny", &constraints.domains_deny),
    ] {
        if list.as_ref().is_some_and(|l| l.len() > 32) {
            return Err(AppError::InvalidInput(format!(
                "{name} exceeds the maximum of 32 entries"
            )));
        }
    }

    Ok(RunSettings {
        angles: tier.angles,
        max_sources: constraints
            .max_sources
            .map_or(tier.max_sources, |n| n as usize),
        verify_k: tier.verify_k,
        deadline_ms: constraints.deadline_ms.unwrap_or(tier.default_deadline_ms),
        budget_tokens: constraints
            .budget_tokens
            .unwrap_or(tier.default_budget_tokens),
        domains_allow: constraints.domains_allow.unwrap_or_default(),
        domains_deny: constraints.domains_deny.unwrap_or_default(),
    })
}

/// Per-angle search count: enough headroom for dedup losses, capped at the
/// provider's maximum.
pub fn per_angle_count(settings: &RunSettings, angles: usize) -> u8 {
    let want = (settings.max_sources * 2).div_ceil(angles.max(1));
    u8::try_from(want.clamp(1, usize::from(SEARCH_COUNT_MAX))).unwrap_or(SEARCH_COUNT_MAX)
}
