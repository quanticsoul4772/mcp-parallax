//! The five-phase research pipeline (RESEARCH_PRIMITIVE.md §2; FR-002).
//!
//! Scope (1 call) → angle searches (concurrent, URL-dedup barrier) →
//! fetch+extract (per-source pipeline, no cross-source barrier) → verify
//! (fan-out per deduped claim, refute-biased ensemble) → synthesize
//! ([`crate::research::synthesis`]: the model writes prose only) → the
//! grounding gate (one retry, then demotion).
//!
//! Budget/deadline are enforced ceilings, probed before *and inside* every
//! unit of work: between phases, before each spawn, and again after each
//! task acquires its concurrency permit — a mid-phase budget blowout stops
//! the remaining tasks, not just the next phase (FR-007).

use crate::error::AppError;
use crate::modes::verify::{self, VerifyParams};
use crate::modes::CorrectiveMode;
use crate::research::contract::{ResearchParams, ResearchResult, SourceRef, Stats, StopReason};
use crate::research::extract;
use crate::research::prompts::ScopeOut;
use crate::research::settings::{per_angle_count, validate_params, RunSettings};
use crate::research::synthesis::{assemble, synthesize_grounded};
use crate::research::verdict::{self, source_credibility};
use crate::research::{
    claim_key, domain_matches, url_key, Claim, RunMeter, ScopePlan, SourceRecord, Support,
    VerifiedClaim, MAX_SUB_QUESTIONS,
};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use crate::traits::clock::TimeProvider;
use crate::traits::fetcher::Fetcher;
use crate::traits::search::SearchProvider;
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::Semaphore;

pub use crate::research::prompts::{
    register, research_verify_mode, EXTRACT_MODE_ID, SCOPE_MODE_ID, SYNTH_MODE_ID,
};

/// Everything one research run needs, composed from the server's seams.
pub struct ResearchDeps {
    /// For scope/extract/verify/synthesis calls.
    pub model_client: Arc<dyn ModelClient>,
    /// The search backend.
    pub search: Arc<dyn SearchProvider>,
    /// The shared clock (deadline checks + stats).
    pub clock: Arc<dyn TimeProvider>,
    /// Scope mode (registered by [`register`]).
    pub scope_mode: CorrectiveMode,
    /// Extraction mode.
    pub extract_mode: CorrectiveMode,
    /// Synthesis mode.
    pub synth_mode: CorrectiveMode,
    /// The verify mode with the refute-biased template; `ensemble_k` is
    /// overridden per tier at run time.
    pub verify_mode: CorrectiveMode,
    /// Generic input bound (`INPUT_MAX_CHARS`).
    pub input_max_chars: usize,
    /// Concurrent fetch/extract/verify cap (`RESEARCH_CONCURRENCY`).
    pub concurrency: usize,
}

/// The run's enforced ceilings: the budget meter and the wall clock, with a
/// latched first-trip reason shared by every task (FR-007).
struct Ceiling<'a> {
    meter: &'a RunMeter,
    clock: &'a dyn TimeProvider,
    started_at: DateTime<Utc>,
    budget_tokens: u64,
    deadline_ms: u64,
    tripped: OnceLock<StopReason>,
}

impl Ceiling<'_> {
    fn elapsed_ms(&self) -> u64 {
        u64::try_from(
            (self.clock.now() - self.started_at)
                .num_milliseconds()
                .max(0),
        )
        .unwrap_or(u64::MAX)
    }

    /// Probe the ceilings: latches and returns the first trip; `None` while
    /// under both limits.
    fn probe(&self) -> Option<StopReason> {
        if let Some(reason) = self.tripped.get() {
            return Some(*reason);
        }
        let hit = if self.meter.total() >= self.budget_tokens {
            Some(StopReason::Budget)
        } else if self.elapsed_ms() >= self.deadline_ms {
            Some(StopReason::Deadline)
        } else {
            None
        };
        if let Some(reason) = hit {
            let _ = self.tripped.set(reason);
        }
        hit
    }

    fn current(&self) -> Option<StopReason> {
        self.tripped.get().copied()
    }
}

/// Run one research invocation. Returns the result plus (input, output)
/// token usage for the invocation record.
///
/// # Errors
///
/// `InvalidInput` before any provider call; the scope call's class if scope
/// fails; `SearchProvider`-class when every angle search fails. Individual
/// source/claim failures degrade the run instead (FR-013).
#[allow(clippy::too_many_lines)] // the five-phase spine reads best unbroken; helpers carry the logic
pub async fn run(
    deps: &ResearchDeps,
    fetcher: &dyn Fetcher,
    params: &ResearchParams,
) -> Result<(ResearchResult, u64, u64), AppError> {
    let settings = validate_params(deps, params)?;
    let meter = RunMeter::default();
    let ceiling = Ceiling {
        meter: &meter,
        clock: deps.clock.as_ref(),
        started_at: deps.clock.now(),
        budget_tokens: settings.budget_tokens,
        deadline_ms: settings.deadline_ms,
        tripped: OnceLock::new(),
    };

    let mut stats = Stats::default();

    // ---- (1) SCOPE — the only fully sequential phase -----------------------
    let plan = scope(deps, params, &settings, &meter).await?;
    stats.angles = u32::try_from(plan.angles.len()).unwrap_or(u32::MAX);

    // ---- (2) SEARCH — concurrent, then the URL-dedup barrier ---------------
    // A ceiling tripped by scope alone skips the entire fan-out.
    let mut candidates: Vec<(String, String)> = Vec::new(); // (url, title)
    if ceiling.probe().is_none() {
        let per_angle = per_angle_count(&settings, plan.angles.len());
        let searches = futures::future::join_all(
            plan.angles
                .iter()
                .map(|angle| deps.search.search(angle, per_angle)),
        )
        .await;

        let mut seen_urls = std::collections::BTreeSet::new();
        let mut search_errors: Vec<AppError> = Vec::new();
        for outcome in searches {
            match outcome {
                Ok(hits) => {
                    stats.searches += 1;
                    for hit in hits {
                        if seen_urls.insert(url_key(&hit.url)) {
                            candidates.push((hit.url, hit.title));
                        }
                    }
                }
                Err(e) => search_errors.push(e),
            }
        }
        if stats.searches == 0 {
            if let Some(first) = search_errors.into_iter().next() {
                // The whole search phase failed — the invocation fails with
                // the provider's class (edge case: provider down).
                return Err(first);
            }
        }
    }

    // Candidates found, post URL dedup (counted BEFORE the domain filter so
    // policy-excluded candidates don't silently vanish from the accounting).
    stats.sources_found = u32::try_from(candidates.len()).unwrap_or(u32::MAX);

    // Domain pre-filter (pure) — denied domains never reach the fetcher.
    candidates.retain(|(url, _)| {
        reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(String::from))
            .is_some_and(|host| {
                !settings
                    .domains_deny
                    .iter()
                    .any(|d| domain_matches(&host, d))
                    && (settings.domains_allow.is_empty()
                        || settings
                            .domains_allow
                            .iter()
                            .any(|d| domain_matches(&host, d)))
            })
    });
    candidates.truncate(settings.max_sources);

    // ---- (3) FETCH + EXTRACT — per-source pipeline, no cross-source barrier
    let semaphore = Arc::new(Semaphore::new(deps.concurrency));
    let mut sources: Vec<SourceRecord> = Vec::new();
    {
        let mut tasks = Vec::new();
        for (index, (url, _title)) in candidates.iter().enumerate() {
            if ceiling.probe().is_some() {
                break;
            }
            let id = format!("s{}", index + 1);
            tasks.push(fetch_and_extract(
                deps,
                fetcher,
                Arc::clone(&semaphore),
                &meter,
                &ceiling,
                id,
                url.clone(),
            ));
        }
        for record in futures::future::join_all(tasks).await.into_iter().flatten() {
            sources.push(record);
        }
    }
    stats.sources_fetched = u32::try_from(sources.len()).unwrap_or(u32::MAX);

    // ---- (4) VERIFY — dedup, then fan-out per unique claim -----------------
    let source_meta: BTreeMap<String, &SourceRecord> =
        sources.iter().map(|s| (s.id.clone(), s)).collect();
    let mut unique: BTreeMap<String, Claim> = BTreeMap::new();
    let mut claims_extracted = 0_u32;
    for source in &sources {
        for text in &source.claims {
            claims_extracted += 1;
            let key = claim_key(text);
            let entry = unique.entry(key).or_insert_with(|| Claim {
                text: text.clone(),
                source_ids: Vec::new(),
            });
            if !entry.source_ids.contains(&source.id) {
                entry.source_ids.push(source.id.clone());
            }
        }
    }
    stats.claims_extracted = claims_extracted;
    stats.claims_after_dedup = u32::try_from(unique.len()).unwrap_or(u32::MAX);

    let mut verify_mode = deps.verify_mode.clone();
    verify_mode.ensemble_k = settings.verify_k;
    let mut verified: Vec<VerifiedClaim> = Vec::new();
    let mut claims_dropped = 0_u32;
    {
        let mut tasks = Vec::new();
        let mut deferred = 0_u32;
        for claim in unique.into_values() {
            if ceiling.probe().is_some() {
                deferred += 1;
                continue;
            }
            tasks.push(verify_claim(
                deps,
                &verify_mode,
                Arc::clone(&semaphore),
                &meter,
                &ceiling,
                &source_meta,
                claim,
            ));
        }
        claims_dropped += deferred;
        for outcome in futures::future::join_all(tasks).await {
            match outcome {
                Some(v) => verified.push(v),
                None => claims_dropped += 1,
            }
        }
    }
    stats.claims_verified = u32::try_from(verified.len()).unwrap_or(u32::MAX);

    // ---- (5) SYNTHESIZE + grounding gate ------------------------------------
    let (refuted, surviving): (Vec<_>, Vec<_>) = verified
        .into_iter()
        .partition(|v| v.support == Support::Refuted);
    claims_dropped += u32::try_from(refuted.len()).unwrap_or(u32::MAX);
    stats.claims_dropped = claims_dropped;

    let assembled = assemble(&surviving);
    let fetched_ids: std::collections::BTreeSet<String> =
        sources.iter().map(|s| s.id.clone()).collect();

    let mut stop_reason = ceiling.current();
    let (answer, mut gaps, grounded_ids) = if surviving.is_empty() {
        // Nothing verified — deterministic honest-gap answer; no synthesis
        // call, nothing to ground (never fabricated).
        let answer = if refuted.is_empty() {
            "No verifiable findings could be established from the web for this question."
                .to_string()
        } else {
            "Verification refuted the available claims for this question; nothing is asserted."
                .to_string()
        };
        (answer, plan.sub_questions.clone(), Vec::new())
    } else {
        synthesize_grounded(
            deps.model_client.as_ref(),
            &deps.synth_mode,
            params,
            &plan,
            &surviving,
            &refuted,
            &source_meta,
            &fetched_ids,
            &meter,
            &mut stop_reason,
        )
        .await?
    };

    // Confidence: coverage-weighted (FR-005).
    let finding_confidences: Vec<f32> = assembled.findings.iter().map(|f| f.confidence).collect();
    let settled = plan.sub_questions.len().saturating_sub(gaps.len());
    let confidence =
        verdict::overall_confidence(&finding_confidences, settled, plan.sub_questions.len());

    // Sources: only what the grounding kept (uncited pruned).
    let source_refs: Vec<SourceRef> = grounded_ids
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

    gaps.truncate(crate::research::MAX_GAPS);
    stats.tokens = meter.total();
    stats.elapsed_ms = ceiling.elapsed_ms();
    stats.stopped_early = stop_reason.is_some();
    stats.stop_reason = stop_reason;

    Ok((
        ResearchResult {
            answer,
            confidence,
            key_findings: assembled.findings,
            disagreements: assembled.disagreements,
            gaps,
            sources: source_refs,
            stats,
        },
        meter.input_tokens(),
        meter.output_tokens(),
    ))
}

async fn scope(
    deps: &ResearchDeps,
    params: &ResearchParams,
    settings: &RunSettings,
    meter: &RunMeter,
) -> Result<ScopePlan, AppError> {
    let focus_clause = params.focus.as_ref().map_or(String::new(), |focus| {
        format!(
            " Bias the angles toward these caller-named facets: {}.",
            focus.join("; ")
        )
    });
    let prompt = deps
        .scope_mode
        .prompt_template
        .replace("<<angles_max>>", &settings.angles.to_string())
        .replace("<<focus_clause>>", &focus_clause)
        .replace("<<question>>", params.question.trim());

    let completion = deps
        .model_client
        .complete(&prompt, &deps.scope_mode.sanitized_schema)
        .await?;
    meter.add(completion.input_tokens, completion.output_tokens);
    validate(&deps.scope_mode.output_schema, &completion.value)?;
    let out: ScopeOut = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("scope shape: {e}")))?;

    let mut angles: Vec<String> = out
        .angles
        .into_iter()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .take(usize::from(settings.angles))
        .collect();
    if angles.is_empty() {
        // A scope that produced nothing still leaves one honest angle: the
        // question itself.
        angles.push(params.question.trim().to_string());
    }
    let sub_questions = out
        .sub_questions
        .into_iter()
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty())
        .take(MAX_SUB_QUESTIONS)
        .collect();
    Ok(ScopePlan {
        angles,
        sub_questions,
    })
}

/// One source's fetch → readable-text → claim-extraction pipeline. `None`
/// drops the source (counted by the caller's arithmetic; FR-013). The
/// ceiling is re-probed after the permit — a budget blown mid-phase stops
/// queued sources, not just the next phase.
async fn fetch_and_extract(
    deps: &ResearchDeps,
    fetcher: &dyn Fetcher,
    semaphore: Arc<Semaphore>,
    meter: &RunMeter,
    ceiling: &Ceiling<'_>,
    id: String,
    url: String,
) -> Option<SourceRecord> {
    let Ok(_permit) = semaphore.acquire().await else {
        return None;
    };
    if ceiling.probe().is_some() {
        return None;
    }
    let page = match fetcher.fetch(&url).await {
        Ok(page) => page,
        Err(e) => {
            tracing::debug!(url, error = %e, "source dropped at fetch");
            return None;
        }
    };
    let Some(readable) = extract::readable_text(&page) else {
        tracing::debug!(
            url,
            "source dropped at readability (no extractable main text)"
        );
        return None;
    };
    let (claims, input, output) =
        match extract::extract_claims(deps.model_client.as_ref(), &deps.extract_mode, &readable)
            .await
        {
            Ok(ok) => ok,
            Err(e) => {
                tracing::debug!(url, error = %e, "source dropped at extraction");
                return None;
            }
        };
    meter.add(input, output);

    let host = reqwest::Url::parse(&page.url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default();
    Some(SourceRecord {
        id,
        url: page.url.clone(),
        title: readable.title,
        fetched_at: deps.clock.now().to_rfc3339(),
        credibility: source_credibility(&host),
        claims,
    })
}

/// Verify one claim through the refute-biased ensemble. `None` drops the
/// claim (counted; FR-013). Ceiling re-probed after the permit.
async fn verify_claim(
    deps: &ResearchDeps,
    mode: &CorrectiveMode,
    semaphore: Arc<Semaphore>,
    meter: &RunMeter,
    ceiling: &Ceiling<'_>,
    source_meta: &BTreeMap<String, &SourceRecord>,
    claim: Claim,
) -> Option<VerifiedClaim> {
    let Ok(_permit) = semaphore.acquire().await else {
        return None;
    };
    if ceiling.probe().is_some() {
        return None;
    }
    let context = claim
        .source_ids
        .iter()
        .filter_map(|id| source_meta.get(id))
        .map(|s| {
            let host = reqwest::Url::parse(&s.url)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .unwrap_or_default();
            format!("{} ({host})", s.title)
        })
        .collect::<Vec<_>>()
        .join("; ");
    let verify_params = VerifyParams {
        claim: claim.text.clone(),
        context: Some(format!("Claim extracted from: {context}")),
    };

    let run = match verify::run(
        deps.model_client.as_ref(),
        mode,
        &verify_params,
        deps.input_max_chars,
    )
    .await
    {
        Ok(run) => run,
        Err(e) => {
            tracing::debug!(claim = %claim.text, error = %e, "claim dropped at verification");
            return None;
        }
    };
    meter.add(run.input_tokens, run.output_tokens);

    let mean_credibility = {
        let credibilities: Vec<f32> = claim
            .source_ids
            .iter()
            .filter_map(|id| source_meta.get(id))
            .map(|s| s.credibility)
            .collect();
        if credibilities.is_empty() {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            {
                credibilities.iter().sum::<f32>() / (credibilities.len() as f32)
            }
        }
    };
    let support = verdict::support(
        run.verdict.passes,
        run.verdict.confidence,
        run.verdict.verdict,
        claim.source_ids.len(),
    );
    let confidence = verdict::claim_confidence(
        run.verdict.confidence,
        claim.source_ids.len(),
        mean_credibility,
    );
    Some(VerifiedClaim {
        claim,
        support,
        confidence,
        findings: run.verdict.findings,
    })
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
