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
    let mut registry = ModeRegistry::new();
    crate::modes::verify::register(&mut registry, 3).unwrap();
    register(&mut registry).unwrap();
    let verify_mode = research_verify_mode(registry.get(crate::modes::verify::VERIFY_ID).unwrap());
    ResearchDeps {
        model_client: client,
        search: Arc::new(search),
        clock,
        scope_mode: registry.get(SCOPE_MODE_ID).unwrap().clone(),
        extract_mode: registry.get(EXTRACT_MODE_ID).unwrap().clone(),
        synth_mode: registry.get(SYNTH_MODE_ID).unwrap().clone(),
        verify_mode,
        input_max_chars: 50_000,
        concurrency: 4,
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
        |_| json!({ "answer": "Shared holds [s1][s2]; solo noted [s2].", "gaps": [] }),
    );
    let search = search_returning(&["https://example.com/a", "https://example.com/b"]);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let (result, input_tokens, output_tokens) = run(
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
    assert_eq!((input_tokens, output_tokens), (60, 30));
    assert_eq!(result.stats.tokens, 90);
    assert!(result.confidence > 0.0);
}

#[tokio::test]
async fn focus_reaches_the_scope_prompt() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [] }),
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
        |_| json!({ "answer": "Good [s2].", "gaps": [] }),
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

    let (result, _, _) = run(&deps, &fetcher, &params("q?", Some(Depth::Quick)))
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
        |_| json!({ "answer": "Right [s1]; contested noted [s1].", "gaps": [] }),
    );
    let search = search_returning(&["https://example.com/one"]);
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _, _) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Standard)))
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
                json!({ "answer": "Fabricated [s99].", "gaps": [] })
            } else {
                json!({ "answer": "Grounded [s1].", "gaps": [] })
            }
        },
    );
    let search = search_returning(&["https://example.com/one"]);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let (result, _, _) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
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
        |_| json!({ "answer": "Always fabricated [s99].", "gaps": [] }),
    );
    let search = search_returning(&["https://example.com/one"]);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let (result, _, _) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
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

    let (result, _, _) = run(&deps, &fetcher, &params("q?", Some(Depth::Quick)))
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
            |_| json!({ "answer": "n/a", "gaps": [] }),
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
        |_| json!({ "answer": "n/a", "gaps": [] }),
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
    let (result, _, _) = run(&deps, &fetcher, &p).await.unwrap();
    assert_eq!(result.stats.sources_fetched, 1);
}

#[tokio::test]
async fn denied_and_unallowed_domains_never_reach_the_fetcher() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [] }),
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
    let search = search_returning(&["https://example.com/a"]);
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
    let (result, _, _) = run(&deps, &fetcher, &p).await.unwrap();
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
    let (result, _, _) = run(&deps, &fetcher, &p).await.unwrap();
    assert!(result.stats.stopped_early);
    assert_eq!(result.stats.stop_reason, Some(StopReason::Deadline));
    assert!(!result.answer.is_empty());
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
    assert!(matches!(err, AppError::SearchProvider(_)), "{err}");
}

#[tokio::test]
async fn partial_angle_failure_degrades_and_counts() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["surviving claim"] }),
        |_, _| supported(),
        |_| json!({ "answer": "Survives [s1].", "gaps": [] }),
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

    let (result, _, _) = run(&deps, &fetcher_ok(), &params("q?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(result.stats.angles, 2);
    assert_eq!(result.stats.searches, 1); // one angle lost, counted honestly
    assert_eq!(result.key_findings.len(), 1);
}
