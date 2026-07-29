//! Pipeline unit tests (T012/T015/T016) — everything through the mock seams,
//! no network, no disk.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::significant_drop_tightening, clippy::type_complexity)]

use super::*;
use crate::modes::ModeRegistry;
use crate::research::contract::Constraints;
use crate::research::Depth;
use crate::traits::client::Completion;
use crate::traits::clock::{MockTimeProvider, SystemClock};
use crate::traits::fetcher::MockFetcher;
use crate::traits::search::{MockSearchProvider, SearchHit};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// A scripted [`ModelClient`] routing on the prompt's phase marker.
struct ScriptedClient {
    prompts: std::sync::Mutex<Vec<String>>,
    scope: Value,
    on_extract: Box<dyn Fn(&str) -> Value + Send + Sync>,
    on_verify: Box<dyn Fn(&str, usize) -> Value + Send + Sync>,
    on_synth: Box<dyn Fn(usize) -> Value + Send + Sync>,
    usage: (u64, u64),
}

impl ScriptedClient {
    fn count_containing(&self, marker: &str) -> usize {
        self.prompts
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.contains(marker))
            .count()
    }
}

#[async_trait::async_trait]
impl crate::traits::client::ModelClient for ScriptedClient {
    async fn complete(&self, prompt: &str, _schema: &Value) -> Result<Completion, AppError> {
        let value = if prompt.contains("scoping a web research run") {
            self.prompts.lock().unwrap().push(prompt.to_string());
            self.scope.clone()
        } else if prompt.contains("extract falsifiable claims") {
            self.prompts.lock().unwrap().push(prompt.to_string());
            (self.on_extract)(prompt)
        } else if prompt.contains("adversarial fact-checker") {
            let nth = {
                let mut prompts = self.prompts.lock().unwrap();
                let nth = prompts.iter().filter(|p| p.as_str() == prompt).count();
                prompts.push(prompt.to_string());
                nth
            };
            (self.on_verify)(prompt, nth)
        } else if prompt.contains("executive synthesis") {
            let nth = {
                let mut prompts = self.prompts.lock().unwrap();
                let nth = prompts
                    .iter()
                    .filter(|p| p.contains("executive synthesis"))
                    .count();
                prompts.push(prompt.to_string());
                nth
            };
            (self.on_synth)(nth)
        } else {
            panic!("unroutable prompt: {prompt}")
        };
        Ok(Completion {
            value,
            input_tokens: self.usage.0,
            output_tokens: self.usage.1,
        })
    }
}

fn scripted(
    scope: Value,
    on_extract: impl Fn(&str) -> Value + Send + Sync + 'static,
    on_verify: impl Fn(&str, usize) -> Value + Send + Sync + 'static,
    on_synth: impl Fn(usize) -> Value + Send + Sync + 'static,
) -> Arc<ScriptedClient> {
    Arc::new(ScriptedClient {
        prompts: std::sync::Mutex::new(Vec::new()),
        scope,
        on_extract: Box::new(on_extract),
        on_verify: Box::new(on_verify),
        on_synth: Box::new(on_synth),
        usage: (10, 5),
    })
}

fn supported() -> Value {
    json!({ "verdict": "supported", "findings": [] })
}

fn refuted(reason: &str) -> Value {
    json!({ "verdict": "refuted", "findings": [reason] })
}

fn deps_with(
    client: Arc<ScriptedClient>,
    search: MockSearchProvider,
    clock: Arc<dyn TimeProvider>,
) -> ResearchDeps {
    deps_with_client(client, search, clock)
}

/// [`deps_with`] for a client that is not the shared `ScriptedClient` — the
/// billed-failure fixtures need to return `Err`, which the scripted one cannot.
fn deps_with_client(
    client: Arc<dyn ModelClient>,
    search: MockSearchProvider,
    clock: Arc<dyn TimeProvider>,
) -> ResearchDeps {
    let mut registry = ModeRegistry::new();
    crate::modes::verify::register(&mut registry, 3).unwrap();
    register(&mut registry).unwrap();
    let verify_mode = research_verify_mode(registry.get(crate::modes::verify::VERIFY_ID).unwrap());
    ResearchDeps {
        // 018: the four research call sites are routable independently. These
        // fixtures share one scripted client, so every existing assertion's
        // expected value is unchanged — the split is structural here, and the
        // per-site routing behavior is asserted in `client::pool`.
        scope_client: Arc::clone(&client),
        extract_client: Arc::clone(&client),
        verify_client: Arc::clone(&client),
        synth_client: client,
        search: Arc::new(search),
        clock,
        scope_mode: registry.get(SCOPE_MODE_ID).unwrap().clone(),
        extract_mode: registry.get(EXTRACT_MODE_ID).unwrap().clone(),
        synth_mode: registry.get(SYNTH_MODE_ID).unwrap().clone(),
        verify_mode,
        input_max_chars: 50_000,
        concurrency: 4,
        // These fixtures share one scripted client, so every call site reports
        // the same model and existing token assertions are unchanged.
        routing: crate::routing::RoutingTable::single("claude-opus-4-8"),
    }
}

fn article_html(text: &str) -> String {
    format!(
        "<html><head><title>Page</title></head><body><article><h1>Heading</h1>\
         <p>{text} This paragraph carries enough running text for the extractor \
         to classify it as main content rather than boilerplate.</p></article></body></html>"
    )
}

fn search_returning(urls: &'static [&'static str]) -> MockSearchProvider {
    let mut search = MockSearchProvider::new();
    search.expect_search().returning(move |_, _| {
        Ok(urls
            .iter()
            .map(|u| SearchHit {
                url: (*u).to_string(),
                title: format!("title of {u}"),
                snippet: String::new(),
            })
            .collect())
    });
    search
}

fn fetcher_ok() -> MockFetcher {
    let mut fetcher = MockFetcher::new();
    fetcher.expect_fetch().returning(|url| {
        Ok(crate::traits::fetcher::FetchedPage {
            url: url.to_string(),
            html: article_html(&format!("Content of {url}.")),
        })
    });
    fetcher
}

fn params(question: &str, depth: Option<Depth>) -> ResearchParams {
    ResearchParams {
        question: question.to_string(),
        depth,
        focus: None,
        constraints: None,
    }
}

fn scope_value() -> Value {
    json!({
        "angles": ["angle one", "angle two"],
        "sub_questions": ["does it hold?", "since when?"]
    })
}

// ---- T012: the five phases through mock seams -------------------------------

#[tokio::test]
async fn happy_path_citations_resolve_and_stats_account() {
    // Both sources assert the shared claim; s2 adds a solo claim.
    let client = scripted(
        scope_value(),
        |prompt| {
            if prompt.contains("example.com/a") {
                json!({ "claims": ["the shared claim holds"] })
            } else {
                json!({ "claims": ["the shared claim holds", "a solo claim"] })
            }
        },
        |_, _| supported(),
        |_| json!({ "answer": "Shared holds [s1][s2]; solo noted [s2].", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&["https://example.com/a", "https://example.com/b"]);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let (result, usage) = run(
        &deps,
        &fetcher_ok(),
        &params("does it hold?", Some(Depth::Quick)),
    )
    .await
    .unwrap();

    // Citations resolve: every finding source id and every [sN] is listed.
    let listed: Vec<&str> = result.sources.iter().map(|s| s.id.as_str()).collect();
    for finding in &result.key_findings {
        for id in &finding.sources {
            assert!(listed.contains(&id.as_str()), "finding cites unlisted {id}");
        }
    }
    assert_eq!(result.key_findings.len(), 2);
    let shared = result
        .key_findings
        .iter()
        .find(|f| f.claim.contains("shared"))
        .unwrap();
    assert_eq!(shared.support, Support::Confirmed); // n = 2 sources
    assert_eq!(shared.sources.len(), 2);
    let solo = result
        .key_findings
        .iter()
        .find(|f| f.claim.contains("solo"))
        .unwrap();
    assert_eq!(solo.support, Support::Unverified); // n = 1, never stated as fact

    // Stats account honestly.
    assert_eq!(result.stats.angles, 2);
    assert_eq!(result.stats.searches, 2);
    assert_eq!(result.stats.sources_found, 2); // dedup across angles
    assert_eq!(result.stats.sources_fetched, 2);
    assert_eq!(result.stats.claims_extracted, 3);
    assert_eq!(result.stats.claims_after_dedup, 2);
    assert_eq!(result.stats.claims_verified, 2);
    assert_eq!(result.stats.claims_dropped, 0);
    assert!(!result.stats.stopped_early);
    assert_eq!(result.stats.stop_reason, None);

    // FR-012: no page bodies on the wire.
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains("running text for the extractor"));

    // Token usage summed across every call: scope + 2 extract + 2 verify (K=1)
    // + 1 synthesis = 6 calls at (10, 5).
    // Same expected totals as before 018 — only the accessor changed. The
    // fixtures run every call site on one model, so this is also the
    // single-model identity FR-009a promises.
    assert_eq!(usage.totals(), (60, 30));
    assert_eq!(usage.models(), vec!["claude-opus-4-8".to_string()]);
    assert_eq!(result.stats.tokens, 90);
    assert!(result.confidence > 0.0);
}

// 004 D3 amendment (2026-07-24): the refute-biased judge must see the
// fetched source text, not source titles alone — a prior-only judge
// systematically refutes true post-cutoff facts.
#[tokio::test]
async fn verify_prompt_carries_source_excerpts_not_titles_alone() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["the page claim"] }),
        |prompt, _| {
            assert!(prompt.contains("Source excerpts the claim was extracted from"));
            assert!(
                prompt.contains("running text for the extractor"),
                "the judge must receive the fetched evidence, not titles alone"
            );
            assert!(prompt.contains("[s1]"));
            supported()
        },
        |_| json!({ "answer": "ok [s1]", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&["https://example.com/a"]);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();

    assert_eq!(result.stats.claims_verified, 1);
    // FR-012 still holds: evidence reaches the judge, never the wire.
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains("running text for the extractor"));
}

#[tokio::test]
async fn focus_reaches_the_scope_prompt() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&[]);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let mut p = params("q?", Some(Depth::Quick));
    p.focus = Some(vec!["the pricing facet".to_string()]);
    let fetcher = MockFetcher::new(); // zero candidates → never called
    run(&deps, &fetcher, &p).await.unwrap();

    let prompts = client.prompts.lock().unwrap();
    let scope_prompt = prompts
        .iter()
        .find(|p| p.contains("scoping a web research run"))
        .unwrap();
    assert!(scope_prompt.contains("the pricing facet"), "{scope_prompt}");
}

#[tokio::test]
async fn single_fetch_failure_degrades_and_counts_never_fails_the_run() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["good claim"] }),
        |_, _| supported(),
        |_| json!({ "answer": "Good [s2].", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&["https://bad.example/x", "https://example.com/ok"]);
    let mut fetcher = MockFetcher::new();
    fetcher.expect_fetch().returning(|url| {
        if url.contains("bad.example") {
            Err(AppError::SearchProvider("HTTP 503".into()))
        } else {
            Ok(crate::traits::fetcher::FetchedPage {
                url: url.to_string(),
                html: article_html("Reachable content."),
            })
        }
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &fetcher, &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(result.stats.sources_found, 2);
    assert_eq!(result.stats.sources_fetched, 1);
    assert_eq!(result.key_findings.len(), 1);
}

#[tokio::test]
async fn refuted_claims_are_dropped_and_contested_claims_surface() {
    // Standard depth → K=2. "contested" splits 1–1; "wrong" refutes 2–0;
    // "right" supports 2–0.
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["the wrong claim", "the contested claim", "the right claim"] }),
        |prompt, nth| {
            if prompt.contains("the wrong claim") {
                refuted("it is false because X")
            } else if prompt.contains("the contested claim") {
                if nth == 0 {
                    supported()
                } else {
                    refuted("half the panel disagrees")
                }
            } else {
                supported()
            }
        },
        |_| json!({ "answer": "Right [s1]; contested noted [s1].", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&["https://example.com/one"]);
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Standard)))
        .await
        .unwrap();

    // The refuted claim is absent from the body and counted.
    assert!(!result
        .key_findings
        .iter()
        .any(|f| f.claim.contains("wrong")));
    assert!(!result.answer.contains("wrong claim"));
    assert_eq!(result.stats.claims_dropped, 1);

    // The contested claim surfaces in disagreements with ≥ 2 positions.
    let contested = result
        .key_findings
        .iter()
        .find(|f| f.claim.contains("contested"))
        .unwrap();
    assert_eq!(contested.support, Support::Contested);
    assert_eq!(result.disagreements.len(), 1);
    assert!(result.disagreements[0].positions.len() >= 2);
    assert!(result.disagreements[0].positions[1]
        .stance
        .contains("half the panel disagrees"));
}

#[tokio::test]
async fn grounding_violation_retries_once_with_the_violation_named() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["a claim"] }),
        |_, _| supported(),
        |attempt| {
            if attempt == 0 {
                json!({ "answer": "Fabricated [s99].", "gaps": [], "gap_targets": [] })
            } else {
                json!({ "answer": "Grounded [s1].", "gaps": [], "gap_targets": [] })
            }
        },
    );
    let search = search_returning(&["https://example.com/one"]);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(result.answer, "Grounded [s1].");
    assert!(!result.stats.stopped_early);
    assert_eq!(client.count_containing("executive synthesis"), 2);
    // The retry prompt named the violation.
    let prompts = client.prompts.lock().unwrap();
    let retry = prompts
        .iter()
        .filter(|p| p.contains("executive synthesis"))
        .nth(1)
        .unwrap();
    assert!(retry.contains("[s99]"), "retry must name the violation");
}

#[tokio::test]
async fn second_grounding_failure_demotes_instead_of_emitting_ungrounded_content() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["a claim"] }),
        |_, _| supported(),
        |_| json!({ "answer": "Always fabricated [s99].", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&["https://example.com/one"]);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert!(!result.answer.contains("[s99]"), "{}", result.answer);
    assert!(result.answer.contains("could not be grounded"));
    assert!(result.stats.stopped_early);
    assert_eq!(result.stats.stop_reason, Some(StopReason::Grounding));
    assert!(result.gaps.iter().any(|g| g.contains("demoted")));
    // The verified finding itself survives — it was server-assembled.
    assert_eq!(result.key_findings.len(), 1);
}

#[tokio::test]
async fn no_verified_findings_yields_a_deterministic_honest_gap_answer() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| panic!("synthesis must not be called with nothing to ground"),
    );
    let search = search_returning(&[]);
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let fetcher = MockFetcher::new();

    let (result, _usage) = run(&deps, &fetcher, &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert!(result.answer.contains("No verifiable findings"));
    assert!(result.key_findings.is_empty());
    assert!(result.sources.is_empty());
    assert_eq!(result.gaps.len(), 2); // the scoped sub-questions
    assert!((result.confidence - 0.0).abs() < f32::EPSILON);
}

// ---- T012: input validation before any provider call ------------------------

#[tokio::test]
async fn invalid_inputs_are_rejected_before_any_provider_call() {
    let client = scripted(
        scope_value(),
        |_| panic!("no call expected"),
        |_, _| panic!("no call expected"),
        |_| panic!("no call expected"),
    );
    let mut search = MockSearchProvider::new();
    search.expect_search().times(0);
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let fetcher = MockFetcher::new();

    for (build, marker) in [
        (params("   ", None), "empty"),
        (
            ResearchParams {
                question: "x".repeat(50_001),
                depth: None,
                focus: None,
                constraints: None,
            },
            "INPUT_MAX_CHARS",
        ),
        (
            ResearchParams {
                constraints: Some(Constraints {
                    max_sources: Some(0),
                    ..Constraints::default()
                }),
                ..params("q?", None)
            },
            "max_sources",
        ),
        (
            ResearchParams {
                constraints: Some(Constraints {
                    budget_tokens: Some(10),
                    ..Constraints::default()
                }),
                ..params("q?", None)
            },
            "budget_tokens",
        ),
        (
            ResearchParams {
                constraints: Some(Constraints {
                    deadline_ms: Some(10),
                    ..Constraints::default()
                }),
                ..params("q?", None)
            },
            "deadline_ms",
        ),
        (
            ResearchParams {
                focus: Some(vec![String::new()]),
                ..params("q?", None)
            },
            "focus",
        ),
    ] {
        let err = run(&deps, &fetcher, &build).await.unwrap_err();
        assert!(matches!(err, AppError::InvalidInput(_)), "{marker}: {err}");
        assert!(err.to_string().contains(marker), "{marker}: {err}");
    }
}

// ---- T015: tier scaling + constraint overrides -------------------------------

#[tokio::test]
async fn depth_scales_the_scope_and_constraints_override_the_tier() {
    for (depth, angles_max) in [(Depth::Quick, "3"), (Depth::Deep, "8")] {
        let client = scripted(
            scope_value(),
            |_| json!({ "claims": [] }),
            |_, _| supported(),
            |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
        );
        let search = search_returning(&[]);
        let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));
        run(&deps, &MockFetcher::new(), &params("q?", Some(depth)))
            .await
            .unwrap();
        let prompts = client.prompts.lock().unwrap();
        let scope_prompt = prompts
            .iter()
            .find(|p| p.contains("scoping a web research run"))
            .unwrap();
        assert!(
            scope_prompt.contains(&format!("no more \nthan {angles_max}"))
                || scope_prompt.contains(&format!("no more than {angles_max}")),
            "{depth:?}: {scope_prompt}"
        );
    }

    // max_sources override: 3 candidates, cap 1 → exactly one fetch.
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&[
        "https://example.com/1",
        "https://example.com/2",
        "https://example.com/3",
    ]);
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let mut fetcher = MockFetcher::new();
    fetcher.expect_fetch().times(1).returning(|url| {
        Ok(crate::traits::fetcher::FetchedPage {
            url: url.to_string(),
            html: article_html("Capped run content."),
        })
    });
    let p = ResearchParams {
        constraints: Some(Constraints {
            max_sources: Some(1),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Deep))
    };
    let (result, _usage) = run(&deps, &fetcher, &p).await.unwrap();
    assert_eq!(result.stats.sources_fetched, 1);
}

#[tokio::test]
async fn denied_and_unallowed_domains_never_reach_the_fetcher() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&[
        "https://evil.example/page",
        "https://good.example/page",
        "https://other.example/page",
    ]);
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let mut fetcher = MockFetcher::new();
    // Only good.example may ever be fetched.
    fetcher
        .expect_fetch()
        .withf(|url| url.contains("good.example"))
        .times(1)
        .returning(|url| {
            Ok(crate::traits::fetcher::FetchedPage {
                url: url.to_string(),
                html: article_html("Allowed content."),
            })
        });

    let p = ResearchParams {
        constraints: Some(Constraints {
            domains_allow: Some(vec!["good.example".into(), "evil.example".into()]),
            domains_deny: Some(vec!["evil.example".into()]),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    };
    run(&deps, &fetcher, &p).await.unwrap();
}

// ---- T016: ceilings — graceful early synthesis -------------------------------

#[tokio::test]
async fn budget_ceiling_stops_new_work_and_synthesizes_early() {
    // Scope alone consumes 1200 tokens against a 1000-token budget.
    let client = Arc::new(ScriptedClient {
        prompts: std::sync::Mutex::new(Vec::new()),
        scope: scope_value(),
        on_extract: Box::new(|_| panic!("budget tripped before extraction")),
        on_verify: Box::new(|_, _| panic!("budget tripped before verification")),
        on_synth: Box::new(|_| panic!("nothing verified — deterministic answer expected")),
        usage: (700, 500),
    });
    // The ceiling is probed between scope and search: no search egress
    // happens after the budget is already blown.
    let mut search = MockSearchProvider::new();
    search.expect_search().times(0);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));
    let mut fetcher = MockFetcher::new();
    fetcher.expect_fetch().times(0);

    let p = ResearchParams {
        constraints: Some(Constraints {
            budget_tokens: Some(1_000),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    };
    let (result, _usage) = run(&deps, &fetcher, &p).await.unwrap();
    assert!(result.stats.stopped_early);
    assert_eq!(result.stats.stop_reason, Some(StopReason::Budget));
    assert!(!result.answer.is_empty(), "well-formed, not an error");
    assert_eq!(result.stats.sources_fetched, 0);
}

#[tokio::test]
async fn deadline_ceiling_stops_new_work_with_the_reason_named() {
    let started = DateTime::parse_from_rfc3339("2026-06-12T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mut clock = MockTimeProvider::new();
    let calls = std::sync::atomic::AtomicU32::new(0);
    clock.expect_now().returning(move || {
        // First call is the run start; every later check is past the deadline.
        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            started
        } else {
            started + chrono::Duration::seconds(60)
        }
    });

    let client = scripted(
        scope_value(),
        |_| panic!("deadline tripped before extraction"),
        |_, _| panic!("deadline tripped before verification"),
        |_| panic!("nothing verified — deterministic answer expected"),
    );
    let search = search_returning(&["https://example.com/a"]);
    let deps = deps_with(client, search, Arc::new(clock));
    let mut fetcher = MockFetcher::new();
    fetcher.expect_fetch().times(0);

    let p = ResearchParams {
        constraints: Some(Constraints {
            deadline_ms: Some(5_000),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    };
    let (result, _usage) = run(&deps, &fetcher, &p).await.unwrap();
    assert!(result.stats.stopped_early);
    assert_eq!(result.stats.stop_reason, Some(StopReason::Deadline));
    assert!(!result.answer.is_empty());
}

// ---- billed failures are counted once ----------------------------------------

/// Succeeds up to a named phase, then fails every call with a **billed**
/// error — the 020 shape: a 200 the provider ran the model for and charged.
///
/// `extract` carries the claims when extraction is meant to survive; `None`
/// makes extraction the failing phase.
struct BilledFailureAt {
    ok_usage: (u64, u64),
    failure_usage: (u64, u64),
    extract: Option<Value>,
}

#[async_trait::async_trait]
impl ModelClient for BilledFailureAt {
    async fn complete(&self, prompt: &str, _schema: &Value) -> Result<Completion, AppError> {
        let ok = |value| {
            Ok(Completion {
                value,
                input_tokens: self.ok_usage.0,
                output_tokens: self.ok_usage.1,
            })
        };
        if prompt.contains("scoping a web research run") {
            return ok(scope_value());
        }
        if prompt.contains("extract falsifiable claims") {
            if let Some(claims) = self.extract.clone() {
                return ok(claims);
            }
        }
        Err(
            AppError::Truncation("output budget exhausted after 3 output tokens".into())
                .metered(self.failure_usage.0, self.failure_usage.1),
        )
    }
}

/// A wholly failed extraction phase must bill each call exactly once.
///
/// Two disciplines meet on this path and each is correct alone. The per-source
/// `Err` arm adds the failed call's tokens to the run meter (020), and
/// `dominant_failure_metered` sums `billed()` across the failure set so the
/// class vote cannot decide how much spend is reported (also 020). Applied
/// together the failure tokens land in both, and `run_at` closes with
/// `error.metered(meter…)`, which **adds** rather than replaces — so every
/// failed extraction was counted twice.
///
/// Exact equality, not `> 0`: a doubled bill is greater than zero, and the
/// existing search-phase test asserts only that, on a path that never reaches
/// this aggregation.
#[tokio::test]
async fn a_wholly_failed_extraction_phase_bills_each_call_once() {
    let client = Arc::new(BilledFailureAt {
        ok_usage: (10, 5),
        failure_usage: (7, 3),
        extract: None,
    });
    let deps = deps_with_client(
        client,
        search_returning(&["https://example.com/a", "https://example.com/b"]),
        Arc::new(SystemClock),
    );

    let err = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap_err();

    assert!(matches!(err.root(), AppError::Truncation(_)), "{err}");
    // scope (10, 5) + two billed extraction failures at (7, 3) each.
    assert_eq!(
        err.billed(),
        (24, 11),
        "each billed call must appear once: scope 10/5 plus two failures at 7/3"
    );
}

/// The same, one phase later — verification is the run's dominant cost, so a
/// doubled bill here is the largest over-report of the three phases.
///
/// This path carries an extra aggregation the extraction one does not: each
/// claim's failure is already `verify::run`'s ensemble aggregate, summed across
/// its passes by `dominant_failure_metered`. That sum is correct — the passes
/// have no meter but their own errors — and it is what the pipeline meters and
/// must not sum a second time.
///
/// Runs at [`Depth::Deep`] deliberately. `verify_k` is depth-derived
/// (`pipeline.rs`), and `Quick` is 1 — which would leave the ensemble's own
/// summation untested, since one pass makes summing and not summing agree.
#[tokio::test]
async fn a_wholly_failed_verification_phase_bills_each_call_once() {
    let client = Arc::new(BilledFailureAt {
        ok_usage: (10, 5),
        failure_usage: (7, 3),
        extract: Some(json!({ "claims": ["the one claim"] })),
    });
    let deps = deps_with_client(
        client,
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );

    let err = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Deep)))
        .await
        .unwrap_err();

    assert!(matches!(err.root(), AppError::Truncation(_)), "{err}");
    // scope (10, 5) + one extraction (10, 5) + one claim whose three-pass
    // ensemble failed at (7, 3) a pass, aggregated by `verify::run` to (21, 9).
    assert_eq!(
        err.billed(),
        (41, 19),
        "each billed call must appear once: 10/5 scope, 10/5 extract, 21/9 ensemble"
    );
}

// ---- search-phase failure classes --------------------------------------------

#[tokio::test]
async fn all_angles_failing_fails_the_invocation_with_the_provider_class() {
    let client = scripted(
        scope_value(),
        |_| panic!("no extraction after total search failure"),
        |_, _| panic!("no verification"),
        |_| panic!("no synthesis"),
    );
    let mut search = MockSearchProvider::new();
    search
        .expect_search()
        .returning(|_, _| Err(AppError::SearchProvider("HTTP 503".into())));
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let err = run(
        &deps,
        &MockFetcher::new(),
        &params("q?", Some(Depth::Quick)),
    )
    .await
    .unwrap_err();
    assert!(matches!(err.root(), AppError::SearchProvider(_)), "{err}");
    // 020: the search phase failed, but scope already ran and was billed. The
    // record must carry that spend rather than reporting the run as free.
    let (input_tokens, output_tokens) = err.billed();
    assert!(
        input_tokens > 0 && output_tokens > 0,
        "scope spend lost on a search-phase failure: {input_tokens}/{output_tokens}"
    );
}

#[tokio::test]
async fn partial_angle_failure_degrades_and_counts() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["surviving claim"] }),
        |_, _| supported(),
        |_| json!({ "answer": "Survives [s1].", "gaps": [], "gap_targets": [] }),
    );
    let failed = std::sync::atomic::AtomicBool::new(false);
    let mut search = MockSearchProvider::new();
    search.expect_search().returning(move |_, _| {
        if failed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            Ok(vec![SearchHit {
                url: "https://example.com/only".into(),
                title: "only".into(),
                snippet: String::new(),
            }])
        } else {
            Err(AppError::SearchProvider("HTTP 503".into()))
        }
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(result.stats.angles, 2);
    assert_eq!(result.stats.searches, 1); // one angle lost, counted honestly
    assert_eq!(result.key_findings.len(), 1);
}

// ---- 021: confidence aggregation --------------------------------------------

/// T004 / SC-001 — the observed defect, reproduced.
///
/// Two live runs reported `confidence: 0` for factually correct answers whose
/// every claim survived refute-biased verification at ~0.78. The cause was the
/// coverage term: the synthesis wrote one gap per sub-question, `settled`
/// saturated to zero, and the product annihilated a well-supported answer.
/// A confidence of exactly 0 asserts certainty of falsehood.
#[tokio::test]
async fn every_sub_question_gapped_no_longer_annihilates_a_supported_answer() {
    let client = scripted(
        scope_value(), // two sub-questions
        |_| json!({ "claims": ["the claim holds"] }),
        |_, _| supported(),
        // One gap per sub-question — the exact shape that collapsed the score.
        |_| {
            json!({
                "answer": "The claim holds [s1].",
                "gaps": ["still unclear whether it holds", "and since when"],
                "gap_targets": [1, 2]
            })
        },
    );
    let search = search_returning(&["https://example.com/a"]);
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();

    assert!(
        result.confidence > 0.0,
        "a supported answer must not report certainty of falsehood; got {}",
        result.confidence
    );
    // Breadth is not lost, it moves to `coverage` — asserted in US2, which
    // introduces the field. This test stays inside US1 so the story's
    // checkpoint compiles and passes on its own.
}

/// T005 / SC-006 — refuted claims leave confidence alone and surface as their
/// own rate, so a run whose evidence largely fell apart is distinguishable
/// from one whose evidence held.
#[tokio::test]
async fn refutation_is_reported_as_its_own_rate_not_folded_into_confidence() {
    let synth = |_: usize| json!({ "answer": "It holds [s1].", "gaps": [], "gap_targets": [] });

    // Run A: both claims survive.
    let client_a = scripted(
        scope_value(),
        |_| json!({ "claims": ["claim one holds", "claim two holds"] }),
        |_, _| supported(),
        synth,
    );
    let deps_a = deps_with(
        client_a,
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let (a, _) = run(&deps_a, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();

    // Run B: same surviving claim, plus one the ensemble refutes.
    let client_b = scripted(
        scope_value(),
        |_| json!({ "claims": ["claim one holds", "claim two holds"] }),
        |prompt, _| {
            if prompt.contains("claim two") {
                refuted("claim two is false because X")
            } else {
                supported()
            }
        },
        synth,
    );
    let deps_b = deps_with(
        client_b,
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let (b, _) = run(&deps_b, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();

    assert!(
        (a.confidence - b.confidence).abs() < 1e-6,
        "confidence must report the support of what the answer asserts, \
         identical here: {} vs {}",
        a.confidence,
        b.confidence
    );
    assert!((a.refutation_rate - 0.0).abs() < f32::EPSILON);
    assert!(
        b.refutation_rate > 0.0,
        "a run that refuted half its claims must say so"
    );
}

/// T013 / FR-005 — the caller receives the basis for coverage, not just the
/// figure. Without the statuses, coverage is a number to be taken on trust.
#[tokio::test]
async fn sub_question_status_is_published_verbatim_and_in_scope_order() {
    let client = scripted(
        scope_value(), // ["does it hold?", "since when?"]
        |_| json!({ "claims": ["the claim holds"] }),
        |_, _| supported(),
        |_| {
            json!({
                "answer": "It holds [s1].",
                "gaps": ["the date is unclear"],
                "gap_targets": [2]
            })
        },
    );
    let deps = deps_with(
        client,
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let (result, _) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();

    assert_eq!(result.sub_question_status.len(), 2);
    assert_eq!(result.sub_question_status[0].sub_question, "does it hold?");
    assert_eq!(result.sub_question_status[1].sub_question, "since when?");
    assert!(result.sub_question_status[0].settled);
    assert!(!result.sub_question_status[1].settled, "gap targeted #2");
    assert!((result.coverage - 0.5).abs() < f32::EPSILON);
}

/// T014 / SC-002 — identical support, different breadth. Before this feature
/// the two were indistinguishable at the top level; now they differ in
/// coverage and agree in confidence, which is the separation the whole change
/// exists to make.
#[tokio::test]
async fn identical_support_with_different_breadth_differs_only_in_coverage() {
    let make = |targets: Value| {
        let client = scripted(
            scope_value(),
            |_| json!({ "claims": ["the claim holds"] }),
            |_, _| supported(),
            move |_| {
                json!({
                    "answer": "It holds [s1].",
                    "gaps": ["a", "b"],
                    "gap_targets": targets.clone()
                })
            },
        );
        deps_with(
            client,
            search_returning(&["https://example.com/a"]),
            Arc::new(SystemClock),
        )
    };

    let both = make(json!([1, 2]));
    let (all_gapped, _) = run(&both, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();
    let neither = make(json!([0, 0]));
    let (none_gapped, _) = run(&neither, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();

    assert!(
        (all_gapped.confidence - none_gapped.confidence).abs() < 1e-6,
        "support is identical, so confidence must be: {} vs {}",
        all_gapped.confidence,
        none_gapped.confidence
    );
    assert!((all_gapped.coverage - 0.0).abs() < f32::EPSILON);
    assert!((none_gapped.coverage - 1.0).abs() < f32::EPSILON);
    assert!(all_gapped.confidence > 0.0, "the defect: this used to be 0");
}

/// T015 / SC-003 + SC-005 — coverage is checkable from the output alone, and
/// the pre-change figure stays derivable, so no information the caller had
/// before is lost by splitting the field.
#[tokio::test]
async fn coverage_reconciles_with_the_published_statuses_and_preserves_the_old_figure() {
    for (targets, expected) in [
        (json!([]), 1.0_f32),
        (json!([1]), 0.5),
        (json!([1, 2]), 0.0),
        (json!([2, 2, 2]), 0.5), // FR-004: counted once
    ] {
        let gap_count = targets.as_array().map_or(0, Vec::len);
        let client = scripted(
            scope_value(),
            |_| json!({ "claims": ["the claim holds"] }),
            |_, _| supported(),
            move |_| {
                json!({
                    "answer": "It holds [s1].",
                    "gaps": vec!["g"; gap_count],
                    "gap_targets": targets.clone()
                })
            },
        );
        let deps = deps_with(
            client,
            search_returning(&["https://example.com/a"]),
            Arc::new(SystemClock),
        );
        let (result, _) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
            .await
            .unwrap();

        // SC-003: the figure equals what the published statuses say.
        #[allow(clippy::cast_precision_loss)]
        let from_statuses = (result
            .sub_question_status
            .iter()
            .filter(|s| s.settled)
            .count() as f32)
            / (result.sub_question_status.len() as f32);
        assert!(
            (result.coverage - from_statuses).abs() < 1e-6,
            "coverage must be reconcilable from the output alone"
        );
        assert!((result.coverage - expected).abs() < 1e-6, "{expected}");

        // SC-005: `confidence * coverage` reproduces the pre-change figure.
        // Asserted against the expected coverage rather than a range — both
        // factors are already clamped, so a range check here would be
        // unconditionally true and would test nothing.
        let legacy = result.confidence * result.coverage;
        assert!(
            result.confidence.mul_add(-expected, legacy).abs() < 1e-6,
            "the pre-change value must stay derivable"
        );
    }
}

/// T016 / research.md D2 — a length mismatch between the two parallel arrays
/// is a malformed response, not an invitation to guess. Accepting it and
/// treating absent targets as "concerns nothing" would inflate coverage to
/// full: the server overstating what it established, which is the exact
/// failure this feature exists to correct.
#[tokio::test]
async fn a_gap_target_arity_mismatch_is_retried_then_demoted_never_accepted() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["the claim holds"] }),
        |_, _| supported(),
        // Both attempts return two gaps but one target.
        |_| {
            json!({
                "answer": "It holds [s1].",
                "gaps": ["one", "two"],
                "gap_targets": [1]
            })
        },
    );
    let deps = deps_with(
        Arc::clone(&client),
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let (result, _) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();

    // Retried once, then demoted — never silently accepted.
    assert_eq!(client.count_containing("executive synthesis"), 2);
    // The reason names the failure that actually happened. Reporting this as
    // `Grounding` would tell the caller the answer could not be cited when the
    // grounding gate was never reached, which 004 FR-007 forbids.
    assert_eq!(
        result.stats.stop_reason,
        Some(StopReason::MalformedSynthesis)
    );
    assert!(
        !result.answer.contains("could not be grounded"),
        "the demotion text must not blame grounding: {}",
        result.answer
    );
    assert!((result.coverage - 0.0).abs() < f32::EPSILON, "not inflated");
}

/// T017 — a demoted run settled nothing, and says so. Confidence still reports
/// the support of the findings it did verify.
#[tokio::test]
async fn a_demoted_synthesis_reports_every_sub_question_unsettled() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["the claim holds"] }),
        |_, _| supported(),
        // Cites a source that was never fetched: the grounding gate rejects
        // both attempts.
        |_| json!({ "answer": "It holds [s99].", "gaps": [], "gap_targets": [] }),
    );
    let deps = deps_with(
        client,
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let (result, _) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();

    assert_eq!(result.stats.stop_reason, Some(StopReason::Grounding));
    assert!((result.coverage - 0.0).abs() < f32::EPSILON);
    assert!(result.sub_question_status.iter().all(|s| !s.settled));
    assert!(result.confidence > 0.0, "the findings were still verified");
}

/// T018 — an early stop is when breadth of resolution matters most to the
/// caller, so it is the worst case to leave unpinned. A run cut short before
/// it verified anything must still report a complete status list and a defined
/// coverage, rather than an empty list a caller cannot interpret.
#[tokio::test]
async fn a_run_stopped_early_still_reports_defined_coverage_and_statuses() {
    let started = DateTime::parse_from_rfc3339("2026-07-25T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mut clock = MockTimeProvider::new();
    let calls = std::sync::atomic::AtomicU32::new(0);
    clock.expect_now().returning(move || {
        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            started
        } else {
            started + chrono::Duration::seconds(60)
        }
    });

    let client = scripted(
        scope_value(),
        |_| panic!("deadline tripped before extraction"),
        |_, _| panic!("deadline tripped before verification"),
        |_| panic!("nothing verified — the deterministic answer path applies"),
    );
    let deps = deps_with(
        client,
        search_returning(&["https://example.com/a"]),
        Arc::new(clock),
    );
    let mut fetcher = MockFetcher::new();
    fetcher.expect_fetch().times(0);

    let p = ResearchParams {
        constraints: Some(Constraints {
            deadline_ms: Some(5_000),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    };
    let (result, _) = run(&deps, &fetcher, &p).await.unwrap();

    assert!(result.stats.stopped_early);
    assert_eq!(result.stats.stop_reason, Some(StopReason::Deadline));
    // Nothing was settled, and the caller can see exactly which questions
    // those were — not an absent field it has to guess about.
    assert_eq!(result.sub_question_status.len(), 2);
    assert!(result.sub_question_status.iter().all(|s| !s.settled));
    assert!((result.coverage - 0.0).abs() < f32::EPSILON);
}

/// T026 — the figure stays auditable at the gap cap.
///
/// The original defect was that the confidence penalty came from the
/// *untruncated* gap list while the caller saw the truncated one, so the
/// number could not be checked against the run's own output. It is fixed by
/// publishing the statuses rather than by reordering two statements: coverage
/// is derived from every target the synthesis returned, `gaps` is capped
/// independently, and the two published values agree with each other whatever
/// the cap does to the explanatory text.
///
/// Truncation is in fact unreachable today — the synthesis schema bounds gaps
/// at `MAX_GAPS` (10) and the in-code gap paths build at most
/// `MAX_SUB_QUESTIONS + 1` (8) — so this pins the invariant at the largest
/// reachable gap count instead of a state the validator forbids.
#[tokio::test]
async fn nine_gaps_on_one_sub_question_leave_the_other_settled() {
    let client = scripted(
        scope_value(), // two sub-questions
        |_| json!({ "claims": ["the claim holds"] }),
        |_, _| supported(),
        // Ten gaps — the schema maximum — nine of them piled on sub-question
        // 2, which under the old arithmetic would have driven `settled` to
        // zero and annihilated a well-supported answer.
        |_| {
            json!({
                "answer": "It holds [s1].",
                "gaps": ["g1", "g2", "g3", "g4", "g5", "g6", "g7", "g8", "g9", "g10"],
                "gap_targets": [2, 2, 2, 2, 2, 2, 2, 2, 2, 0]
            })
        },
    );
    let deps = deps_with(
        client,
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let (result, _) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();

    // `gaps.truncate(MAX_GAPS)` is unreachable here — the synthesis schema
    // already bounds the list at 10 — so asserting the length would be
    // unconditionally true and would test nothing.

    // FR-004: nine gaps on one sub-question leave it unsettled once, so the
    // other sub-question is still settled.
    assert!((result.coverage - 0.5).abs() < f32::EPSILON);

    // SC-003: the figure equals what the published statuses say — checkable
    // from this output alone, with no appeal to the untruncated list.
    #[allow(clippy::cast_precision_loss)]
    let from_statuses = (result
        .sub_question_status
        .iter()
        .filter(|s| s.settled)
        .count() as f32)
        / (result.sub_question_status.len() as f32);
    assert!((result.coverage - from_statuses).abs() < 1e-6);

    // And the answer is not annihilated by a long gap list.
    assert!(result.confidence > 0.0);
}

// ---- 023: a phase that wholly fails must say so ----------------------------

/// The production failure, reproduced.
///
/// `PARALLAX_EFFORT_BULK=low` was set while the bulk tier routed to a model
/// that does not support the `effort` parameter, so every extraction call
/// returned a 400. The run reported `outcome: success` with
/// `sources_found: 10, sources_fetched: 0`, an empty answer, `confidence: 0`,
/// and six plausible-looking gaps — indistinguishable from "the web does not
/// know", which was false.
#[tokio::test]
async fn every_extraction_failing_is_an_error_not_an_empty_answer() {
    // A client whose extraction calls all fail, as a rejected request would.
    struct FailingExtract(Arc<ScriptedClient>);
    #[async_trait::async_trait]
    impl crate::traits::client::ModelClient for FailingExtract {
        async fn complete(&self, prompt: &str, schema: &Value) -> Result<Completion, AppError> {
            if prompt.contains("extract falsifiable claims") {
                return Err(AppError::Client(
                    "HTTP 400: effort is not supported for this model".to_string(),
                ));
            }
            self.0.complete(prompt, schema).await
        }
    }

    let client = scripted(
        scope_value(),
        |_| panic!("extraction is stubbed by the failing client below"),
        |_, _| supported(),
        |_| panic!("synthesis must not run when nothing was extracted"),
    );

    let mut deps = deps_with(
        Arc::clone(&client),
        search_returning(&["https://example.com/a", "https://example.com/b"]),
        Arc::new(SystemClock),
    );
    deps.extract_client = Arc::new(FailingExtract(Arc::clone(&client)));

    let error = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap_err();

    // The class the provider gave, not a synthesized empty answer.
    assert!(
        matches!(error.root(), AppError::Client(_)),
        "a whole-phase failure must surface, got: {error}"
    );
    assert!(
        error.to_string().contains("effort is not supported"),
        "the provider's reason must reach the caller: {error}"
    );
}

/// The line this draws: one surviving source is still a degraded run, not a
/// failure. 004 FR-013 makes per-source failure tolerable, and 023 must not
/// turn a partly-successful run into an error.
#[tokio::test]
async fn one_surviving_source_still_degrades_rather_than_failing() {
    let client = scripted(
        scope_value(),
        |prompt| {
            if prompt.contains("example.com/a") {
                json!({ "claims": ["the surviving claim"] })
            } else {
                json!({ "claims": ["also fine"] })
            }
        },
        |_, _| supported(),
        |_| json!({ "answer": "Survives [s1].", "gaps": [], "gap_targets": [] }),
    );
    let deps = deps_with(
        client,
        search_returning(&["https://example.com/a", "https://example.com/b"]),
        Arc::new(SystemClock),
    );
    // Only the first URL fetches; the second fails.
    let mut fetcher = MockFetcher::new();
    fetcher.expect_fetch().returning(|url| {
        if url.contains("example.com/a") {
            Ok(crate::traits::fetcher::FetchedPage {
                url: url.to_string(),
                html: article_html("Content that extracts."),
            })
        } else {
            Err(AppError::Client("fetch refused".to_string()))
        }
    });

    let (result, _) = run(&deps, &fetcher, &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(result.stats.sources_fetched, 1);
    assert!(!result.key_findings.is_empty());
}

/// A page that loaded and held no readable text is not a failure. A run where
/// every candidate is like that genuinely has no findings, and the honest
/// answer is the empty one — so this must NOT become an error.
#[tokio::test]
async fn unreadable_pages_still_produce_the_honest_empty_answer() {
    let client = scripted(
        scope_value(),
        |_| panic!("nothing readable to extract from"),
        |_, _| supported(),
        |_| panic!("synthesis must not be called with nothing to ground"),
    );
    let deps = deps_with(
        client,
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let mut fetcher = MockFetcher::new();
    fetcher.expect_fetch().returning(|url| {
        Ok(crate::traits::fetcher::FetchedPage {
            url: url.to_string(),
            html: "<html><body></body></html>".to_string(),
        })
    });

    let (result, _) = run(&deps, &fetcher, &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(result.stats.sources_fetched, 0);
    assert!(result.answer.contains("No verifiable findings"));
}

/// The same rule in the phase that dominates a run's cost. A systematic
/// verification failure would otherwise produce an answer asserting nothing
/// while reporting success — which reads as "the evidence did not support
/// anything" rather than "nothing was checked".
#[tokio::test]
async fn every_verification_failing_is_an_error_not_a_silent_empty_answer() {
    struct FailingVerify(Arc<ScriptedClient>);
    #[async_trait::async_trait]
    impl crate::traits::client::ModelClient for FailingVerify {
        async fn complete(&self, prompt: &str, schema: &Value) -> Result<Completion, AppError> {
            if prompt.contains("adversarial fact-checker") {
                return Err(AppError::Client("HTTP 400: bad request".to_string()));
            }
            self.0.complete(prompt, schema).await
        }
    }

    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["a claim that will never be checked"] }),
        |_, _| panic!("verification is stubbed by the failing client below"),
        |_| panic!("synthesis must not run when nothing was verified"),
    );

    let mut deps = deps_with(
        Arc::clone(&client),
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    deps.verify_client = Arc::new(FailingVerify(Arc::clone(&client)));

    let error = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap_err();
    assert!(matches!(error.root(), AppError::Client(_)), "got: {error}");
}

/// 028 T031/T032/T033 / FR-015, D3: a caller may reduce a run's concurrency;
/// a request above the ceiling is reduced to it rather than failing the run.
///
/// Calls the production function rather than restating it. The first version
/// of this test defined a local closure with the same expression and asserted
/// the closure against itself — it would have stayed green with the real
/// clamp deleted, which is the opposite of what a test citing FR-015 should do.
///
/// The asymmetry with the pass count is deliberate. Concurrency is advice
/// about running work already authorised and does not change what the answer
/// means, so exceeding the ceiling is reduced. A pass count *is* the basis for
/// the returned confidence, so exceeding that errors.
#[test]
fn a_caller_may_lower_concurrency_and_a_raise_is_reduced_to_the_ceiling() {
    use crate::research::contract::effective_concurrency;

    // Omitted: the configured value.
    assert_eq!(effective_concurrency(None, 8).unwrap(), 8);

    // Lower: honoured.
    assert_eq!(effective_concurrency(Some(2), 8).unwrap(), 2);
    assert_eq!(effective_concurrency(Some(1), 8).unwrap(), 1);

    // Equal: honoured, not an off-by-one.
    assert_eq!(effective_concurrency(Some(8), 8).unwrap(), 8);

    // Above the ceiling: reduced, and the run still proceeds.
    assert_eq!(effective_concurrency(Some(16), 8).unwrap(), 8);
    assert_eq!(effective_concurrency(Some(u32::MAX), 8).unwrap(), 8);

    // Zero is rejected, matching the published `minimum: 1` and matching how
    // `resolve_passes` treats the same mistake. Silently rewriting a value the
    // contract calls invalid would be a swallowed input.
    assert!(effective_concurrency(Some(0), 8).is_err());

    // The ceiling follows configuration, not a constant.
    assert_eq!(effective_concurrency(Some(16), 32).unwrap(), 16);
    assert_eq!(effective_concurrency(Some(16), 4).unwrap(), 4);
}
