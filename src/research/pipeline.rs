//! The five-phase research pipeline (RESEARCH_PRIMITIVE.md §2; FR-002).
//!
//! Scope (1 call) → angle searches (concurrent, URL-dedup barrier) →
//! fetch+extract (per-source pipeline, no cross-source barrier) → verify
//! (fan-out per deduped claim, refute-biased ensemble) → synthesize
//! ([`crate::research::synthesis`]: the model writes prose, gaps, and the
//! sub-question each gap concerns) → the
//! grounding gate (one retry, then demotion).
//!
//! Budget/deadline are enforced ceilings, probed before *and inside* every
//! unit of work: between phases, before each spawn, and again after each
//! task acquires its concurrency permit — a mid-phase budget blowout stops
//! the remaining tasks, not just the next phase (FR-007).

use crate::error::AppError;
use crate::modes::verify::{self, VerifyParams};
use crate::modes::CorrectiveMode;
use crate::research::contract::{ResearchParams, ResearchResult, Stats, StopReason};
use crate::research::evidence;
use crate::research::extract;
use crate::research::outcome;
use crate::research::prompts::ScopeOut;
use crate::research::settings::{per_angle_count, validate_params, RunSettings};
use crate::research::synthesis::{assemble, synthesize_grounded, Synthesized};
use crate::research::verdict::{self, source_credibility};
use crate::research::{
    claim_key, domain_matches, url_key, Claim, Depth, RunMeter, ScopePlan, SourceRecord, Support,
    VerifiedClaim, MAX_SUB_QUESTIONS,
};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use crate::traits::clock::TimeProvider;
use crate::traits::search::{ContentsRequest, Page, SearchProvider, SearchRequest};
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
    /// Client for the scope call (018: `research_scope`).
    pub scope_client: Arc<dyn ModelClient>,
    /// Client for the per-source extraction call (018: `research_extract` —
    /// the sole `bulk`-tier call site, and the one whose volume scales with
    /// the number of fetched sources).
    pub extract_client: Arc<dyn ModelClient>,
    /// Client for the per-claim verification calls (018: `research_verify`).
    pub verify_client: Arc<dyn ModelClient>,
    /// Client for the synthesis call (018: `research_synthesize`).
    pub synth_client: Arc<dyn ModelClient>,
    /// The search backend.
    pub search: Arc<dyn SearchProvider>,
    /// Scores fetched pages against the question (061), present with
    /// `VOYAGE_API_KEY`; `None` leaves research as before.
    pub reranker: Option<Arc<dyn crate::traits::rerank::Reranker>>,
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
    /// Concurrent extract/verify cap (`RESEARCH_CONCURRENCY`).
    pub concurrency: usize,
    /// Freshness limit for source text in hours (`EXA_MAX_AGE_HOURS`, 059).
    pub max_age_hours: i32,
    /// 018: which model each research call site resolved to, so per-model
    /// token usage can be attributed without asking a client what it is
    /// (the `ModelClient` trait deliberately does not say — research D2).
    pub routing: crate::routing::RoutingTable,
    /// The selected model backend (062): research's default deadlines are
    /// longer on `claude_cli`.
    pub backend: crate::config::Backend,
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

    /// Time left before the deadline (059 review finding 1): bounds every
    /// Exa call, so a call in progress cannot carry the run past it.
    fn time_left(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.deadline_ms.saturating_sub(self.elapsed_ms()))
    }

    fn current(&self) -> Option<StopReason> {
        self.tripped.get().copied()
    }
}

/// What one source produced (023).
///
/// The distinction that matters is between *nothing to say* and *a call that
/// failed*. Both used to collapse to `None`, so a configuration error that
/// broke every extraction call in a run was indistinguishable from a set of
/// pages that genuinely held no claims — and the run reported success with an
/// empty answer.
enum SourceOutcome {
    /// Claims were extracted.
    Extracted(Box<SourceRecord>),
    /// Nothing was produced and nothing failed: an unreadable page, or a
    /// ceiling that stopped work before this source. Legitimate degradation.
    Empty,
    /// A call failed. Carried so that *every* source failing can surface as a
    /// phase failure rather than as an honest-looking empty answer.
    Failed(Box<AppError>),
}

/// What one claim's verification produced (023) — the same distinction
/// [`SourceOutcome`] draws, in the phase that dominates a run's cost.
enum ClaimOutcome {
    /// The claim was verified, whatever the verdict.
    Verified(Box<VerifiedClaim>),
    /// Skipped without failing: a ceiling stopped work before this claim.
    Empty,
    /// A verification call failed.
    Failed(Box<AppError>),
}

/// Run one research invocation. Returns the result plus (input, output)
/// token usage for the invocation record.
///
/// # Errors
///
/// `InvalidInput` before any provider call; the scope call's class if scope
/// fails; `SearchProvider`-class when every angle search fails. Individual
/// source/claim failures degrade the run instead (FR-013).
pub async fn run(
    deps: &ResearchDeps,
    params: &ResearchParams,
) -> Result<(ResearchResult, crate::telemetry::ModelUsage), AppError> {
    run_at(
        deps,
        params,
        deps.concurrency,
        &crate::telemetry::SearchCost::default(),
        &crate::telemetry::PageCounts::default(),
        &crate::telemetry::NonModelUsage::default(),
    )
    .await
}

/// Run at a caller-reduced concurrency (028 FR-015).
///
/// `concurrency` is the **effective** value: the server clamps a caller's
/// request to the configured ceiling before it gets here, so this is only ever
/// at or below `deps.concurrency`.
///
/// # Errors
///
/// Returns [`AppError`] for the same classes [`run`] does, metered with
/// whatever the run had already spent.
pub async fn run_at(
    deps: &ResearchDeps,
    params: &ResearchParams,
    concurrency: usize,
    search_cost: &crate::telemetry::SearchCost,
    pages: &crate::telemetry::PageCounts,
    non_model: &crate::telemetry::NonModelUsage,
) -> Result<(ResearchResult, crate::telemetry::ModelUsage), AppError> {
    let meter = RunMeter::default();
    let outcome = run_metered_at(
        deps,
        params,
        &meter,
        concurrency,
        search_cost,
        pages,
        non_model,
    )
    .await;
    // 020: the pipeline can fail after most of its spend — synthesis is the
    // last phase and propagates. Without this the record would show only the
    // failing call's own tokens: a plausible few thousand in place of the
    // couple of hundred thousand the run actually cost, which reads as a real
    // number and is worse than the zero it used to show.
    //
    // Totals rather than the meter's per-model breakdown, because
    // `AppError::metered` carries raw tokens: with routing configured this can
    // price bulk-tier extraction at the judgment rate, over-estimating a rare
    // error row. Losing 99% of the tokens is the larger error.
    outcome.map_err(|error| error.metered(meter.input_tokens(), meter.output_tokens()))
}

#[allow(clippy::too_many_lines)] // the five-phase spine reads best unbroken; helpers carry the logic
async fn run_metered_at(
    deps: &ResearchDeps,
    params: &ResearchParams,
    meter: &RunMeter,
    concurrency: usize,
    search_cost: &crate::telemetry::SearchCost,
    pages_counted: &crate::telemetry::PageCounts,
    non_model: &crate::telemetry::NonModelUsage,
) -> Result<(ResearchResult, crate::telemetry::ModelUsage), AppError> {
    let settings = validate_params(deps, params)?;
    let ceiling = Ceiling {
        meter,
        clock: deps.clock.as_ref(),
        started_at: deps.clock.now(),
        budget_tokens: settings.budget_tokens,
        deadline_ms: settings.deadline_ms,
        tripped: OnceLock::new(),
    };

    let mut stats = Stats::default();

    // ---- (1) SCOPE — the only fully sequential phase -----------------------
    let plan = scope(deps, params, &settings, meter).await?;
    stats.angles = u32::try_from(plan.angles.len()).unwrap_or(u32::MAX);

    // ---- (2) SEARCH — concurrent, then the URL-dedup barrier ---------------
    // A ceiling tripped by scope alone skips the entire fan-out.
    let mut candidates: Vec<(String, String)> = Vec::new(); // (url, title)
    if ceiling.probe().is_none() {
        let per_angle = per_angle_count(&settings, plan.angles.len());
        let requests: Vec<SearchRequest> = plan
            .angles
            .iter()
            .map(|angle| SearchRequest {
                query: angle.clone(),
                count: per_angle,
                search_type: params.depth.unwrap_or(Depth::Standard).search_type(),
                include_domains: settings.domains_allow.clone(),
                exclude_domains: settings.domains_deny.clone(),
                time_left: Some(ceiling.time_left()),
            })
            .collect();
        for _ in &requests {
            search_cost.begin();
        }
        let searches =
            futures::future::join_all(requests.iter().map(|r| deps.search.search(r))).await;

        let mut seen_urls = std::collections::BTreeSet::new();
        let mut search_errors: Vec<AppError> = Vec::new();
        for outcome in searches {
            match outcome {
                Ok(results) => {
                    stats.searches += 1;
                    search_cost.end(results.cost_usd);
                    for hit in results.hits {
                        if seen_urls.insert(url_key(&hit.url)) {
                            candidates.push((hit.url, hit.title));
                        }
                    }
                }
                Err(e) => {
                    // A failed call may still have been billed (finding 4),
                    // unless it sent nothing (second review L3).
                    search_cost.end(unbilled(&e));
                    search_errors.push(e);
                }
            }
        }
        // Searches cut off because the deadline arrived stop the run early,
        // as the deadline always does (finding 1) — but only a timeout does:
        // a rejection or provider failure still fails the run (L1).
        let deadline_cut = search_errors.iter().all(is_timeout) && ceiling.probe().is_some();
        if stats.searches == 0 && !deadline_cut {
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

    // Domain pre-filter (pure) — denied domains never reach the contents call.
    candidates.retain(|(url, _)| domain_allowed(url, &settings));
    candidates.truncate(settings.max_sources);

    // ---- (3) CONTENTS + EXTRACT — one contents call for the kept sources
    // (059 D1: text is billed only for sources used), then per-source
    // extraction with no cross-source barrier.
    let mut pages: Vec<Option<Page>> = vec![None; candidates.len()];
    if !candidates.is_empty() && ceiling.probe().is_none() {
        let request = ContentsRequest {
            urls: candidates.iter().map(|(url, _)| url.clone()).collect(),
            max_age_hours: deps.max_age_hours,
            max_characters: extract::EXTRACT_TEXT_MAX_CHARS,
            time_left: Some(ceiling.time_left()),
        };
        search_cost.begin();
        match deps.search.contents(&request).await {
            Ok(contents) => {
                search_cost.end(contents.cost_usd);
                pages = pair_pages(&request.urls, contents.pages);
            }
            Err(error) => {
                search_cost.end(unbilled(&error));
                // 059 B1: this one request carries every source's text, so
                // its failure is the phase's failure (023); `run_at` meters
                // the spend so far onto the error (020). Cut off by the
                // deadline instead — a timeout once the deadline has passed —
                // the run stops early (finding 1, second review L1).
                if !(is_timeout(&error) && ceiling.probe().is_some()) {
                    return Err(error);
                }
            }
        }
    }
    // ---- (3b) RERANK (061) — only the top pages reach extraction ---------
    // Finding 2 (059): the page may come back under another address (a
    // redirect); the caller's domain lists apply to that one too, as they did
    // to every redirect hop of the old fetcher. A denied page is dropped as
    // filtered, never extracted or cited — and, since 061 review finding 1,
    // before the rerank, so it takes no top-K slot and its text is not sent.
    let mut denied = vec![false; pages.len()];
    for (index, ((url, _title), page)) in candidates.iter().zip(pages.iter_mut()).enumerate() {
        if let Some(moved) = page.as_ref().filter(|p| !domain_allowed(&p.url, &settings)) {
            tracing::debug!(
                source = url.as_str(),
                returned = moved.url.as_str(),
                "source dropped: its page came back under a denied domain"
            );
            denied[index] = true;
            *page = None;
        }
    }
    // Review finding 2: pages that came back with text (denied ones are
    // already gone); `kept` below counts only those sent to extraction.
    let pages_fetched =
        u32::try_from(pages.iter().filter(|p| has_text(p.as_ref())).count()).unwrap_or(u32::MAX);
    let mut pages_kept: u32 = 0;
    let (kept, relevance, skipped) = rerank_pages(
        deps,
        &params.question,
        &pages,
        settings.keep_sources,
        ceiling.time_left(),
        meter,
        non_model,
    )
    .await;
    stats.rerank_skipped = skipped;
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut sources: Vec<SourceRecord> = Vec::new();
    {
        let mut tasks = Vec::new();
        let mut task_relevance: Vec<Option<f32>> = Vec::new();
        for (index, ((url, _title), page)) in candidates.iter().zip(pages).enumerate() {
            if ceiling.probe().is_some() {
                break;
            }
            if denied[index] || !kept[index] {
                continue;
            }
            if has_text(page.as_ref()) {
                pages_kept += 1;
            }
            let id = format!("s{}", index + 1);
            task_relevance.push(relevance[index]);
            tasks.push(extract_source(
                deps,
                Arc::clone(&semaphore),
                meter,
                &ceiling,
                id,
                url.clone(),
                page,
            ));
        }
        stats.sources_kept = pages_kept;
        pages_counted.set(pages_fetched, pages_kept);
        let mut failures: Vec<AppError> = Vec::new();
        for (outcome, score) in futures::future::join_all(tasks)
            .await
            .into_iter()
            .zip(task_relevance)
        {
            match outcome {
                SourceOutcome::Extracted(mut record) => {
                    record.relevance = score;
                    sources.push(*record);
                }
                SourceOutcome::Empty => {}
                SourceOutcome::Failed(error) => failures.push(*error),
            }
        }
        // 023: every source failing is a phase failure, not degradation. The
        // search phase already draws this line — it propagates when no search
        // succeeded — and extraction did not, so a misconfiguration that broke
        // every extraction call returned a well-formed empty answer reporting
        // success. `sources_found: 10, sources_fetched: 0` was a systematic
        // failure the run had the information to detect and did not.
        //
        // Degradation is preserved: one surviving source is enough to continue
        // (FR-013), and a page that simply held no readable text is `Empty`,
        // not a failure — a run where every candidate is unreadable genuinely
        // has no findings, and saying so is truthful.
        //
        // Unmetered: each failed extraction already added its tokens to
        // `meter` above, and `run_at` attaches the cumulative meter on the way
        // out. Summing them into the error here as well would bill every
        // dropped source twice, because `AppError::metered` adds.
        if sources.is_empty() && !failures.is_empty() {
            return Err(verify::dominant_failure_unmetered(failures));
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
                meter,
                &ceiling,
                &source_meta,
                claim,
            ));
        }
        claims_dropped += deferred;
        let mut failures: Vec<AppError> = Vec::new();
        let mut attempted = 0_u32;
        for outcome in futures::future::join_all(tasks).await {
            attempted += 1;
            match outcome {
                ClaimOutcome::Verified(v) => verified.push(*v),
                ClaimOutcome::Empty => claims_dropped += 1,
                ClaimOutcome::Failed(error) => {
                    claims_dropped += 1;
                    failures.push(*error);
                }
            }
        }
        // 023, same rule as extraction: every attempted claim failing is a
        // phase failure. Verification is the run's dominant cost, so a
        // systematic failure here is both the most expensive to hide and the
        // most misleading — the answer would assert nothing while reporting
        // success, which reads as "the evidence did not support anything"
        // rather than "nothing was checked".
        //
        // Unmetered for the same reason as extraction: the per-claim `Err` arm
        // meters the failed call, and `run_at` adds the cumulative total.
        if attempted > 0 && verified.is_empty() && !failures.is_empty() {
            return Err(verify::dominant_failure_unmetered(failures));
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
    let synthesized = if surviving.is_empty() {
        // Nothing verified — deterministic honest-gap answer; no synthesis
        // call, nothing to ground (never fabricated).
        let answer = if refuted.is_empty() {
            "No verifiable findings could be established from the web for this question."
                .to_string()
        } else {
            "The sources did not establish any of the claims checked for this question; nothing \
             is asserted."
                .to_string()
        };
        // Every sub-question is listed as a gap and keyed to itself: nothing
        // was verified, so nothing was settled (021).
        let gap_targets = (1..=plan.sub_questions.len())
            .map(|i| u32::try_from(i).unwrap_or(u32::MAX))
            .collect();
        Synthesized {
            answer,
            gaps: plan.sub_questions.clone(),
            gap_targets,
            grounded_ids: Vec::new(),
        }
    } else {
        synthesize_grounded(
            deps.synth_client.as_ref(),
            deps.routing
                .model_for(crate::routing::CallSite::ResearchSynthesize),
            &deps.synth_mode,
            params,
            &plan,
            &surviving,
            &refuted,
            &source_meta,
            &fetched_ids,
            meter,
            &mut stop_reason,
        )
        .await?
    };

    stats.tokens = meter.total();
    stats.elapsed_ms = ceiling.elapsed_ms();
    stats.stopped_early = stop_reason.is_some();
    stats.stop_reason = stop_reason;

    // Result assembly is a pure function of finished run state and lives in
    // `outcome.rs` — extracted from this spine (code review, 021) so the
    // coverage/status agreement has a unit-testable home.
    let result = outcome::assemble_result(
        outcome::Outcome {
            plan: &plan,
            assembled,
            synthesized,
            refuted: &refuted,
            surviving: &surviving,
            source_meta: &source_meta,
        },
        stats,
    );
    Ok((result, meter.usage()))
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
        .scope_client
        .complete(&prompt, &deps.scope_mode.sanitized_schema)
        .await?;
    meter.add(
        deps.routing
            .model_for(crate::routing::CallSite::ResearchScope),
        completion.input_tokens,
        completion.output_tokens,
    );
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

/// The caller's domain lists applied to one address: not denied, and allowed
/// when an allow list is given.
/// A page that came back with non-empty text: what the reranker scores and
/// what the page counts count (061).
fn has_text(page: Option<&Page>) -> bool {
    page.is_some_and(|p| !p.text.trim().is_empty())
}

fn domain_allowed(url: &str, settings: &RunSettings) -> bool {
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
}

/// Pair the contents call's pages with the requested URLs, in request order
/// (059 review finding 3, second review M2). Three passes over all pages, so
/// the result does not depend on the order Exa returns them: every echoed
/// `id` match first; then the page's own address, for pages still unpaired;
/// then, when exactly one URL and one page are left, those two — but only if
/// that page's `id` and address match no requested URL at all, so a duplicate
/// or a page whose slot is taken is never placed under an unrelated source.
/// Anything still unpaired stays `None`.
pub(crate) fn pair_pages(requested: &[String], pages: Vec<Page>) -> Vec<Option<Page>> {
    let keys: Vec<String> = requested.iter().map(|url| url_key(url)).collect();
    let mut paired: Vec<Option<Page>> = vec![None; requested.len()];
    let mut unpaired: Vec<Page> = Vec::new();
    for page in pages {
        let key = url_key(&page.requested_url);
        match (0..keys.len()).find(|&i| paired[i].is_none() && keys[i] == key) {
            Some(i) => paired[i] = Some(page),
            None => unpaired.push(page),
        }
    }
    let mut leftovers: Vec<Page> = Vec::new();
    for page in unpaired {
        let key = url_key(&page.url);
        match (0..keys.len()).find(|&i| paired[i].is_none() && keys[i] == key) {
            Some(i) => paired[i] = Some(page),
            None => leftovers.push(page),
        }
    }
    let matches_none = |page: &Page| {
        let (id, own) = (url_key(&page.requested_url), url_key(&page.url));
        !keys.iter().any(|k| *k == id || *k == own)
    };
    let open: Vec<usize> = (0..keys.len()).filter(|&i| paired[i].is_none()).collect();
    if let ([i], [page]) = (open.as_slice(), leftovers.as_slice()) {
        if matches_none(page) {
            paired[*i] = leftovers.pop();
        }
    }
    paired
}

/// The run's deadline or a request timeout cut the call off.
fn is_timeout(error: &AppError) -> bool {
    matches!(error.root(), AppError::Timeout { .. })
}

/// The cost a failed Exa call is credited with: nothing when it sent
/// nothing (second review L3), otherwise unknown — it may have been billed.
fn unbilled(error: &AppError) -> Option<f64> {
    matches!(error.root(), AppError::Timeout { what, .. } if *what == crate::traits::search::NOT_SENT)
        .then_some(0.0)
}

/// One source's text → claim-extraction pipeline. The text came from the
/// run's contents call (059); a URL it returned no page for is a failed
/// source, and a page with no text is an empty one (FR-013). The ceiling is
/// re-probed after the permit — a budget blown mid-phase stops queued
/// sources, not just the next phase.
/// Which candidates reach extraction, each page's relevance, and why a
/// configured rerank did not run (061).
///
/// Without a reranker, or with no more scorable pages (non-empty text) than
/// the keep count, every candidate is kept and nothing is scored. Otherwise
/// the scorable pages are reranked in fetched order and the top `keep` go on;
/// pages without text are not sent and stay out, as before (D7).
///
/// A failed rerank never fails the run (FR-005, the operator-approved
/// exception): an error, a malformed reply, or the call outlasting
/// `time_left` keeps every candidate, scores nothing, and returns the reason.
async fn rerank_pages(
    deps: &ResearchDeps,
    question: &str,
    pages: &[Option<Page>],
    keep: usize,
    time_left: std::time::Duration,
    meter: &RunMeter,
    non_model: &crate::telemetry::NonModelUsage,
) -> (Vec<bool>, Vec<Option<f32>>, Option<String>) {
    // 061 D8: rerank tokens reach the run's usage under the reranker's model,
    // and the side accumulator a failed run's record reads (review finding 3).
    let bill = |tokens: u64, model: &str| {
        if tokens > 0 {
            meter.add_usage(model, tokens);
            non_model.add(model, tokens);
        }
    };
    let count = pages.len();
    let unchanged = |reason: Option<String>| (vec![true; count], vec![None; count], reason);
    let Some(reranker) = &deps.reranker else {
        return unchanged(None);
    };
    let scorable: Vec<usize> = pages
        .iter()
        .enumerate()
        .filter(|(_, p)| has_text(p.as_ref()))
        .map(|(i, _)| i)
        .collect();
    if scorable.len() <= keep {
        return unchanged(None);
    }
    let documents: Vec<String> = scorable
        .iter()
        .filter_map(|&i| {
            pages
                .get(i)
                .and_then(Option::as_ref)
                .map(|p| p.text.clone())
        })
        .collect();
    // Review finding 5: the deadline already passed during the searches —
    // don't start a paid call that could only be cut off at once.
    if time_left.is_zero() {
        tracing::warn!("rerank skipped: the run's deadline passed first");
        return unchanged(Some("timeout".to_string()));
    }
    let result = match tokio::time::timeout(time_left, reranker.rerank(question, &documents)).await
    {
        Err(_elapsed) => {
            tracing::warn!("rerank skipped: the run's deadline passed first");
            return unchanged(Some("timeout".to_string()));
        }
        Ok(Err(error)) => {
            // Review finding 4: a malformed reply was still billed.
            bill(error.billed().0, reranker.model_id());
            let message = error.to_string();
            let reason = if message.contains("out-of-contract") {
                format!("malformed_reply: {message}")
            } else {
                format!("provider_error: {message}")
            };
            tracing::warn!(%reason, "rerank skipped");
            return unchanged(Some(reason));
        }
        Ok(Ok(result)) => result,
    };
    bill(result.total_tokens, reranker.model_id());
    if result.scores.len() != documents.len() {
        let reason = format!(
            "malformed_reply: {} scores for {} pages",
            result.scores.len(),
            documents.len()
        );
        tracing::warn!(%reason, "rerank skipped");
        return unchanged(Some(reason));
    }
    let mut kept = vec![false; count];
    let mut relevance = vec![None; count];
    for (&page, &score) in scorable.iter().zip(&result.scores) {
        relevance[page] = Some(score);
    }
    for top in crate::research::rerank::keep_top(&result.scores, keep) {
        kept[scorable[top]] = true;
    }
    (kept, relevance, None)
}

async fn extract_source(
    deps: &ResearchDeps,
    semaphore: Arc<Semaphore>,
    meter: &RunMeter,
    ceiling: &Ceiling<'_>,
    id: String,
    url: String,
    page: Option<Page>,
) -> SourceOutcome {
    let Ok(_permit) = semaphore.acquire().await else {
        return SourceOutcome::Empty;
    };
    if ceiling.probe().is_some() {
        return SourceOutcome::Empty;
    }
    let Some(page) = page else {
        tracing::debug!(
            url,
            "source dropped: the contents call returned no page for it"
        );
        return SourceOutcome::Failed(Box::new(AppError::SearchProvider(format!(
            "no text returned for {url}"
        ))));
    };
    // Not a failure: the page was read and held no text. A run where every
    // candidate is like that genuinely has no findings, and saying so is the
    // truthful answer.
    let text: String = page
        .text
        .trim()
        .chars()
        .take(extract::EXTRACT_TEXT_MAX_CHARS)
        .collect();
    if text.is_empty() {
        tracing::debug!(url, "source dropped: no text");
        return SourceOutcome::Empty;
    }
    let readable = extract::ReadablePage {
        title: page.title,
        text,
    };
    let (claims, input, output) =
        match extract::extract_claims(deps.extract_client.as_ref(), &deps.extract_mode, &readable)
            .await
        {
            Ok(ok) => ok,
            Err(e) => {
                // The source is dropped, but the extraction call may still
                // have been billed — a truncation or refusal is a 200 the
                // provider charged for (020). Meter it before discarding the
                // result, or a run full of failed extractions reports a
                // fraction of what it cost.
                let (input, output) = e.billed();
                meter.add(
                    deps.routing
                        .model_for(crate::routing::CallSite::ResearchExtract),
                    input,
                    output,
                );
                tracing::debug!(url, error = %e, "source dropped at extraction");
                return SourceOutcome::Failed(Box::new(e));
            }
        };
    meter.add(
        deps.routing
            .model_for(crate::routing::CallSite::ResearchExtract),
        input,
        output,
    );

    let host = reqwest::Url::parse(&page.url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default();
    SourceOutcome::Extracted(Box::new(SourceRecord {
        id,
        url: page.url,
        title: readable.title,
        fetched_at: deps.clock.now().to_rfc3339(),
        credibility: source_credibility(&host),
        claims,
        text: readable.text,
        truncated: page.truncated,
        relevance: None,
    }))
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
) -> ClaimOutcome {
    let Ok(_permit) = semaphore.acquire().await else {
        return ClaimOutcome::Empty;
    };
    if ceiling.probe().is_some() {
        return ClaimOutcome::Empty;
    }
    // Evidence-grounded context (004 D3 amendment, 2026-07-24): each backing
    // source contributes a claim-relevant excerpt of its readable text, so
    // the refute-biased judge tests the claim against the fetched evidence
    // instead of its own (possibly stale) priors.
    let context = claim
        .source_ids
        .iter()
        .filter_map(|id| source_meta.get(id))
        .take(evidence::EVIDENCE_SOURCES_MAX)
        .map(|s| {
            let host = reqwest::Url::parse(&s.url)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .unwrap_or_default();
            format!(
                "[{}] {} ({host}):\n{}",
                s.id,
                s.title,
                evidence::evidence_excerpt(
                    &s.text,
                    &claim.text,
                    evidence::EVIDENCE_EXCERPT_MAX_CHARS
                )
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let verify_params = VerifyParams {
        claim: claim.text.clone(),
        // Internal to the research pipeline; `research` exposes no effort
        // argument (028 FR-011), so this takes the configured level.
        effort: None,
        passes: None,
        context: Some(format!(
            "Source excerpts the claim was extracted from:\n\n{context}"
        )),
    };

    let run = match verify::run(
        deps.verify_client.as_ref(),
        mode,
        &verify_params,
        deps.input_max_chars,
    )
    .await
    {
        Ok(run) => run,
        Err(e) => {
            // Same as extraction: the claim is dropped, the tokens were not
            // free (020). Verification is the tool's dominant cost, so
            // silently unmetering its failures is the largest under-report of
            // the three phases.
            let (input, output) = e.billed();
            meter.add(
                deps.routing
                    .model_for(crate::routing::CallSite::ResearchVerify),
                input,
                output,
            );
            tracing::debug!(claim = %claim.text, error = %e, "claim dropped at verification");
            return ClaimOutcome::Failed(Box::new(e));
        }
    };
    meter.add(
        deps.routing
            .model_for(crate::routing::CallSite::ResearchVerify),
        run.input_tokens,
        run.output_tokens,
    );

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
    ClaimOutcome::Verified(Box::new(VerifiedClaim {
        claim,
        support,
        confidence,
        findings: run.verdict.findings,
    }))
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
