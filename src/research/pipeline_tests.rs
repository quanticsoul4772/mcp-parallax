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
use crate::traits::search::{
    ContentsResults, MockSearchProvider, Page, SearchHit, SearchResults, SearchType,
};
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
            reported_cost_micros: None,
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
        max_age_hours: 168,
        // These fixtures share one scripted client, so every call site reports
        // the same model and existing token assertions are unchanged.
        routing: crate::routing::RoutingTable::single(
            "claude-opus-4-8",
            crate::routing::DefaultVar::AnthropicModel,
        ),
        reranker: None,
    }
}

/// A search mock returning `urls` as hits; contents left to the test.
fn search_hits(urls: &'static [&'static str]) -> MockSearchProvider {
    let mut search = MockSearchProvider::new();
    search.expect_search().returning(move |_| {
        Ok(SearchResults {
            hits: urls
                .iter()
                .map(|u| SearchHit {
                    url: (*u).to_string(),
                    title: format!("title of {u}"),
                })
                .collect(),
            cost_usd: None,
        })
    });
    search
}

/// [`search_hits`] whose contents call returns readable text for every
/// requested URL (what `fetcher_ok` did before 059).
fn search_returning(urls: &'static [&'static str]) -> MockSearchProvider {
    let mut search = search_hits(urls);
    search.expect_contents().returning(|req| {
        Ok(ContentsResults {
            pages: req
                .urls
                .iter()
                .map(|u| page(u, &format!("Content of {u}."), false))
                .collect(),
            cost_usd: None,
        })
    });
    search
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

/// A page as Exa would return it for `url`.
fn page(url: &str, text: &str, truncated: bool) -> Page {
    Page {
        requested_url: url.to_string(),
        url: url.to_string(),
        title: format!("title of {url}"),
        text: text.to_string(),
        truncated,
    }
}

// ---- 059 T006: text comes from one contents call -----------------------------

/// The search carries the depth's type and the caller's domain lists; the
/// local filter still drops what Exa returned anyway; one contents call asks
/// for exactly the kept URLs with the freshness limit and the cap; and the
/// extraction reads that text.
#[tokio::test]
async fn text_comes_from_one_contents_call_for_the_kept_urls_only() {
    let client = scripted(
        scope_value(),
        |prompt| {
            assert!(prompt.contains("Exa text for"), "{prompt}");
            json!({ "claims": [] })
        },
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let mut search = MockSearchProvider::new();
    search
        .expect_search()
        .withf(|req| {
            req.search_type == SearchType::DeepLite
                && req.include_domains.is_empty()
                && req.exclude_domains == vec!["evil.example".to_string()]
        })
        .returning(|_| {
            Ok(SearchResults {
                hits: [
                    "https://evil.example/x",
                    "https://good.example/a",
                    "https://good.example/b",
                ]
                .iter()
                .map(|u| SearchHit {
                    url: (*u).to_string(),
                    title: (*u).to_string(),
                })
                .collect(),
                cost_usd: None,
            })
        });
    search
        .expect_contents()
        .times(1)
        .withf(|req| {
            req.urls == ["https://good.example/a", "https://good.example/b"]
                && req.max_age_hours == 168
                && req.max_characters == extract::EXTRACT_TEXT_MAX_CHARS
        })
        .returning(|req| {
            Ok(ContentsResults {
                pages: req
                    .urls
                    .iter()
                    .map(|u| page(u, &format!("Exa text for {u}."), false))
                    .collect(),
                cost_usd: None,
            })
        });
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let p = ResearchParams {
        constraints: Some(Constraints {
            domains_deny: Some(vec!["evil.example".into()]),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Deep))
    };
    let (result, _) = run(&deps, &p).await.unwrap();
    assert_eq!(result.stats.sources_found, 3);
}

/// A kept URL that Exa returned no page for is dropped and counted; the
/// other source carries the run.
#[tokio::test]
async fn a_page_missing_from_contents_is_dropped_and_counted() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["good claim"] }),
        |_, _| supported(),
        |_| json!({ "answer": "Good [s2].", "gaps": [], "gap_targets": [] }),
    );
    let mut search = search_hits(&["https://gone.example/x", "https://example.com/ok"]);
    search.expect_contents().returning(|_| {
        Ok(ContentsResults {
            pages: vec![page("https://example.com/ok", "Readable.", false)],
            cost_usd: None,
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
    assert_eq!(result.stats.sources_found, 2);
    assert_eq!(result.stats.sources_fetched, 1);
    assert_eq!(result.key_findings.len(), 1);
}

/// A page returned under another address still pairs with its source by the
/// address that was requested; the result shows the page's own address.
#[tokio::test]
async fn a_redirected_page_still_pairs_with_its_source() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["a claim"] }),
        |_, _| supported(),
        |_| json!({ "answer": "Holds [s1].", "gaps": [], "gap_targets": [] }),
    );
    let mut search = search_hits(&["http://example.com/old"]);
    search.expect_contents().returning(|req| {
        let mut moved = page("https://example.com/new", "Moved text.", false);
        moved.requested_url = req.urls[0].clone();
        Ok(ContentsResults {
            pages: vec![moved],
            cost_usd: None,
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
    assert_eq!(result.stats.sources_fetched, 1);
    assert_eq!(result.sources[0].url, "https://example.com/new");
}

/// 059 T015: the run's Exa cost is every search's reported cost plus the
/// contents call's (two angles at 0.007, contents at 0.005).
#[tokio::test]
async fn the_runs_exa_cost_sums_every_search_and_the_contents_call() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let mut search = MockSearchProvider::new();
    search.expect_search().returning(|_| {
        Ok(SearchResults {
            hits: vec![SearchHit {
                url: "https://example.com/a".into(),
                title: "A".into(),
            }],
            cost_usd: Some(0.007),
        })
    });
    search.expect_contents().returning(|req| {
        Ok(ContentsResults {
            pages: vec![page(&req.urls[0], "Text.", false)],
            cost_usd: Some(0.005),
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let cost = crate::telemetry::SearchCost::default();
    run_at(
        &deps,
        &params("q?", Some(Depth::Quick)),
        4,
        &cost,
        &crate::telemetry::PageCounts::default(),
    )
    .await
    .unwrap();
    let total = cost.total().unwrap();
    assert!((total - 0.019).abs() < 1e-9, "{total}");
}

// ---- 059 review findings 1-4 ---------------------------------------------------

/// A clock fixed at the run start until `flag` is set, then ten minutes on —
/// so a stand-in call can make the deadline pass while it is running.
fn clock_passing_the_deadline_when(
    flag: Arc<std::sync::atomic::AtomicBool>,
) -> Arc<dyn TimeProvider> {
    let started = DateTime::parse_from_rfc3339("2026-11-22T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let mut clock = MockTimeProvider::new();
    clock.expect_now().returning(move || {
        if flag.load(std::sync::atomic::Ordering::SeqCst) {
            started + chrono::Duration::seconds(600)
        } else {
            started
        }
    });
    Arc::new(clock)
}

fn with_deadline(ms: u64) -> ResearchParams {
    ResearchParams {
        constraints: Some(Constraints {
            deadline_ms: Some(ms),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    }
}

/// Finding 1: every Exa call is bounded by the time left before the deadline.
#[tokio::test]
async fn each_exa_call_carries_the_time_left_before_the_deadline() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let thirty = Some(std::time::Duration::from_secs(30));
    let mut search = MockSearchProvider::new();
    search
        .expect_search()
        .withf(move |req| req.time_left == thirty)
        .returning(|_| {
            Ok(SearchResults {
                hits: vec![SearchHit {
                    url: "https://example.com/a".into(),
                    title: "A".into(),
                }],
                cost_usd: None,
            })
        });
    search
        .expect_contents()
        .withf(move |req| req.time_left == thirty)
        .returning(|req| {
            Ok(ContentsResults {
                pages: vec![page(&req.urls[0], "Text.", false)],
                cost_usd: None,
            })
        });
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let deps = deps_with(client, search, clock_passing_the_deadline_when(flag));
    run(&deps, &with_deadline(30_000)).await.unwrap();
}

/// Finding 1: a search cut off because the deadline arrived stops the run
/// early (the deadline's usual outcome), not as a provider failure.
#[tokio::test]
async fn a_search_cut_off_by_the_deadline_stops_the_run_early() {
    let client = scripted(
        scope_value(),
        |_| panic!("no extraction"),
        |_, _| panic!("no verification"),
        |_| panic!("nothing verified"),
    );
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let passed = Arc::clone(&flag);
    let mut search = MockSearchProvider::new();
    search.expect_search().returning(move |_| {
        passed.store(true, std::sync::atomic::Ordering::SeqCst);
        Err(AppError::Timeout {
            what: "research deadline",
            ms: 0,
        })
    });
    let deps = deps_with(client, search, clock_passing_the_deadline_when(flag));
    let (result, _) = run(&deps, &with_deadline(30_000)).await.unwrap();
    assert!(result.stats.stopped_early);
    assert_eq!(result.stats.stop_reason, Some(StopReason::Deadline));
}

/// Finding 1: the same for the contents call.
#[tokio::test]
async fn a_contents_call_cut_off_by_the_deadline_stops_the_run_early() {
    let client = scripted(
        scope_value(),
        |_| panic!("no extraction"),
        |_, _| panic!("no verification"),
        |_| panic!("nothing verified"),
    );
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let passed = Arc::clone(&flag);
    let mut search = search_hits(&["https://example.com/a"]);
    search.expect_contents().returning(move |_| {
        passed.store(true, std::sync::atomic::Ordering::SeqCst);
        Err(AppError::Timeout {
            what: "research deadline",
            ms: 0,
        })
    });
    let deps = deps_with(client, search, clock_passing_the_deadline_when(flag));
    let (result, _) = run(&deps, &with_deadline(30_000)).await.unwrap();
    assert!(result.stats.stopped_early);
    assert_eq!(result.stats.stop_reason, Some(StopReason::Deadline));
    assert!(result.sources.is_empty());
}

/// Finding 2: a page Exa returns under a denied domain (a redirect) is
/// dropped as filtered — never extracted, never cited.
#[tokio::test]
async fn a_redirect_into_a_denied_domain_is_dropped() {
    let client = scripted(
        scope_value(),
        |_| panic!("a denied page must not be extracted"),
        |_, _| supported(),
        |_| panic!("nothing to synthesize"),
    );
    let mut search = search_hits(&["https://good.example/x"]);
    search.expect_contents().returning(|req| {
        let mut moved = page("https://evil.example/y", "Denied text.", false);
        moved.requested_url = req.urls[0].clone();
        Ok(ContentsResults {
            pages: vec![moved],
            cost_usd: None,
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let p = ResearchParams {
        constraints: Some(Constraints {
            domains_deny: Some(vec!["evil.example".into()]),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    };
    let (result, _) = run(&deps, &p).await.unwrap();
    assert!(result.sources.is_empty());
    assert_eq!(result.stats.sources_fetched, 0);
}

/// Finding 3: pages pair with sources by the echoed address, then by the
/// page's own address, then — only when exactly one of each is left — the
/// two leftovers; anything else stays unpaired.
#[test]
fn pages_pair_by_id_then_url_then_a_single_leftover() {
    let mk = |requested: &str, url: &str| {
        let mut p = page(url, "t", false);
        p.requested_url = requested.to_string();
        p
    };
    let urls = |list: &[&str]| list.iter().map(|u| (*u).to_string()).collect::<Vec<_>>();

    // By echoed id, and by the page's own address when the id is foreign.
    let paired = pair_pages(
        &urls(&["https://a.example/1", "https://b.example/2"]),
        vec![
            mk("https://a.example/1", "https://a.example/moved"),
            mk("opaque-id", "https://b.example/2"),
        ],
    );
    assert_eq!(paired[0].as_ref().unwrap().url, "https://a.example/moved");
    assert_eq!(paired[1].as_ref().unwrap().url, "https://b.example/2");

    // One source and one page left over: paired.
    let single = pair_pages(
        &urls(&["https://a.example/1"]),
        vec![mk("opaque", "https://www.a.example/1?x=1")],
    );
    assert!(single[0].is_some());

    // Two of each left over: nothing is guessed.
    let two = pair_pages(
        &urls(&["https://a.example/1", "https://b.example/2"]),
        vec![
            mk("o1", "https://x.example/p"),
            mk("o2", "https://y.example/q"),
        ],
    );
    assert!(two.iter().all(Option::is_none));
}

/// Second review M2: pairing does not depend on response order. A page with
/// no echoed id whose address is another source's cannot push that source's
/// own page into the leftover rule; a duplicate is never placed elsewhere.
#[test]
fn pairing_survives_order_and_never_places_a_duplicate() {
    let mk = |requested: &str, url: &str| {
        let mut p = page(url, "t", false);
        p.requested_url = requested.to_string();
        p
    };
    let urls = |list: &[&str]| list.iter().map(|u| (*u).to_string()).collect::<Vec<_>>();
    let x = "https://x.example/1";
    let y = "https://y.example/2";

    // P1: no echoed id, address Y (X redirected to Y), listed first. P2:
    // echoed id Y. Y must get P2; X must not get P2 by the leftover rule.
    let paired = pair_pages(
        &urls(&[x, y]),
        vec![mk("opaque", y), mk(y, "https://y.example/2-final")],
    );
    assert_eq!(paired[1].as_ref().unwrap().url, "https://y.example/2-final");
    assert!(paired[0].is_none(), "X took a page that belongs to Y");

    // A second page echoing X's id is a duplicate, never placed under Y.
    let dup = pair_pages(
        &urls(&[x, y]),
        vec![mk(x, x), mk(x, "https://x.example/dup")],
    );
    assert_eq!(dup[0].as_ref().unwrap().url, x);
    assert!(
        dup[1].is_none(),
        "a duplicate was placed under another source"
    );
}

/// Second review L1: only a call cut off by time takes the deadline's
/// stop-early path. A rejection (a bad key, no credits) arriving after the
/// deadline passed still fails the run, for the search and contents calls.
#[tokio::test]
async fn a_rejection_past_the_deadline_still_fails_the_run() {
    for in_contents in [false, true] {
        let client = scripted(
            scope_value(),
            |_| panic!("no extraction"),
            |_, _| panic!("no verification"),
            |_| panic!("no synthesis"),
        );
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let passed = Arc::clone(&flag);
        let rejection = move || {
            passed.store(true, std::sync::atomic::Ordering::SeqCst);
            AppError::ProviderRejected {
                status: 402,
                detail: "no credits".into(),
            }
        };
        let mut search = MockSearchProvider::new();
        if in_contents {
            search.expect_search().returning(|_| {
                Ok(SearchResults {
                    hits: vec![SearchHit {
                        url: "https://example.com/a".into(),
                        title: "A".into(),
                    }],
                    cost_usd: None,
                })
            });
            search
                .expect_contents()
                .returning(move |_| Err(rejection()));
        } else {
            search.expect_search().returning(move |_| Err(rejection()));
        }
        let deps = deps_with(client, search, clock_passing_the_deadline_when(flag));
        let err = run(&deps, &with_deadline(30_000)).await.unwrap_err();
        assert!(
            matches!(err.root(), AppError::ProviderRejected { status: 402, .. }),
            "in_contents={in_contents}: {err}"
        );
    }
}

/// Second review L3: a call that sent nothing (no time was left) cost
/// nothing, so it does not make the run's cost unknown.
#[tokio::test]
async fn a_call_that_sent_nothing_leaves_the_cost_known() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let first = std::sync::atomic::AtomicBool::new(true);
    let mut search = MockSearchProvider::new();
    search.expect_search().returning(move |_| {
        if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
            Ok(SearchResults {
                hits: vec![SearchHit {
                    url: "https://example.com/a".into(),
                    title: "A".into(),
                }],
                cost_usd: Some(0.007),
            })
        } else {
            Err(AppError::Timeout {
                what: crate::traits::search::NOT_SENT,
                ms: 0,
            })
        }
    });
    search.expect_contents().returning(|req| {
        Ok(ContentsResults {
            pages: vec![page(&req.urls[0], "Text.", false)],
            cost_usd: Some(0.005),
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let cost = crate::telemetry::SearchCost::default();
    run_at(
        &deps,
        &params("q?", Some(Depth::Quick)),
        4,
        &cost,
        &crate::telemetry::PageCounts::default(),
    )
    .await
    .unwrap();
    let total = cost.total().unwrap();
    assert!((total - 0.012).abs() < 1e-9, "{total}");
}

/// Finding 4: a failed Exa call may still have been billed, so the run's
/// cost is unknown rather than the successful calls' sum.
#[tokio::test]
async fn a_failed_exa_call_makes_the_runs_cost_unknown() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let failed = std::sync::atomic::AtomicBool::new(false);
    let mut search = MockSearchProvider::new();
    search.expect_search().returning(move |_| {
        if failed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            Err(AppError::SearchProvider("HTTP 503".into()))
        } else {
            Ok(SearchResults {
                hits: vec![SearchHit {
                    url: "https://example.com/a".into(),
                    title: "A".into(),
                }],
                cost_usd: Some(0.007),
            })
        }
    });
    search.expect_contents().returning(|req| {
        Ok(ContentsResults {
            pages: vec![page(&req.urls[0], "Text.", false)],
            cost_usd: Some(0.005),
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let cost = crate::telemetry::SearchCost::default();
    run_at(
        &deps,
        &params("q?", Some(Depth::Quick)),
        4,
        &cost,
        &crate::telemetry::PageCounts::default(),
    )
    .await
    .unwrap();
    assert_eq!(cost.total(), None);
}

/// A page whose text reached the cap is marked truncated on the result.
#[tokio::test]
async fn a_truncated_page_is_marked_on_the_result() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["a claim"] }),
        |_, _| supported(),
        |_| json!({ "answer": "Holds [s1].", "gaps": [], "gap_targets": [] }),
    );
    let mut search = search_hits(&["https://example.com/long"]);
    search.expect_contents().returning(|req| {
        Ok(ContentsResults {
            pages: vec![page(&req.urls[0], "Cut short.", true)],
            cost_usd: None,
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
    assert_eq!(result.sources.len(), 1);
    assert!(result.sources[0].truncated);
}

/// 059 analyze B1: the contents request failing loses every source, so the
/// run fails with that error (023), and the scope spend stays metered (020).
#[tokio::test]
async fn the_contents_request_failing_fails_the_run_metered() {
    let client = scripted(
        scope_value(),
        |_| panic!("no extraction without text"),
        |_, _| panic!("no verification"),
        |_| panic!("no synthesis"),
    );
    let mut search = search_hits(&["https://example.com/a"]);
    search
        .expect_contents()
        .returning(|_| Err(AppError::SearchProvider("HTTP 503".into())));
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let err = run(&deps, &params("q?", Some(Depth::Quick)))
        .await
        .unwrap_err();
    assert!(matches!(err.root(), AppError::SearchProvider(_)), "{err}");
    let (input_tokens, output_tokens) = err.billed();
    assert!(
        input_tokens > 0 && output_tokens > 0,
        "scope spend lost on a contents failure: {input_tokens}/{output_tokens}"
    );
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

    let (result, usage) = run(&deps, &params("does it hold?", Some(Depth::Quick)))
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
    assert!(!wire.contains("Content of https://example.com/a."));

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
                prompt.contains("Content of https://example.com/a."),
                "the judge must receive the fetched evidence, not titles alone"
            );
            assert!(prompt.contains("[s1]"));
            supported()
        },
        |_| json!({ "answer": "ok [s1]", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&["https://example.com/a"]);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

    assert_eq!(result.stats.claims_verified, 1);
    // FR-012 still holds: evidence reaches the judge, never the wire.
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains("Content of https://example.com/a."));
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
    run(&deps, &p).await.unwrap();

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
    let mut search = search_hits(&["https://bad.example/x", "https://example.com/ok"]);
    search.expect_contents().returning(|_| {
        Ok(ContentsResults {
            pages: vec![page("https://example.com/ok", "Reachable content.", false)],
            cost_usd: None,
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
    assert_eq!(result.stats.sources_found, 2);
    assert_eq!(result.stats.sources_fetched, 1);
    assert_eq!(result.key_findings.len(), 1);
}

/// When every claim is refuted, synthesis is skipped and the server writes the
/// answer itself. A refuted claim is one the excerpts did not establish, not
/// one shown false (2026-11-19), so that answer must not say "refuted" either.
#[tokio::test]
async fn an_all_refuted_run_says_the_sources_did_not_establish_the_claims() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["the first claim", "the second claim"] }),
        |_, _| refuted("the excerpts do not establish it"),
        |_| json!({ "answer": "unused", "gaps": [], "gap_targets": [] }),
    );
    let search = search_returning(&["https://example.com/one"]);
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &params("q?", Some(Depth::Standard)))
        .await
        .unwrap();

    assert!(result.key_findings.is_empty());
    assert!(
        result.answer.contains("did not establish"),
        "{}",
        result.answer
    );
    assert!(!result.answer.contains("efuted"), "{}", result.answer);
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

    let (result, _usage) = run(&deps, &params("q?", Some(Depth::Standard)))
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

    let (result, _usage) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
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

    let (result, _usage) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
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

    let (result, _usage) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
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
        let err = run(&deps, &build).await.unwrap_err();
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
        run(&deps, &params("q?", Some(depth))).await.unwrap();
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
    let mut search = search_hits(&[
        "https://example.com/1",
        "https://example.com/2",
        "https://example.com/3",
    ]);
    search
        .expect_contents()
        .times(1)
        .withf(|req| req.urls.len() == 1)
        .returning(|req| {
            Ok(ContentsResults {
                pages: vec![page(&req.urls[0], "Capped run content.", false)],
                cost_usd: None,
            })
        });
    let deps = deps_with(client, search, Arc::new(SystemClock));
    let p = ResearchParams {
        constraints: Some(Constraints {
            max_sources: Some(1),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Deep))
    };
    let (result, _usage) = run(&deps, &p).await.unwrap();
    assert_eq!(result.stats.sources_fetched, 1);
}

#[tokio::test]
async fn denied_and_unallowed_domains_never_reach_the_contents_call() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let mut search = search_hits(&[
        "https://evil.example/page",
        "https://good.example/page",
        "https://other.example/page",
    ]);
    // Only good.example may ever have its text requested.
    search
        .expect_contents()
        .times(1)
        .withf(|req| req.urls == ["https://good.example/page"])
        .returning(|req| {
            Ok(ContentsResults {
                pages: vec![page(&req.urls[0], "Allowed content.", false)],
                cost_usd: None,
            })
        });
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let p = ResearchParams {
        constraints: Some(Constraints {
            domains_allow: Some(vec!["good.example".into(), "evil.example".into()]),
            domains_deny: Some(vec!["evil.example".into()]),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    };
    run(&deps, &p).await.unwrap();
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
    search.expect_contents().times(0);
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));

    let p = ResearchParams {
        constraints: Some(Constraints {
            budget_tokens: Some(1_000),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    };
    let (result, _usage) = run(&deps, &p).await.unwrap();
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
    let mut search = search_hits(&["https://example.com/a"]);
    search.expect_contents().times(0);
    let deps = deps_with(client, search, Arc::new(clock));

    let p = ResearchParams {
        constraints: Some(Constraints {
            deadline_ms: Some(5_000),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    };
    let (result, _usage) = run(&deps, &p).await.unwrap();
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
                reported_cost_micros: None,
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

    let err = run(&deps, &params("q?", Some(Depth::Quick)))
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

    let err = run(&deps, &params("q?", Some(Depth::Deep)))
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
        .returning(|_| Err(AppError::SearchProvider("HTTP 503".into())));
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let err = run(&deps, &params("q?", Some(Depth::Quick)))
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
    search.expect_search().returning(move |_| {
        if failed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            Ok(SearchResults {
                hits: vec![SearchHit {
                    url: "https://example.com/only".into(),
                    title: "only".into(),
                }],
                cost_usd: None,
            })
        } else {
            Err(AppError::SearchProvider("HTTP 503".into()))
        }
    });
    search.expect_contents().returning(|req| {
        Ok(ContentsResults {
            pages: vec![page(&req.urls[0], "Surviving content.", false)],
            cost_usd: None,
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _usage) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
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

    let (result, _usage) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

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
    let (a, _) = run(&deps_a, &params("q?", Some(Depth::Quick)))
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
    let (b, _) = run(&deps_b, &params("q?", Some(Depth::Quick)))
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
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

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
    let (all_gapped, _) = run(&both, &params("q?", Some(Depth::Quick))).await.unwrap();
    let neither = make(json!([0, 0]));
    let (none_gapped, _) = run(&neither, &params("q?", Some(Depth::Quick)))
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
        let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

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
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

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
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

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
    let mut search = search_hits(&["https://example.com/a"]);
    search.expect_contents().times(0);
    let deps = deps_with(client, search, Arc::new(clock));

    let p = ResearchParams {
        constraints: Some(Constraints {
            deadline_ms: Some(5_000),
            ..Constraints::default()
        }),
        ..params("q?", Some(Depth::Quick))
    };
    let (result, _) = run(&deps, &p).await.unwrap();

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
/// Truncation itself is pinned by
/// `twelve_gaps_publish_ten_and_count_every_target` below.
#[tokio::test]
async fn nine_gaps_on_one_sub_question_leave_the_other_settled() {
    let client = scripted(
        scope_value(), // two sub-questions
        |_| json!({ "claims": ["the claim holds"] }),
        |_, _| supported(),
        // Ten gaps, nine of them piled on sub-question
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
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

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

/// The production failure, reproduced: a synthesis returned 12 gaps and the
/// run failed with `validation_failure` ("has more than 10 items at
/// /gap_targets"). The item cap lived only in the local validator — the
/// sanitizer strips `maxItems` from the grammar and the prompt never states
/// it — so the model had no way to honour it. Now the run succeeds, publishes
/// `MAX_GAPS` gap texts, and derives coverage from all twelve targets: the
/// two gaps past the cap, both on sub-question 2, still leave it unsettled.
#[tokio::test]
async fn twelve_gaps_publish_ten_and_count_every_target() {
    let client = scripted(
        scope_value(), // two sub-questions
        |_| json!({ "claims": ["the claim holds"] }),
        |_, _| supported(),
        |_| {
            json!({
                "answer": "It holds [s1].",
                "gaps": [
                    "g1", "g2", "g3", "g4", "g5", "g6",
                    "g7", "g8", "g9", "g10", "g11", "g12"
                ],
                "gap_targets": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 2]
            })
        },
    );
    let deps = deps_with(
        client,
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

    assert_eq!(
        result.gaps.len(),
        crate::research::MAX_GAPS,
        "published text is capped"
    );
    assert_eq!(result.gaps.last().map(String::as_str), Some("g10"));
    assert!((result.coverage - 0.5).abs() < f32::EPSILON);
    assert!(result.sub_question_status[0].settled);
    assert!(!result.sub_question_status[1].settled);
}

/// The same stripped-bound defect, for lengths: a gap phrase past
/// `MAX_GAP_CHARS` is cut at publication, not a failed run.
#[tokio::test]
async fn an_over_long_gap_is_cut_not_fatal() {
    let long = "x".repeat(crate::research::MAX_GAP_CHARS + 50);
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["the claim holds"] }),
        |_, _| supported(),
        move |_| {
            json!({
                "answer": "It holds [s1].",
                "gaps": [long.clone()],
                "gap_targets": [2]
            })
        },
    );
    let deps = deps_with(
        client,
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

    assert_eq!(
        result.gaps[0].chars().count(),
        crate::research::MAX_GAP_CHARS
    );
    assert_eq!(result.stats.stop_reason, None);
}

/// An answer past `MAX_ANSWER_CHARS` is fed back once, then demoted like a
/// malformed gap list — never a failed run, and never cut mid-citation.
#[tokio::test]
async fn an_over_long_answer_is_retried_then_demoted() {
    let long = format!("{} [s1].", "y".repeat(crate::research::MAX_ANSWER_CHARS));
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": ["the claim holds"] }),
        |_, _| supported(),
        move |_| {
            json!({
                "answer": long.clone(),
                "gaps": [],
                "gap_targets": []
            })
        },
    );
    let deps = deps_with(
        Arc::clone(&client),
        search_returning(&["https://example.com/a"]),
        Arc::new(SystemClock),
    );
    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();

    assert_eq!(client.count_containing("executive synthesis"), 2);
    assert_eq!(client.count_containing("the answer was"), 1);
    assert_eq!(
        result.stats.stop_reason,
        Some(StopReason::MalformedSynthesis)
    );
    assert!(result.answer.contains("over-long"), "{}", result.answer);
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

    let error = run(&deps, &params("q?", Some(Depth::Quick)))
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
    let mut search = search_hits(&["https://example.com/a", "https://example.com/b"]);
    // Only the first URL comes back; the second is missing.
    search.expect_contents().returning(|_| {
        Ok(ContentsResults {
            pages: vec![page(
                "https://example.com/a",
                "Content that extracts.",
                false,
            )],
            cost_usd: None,
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
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
    let mut search = search_hits(&["https://example.com/a"]);
    search.expect_contents().returning(|req| {
        Ok(ContentsResults {
            pages: vec![page(&req.urls[0], "", false)],
            cost_usd: None,
        })
    });
    let deps = deps_with(client, search, Arc::new(SystemClock));

    let (result, _) = run(&deps, &params("q?", Some(Depth::Quick))).await.unwrap();
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

    let error = run(&deps, &params("q?", Some(Depth::Quick)))
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

// ---- 061 US1: rerank keeps only the top pages ---------------------------------

const EIGHT: &[&str] = &[
    "https://example.com/u1",
    "https://example.com/u2",
    "https://example.com/u3",
    "https://example.com/u4",
    "https://example.com/u5",
    "https://example.com/u6",
    "https://example.com/u7",
    "https://example.com/u8",
];

/// Extraction prompts that carry `text` (verification prompts also quote
/// kept pages, so a plain count would see them twice).
fn extractions_with(client: &ScriptedClient, text: &str) -> usize {
    client
        .prompts
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.contains("extract falsifiable claims") && p.contains(text))
        .count()
}

fn shared_claim_client() -> Arc<ScriptedClient> {
    scripted(
        scope_value(),
        |_| json!({ "claims": ["the shared claim holds"] }),
        |_, _| supported(),
        |_| json!({ "answer": "Shared holds [s2].", "gaps": [], "gap_targets": [] }),
    )
}

/// 061 T008 (US1 scenario 1, FR-001/FR-002/FR-006): with more scorable pages
/// than the keep count, the reranker sees the question and every page in
/// fetched order, extraction runs on exactly the top-K pages, kept sources
/// carry their relevance, and the stats count what was kept.
#[tokio::test]
async fn rerank_keeps_only_the_top_pages_for_extraction() {
    let client = shared_claim_client();
    let mut deps = deps_with(
        Arc::clone(&client),
        search_returning(EIGHT),
        Arc::new(SystemClock),
    );
    let mut reranker = crate::traits::rerank::MockReranker::new();
    reranker
        .expect_rerank()
        .withf(|query, documents| {
            query == "does it hold?"
                && documents.len() == 8
                && documents[0].contains("example.com/u1")
                && documents[7].contains("example.com/u8")
        })
        .times(1)
        .returning(|_, _| {
            Ok(crate::traits::rerank::RerankResult {
                scores: vec![0.1, 0.9, 0.2, 0.8, 0.3, 0.7, 0.6, 0.5],
                total_tokens: 100,
            })
        });
    reranker
        .expect_model_id()
        .return_const("rerank-3".to_string());
    deps.reranker = Some(Arc::new(reranker));

    let (result, _usage) = run(&deps, &params("does it hold?", Some(Depth::Quick)))
        .await
        .unwrap();

    // Quick keeps 5: u2, u4, u6, u7, u8 (scores 0.9, 0.8, 0.7, 0.6, 0.5).
    assert_eq!(client.count_containing("extract falsifiable claims"), 5);
    for kept in ["u2", "u4", "u6", "u7", "u8"] {
        assert_eq!(
            extractions_with(&client, &format!("Content of https://example.com/{kept}.")),
            1,
            "{kept}"
        );
    }
    for dropped in ["u1", "u3", "u5"] {
        assert_eq!(
            extractions_with(
                &client,
                &format!("Content of https://example.com/{dropped}.")
            ),
            0,
            "{dropped}"
        );
    }
    assert_eq!(result.stats.sources_kept, 5);
    assert_eq!(result.stats.rerank_skipped, None);
    assert!(!result.sources.is_empty());
    assert!(
        result.sources.iter().all(|s| s.relevance.is_some()),
        "{:?}",
        result.sources
    );
}

/// 061 T008 (US1 scenario 2, FR-003; D7): with no more scorable pages than
/// the keep count there is nothing to cut, so the reranker is not called and
/// every page is kept.
#[tokio::test]
async fn with_k_or_fewer_pages_the_reranker_is_not_called() {
    let client = shared_claim_client();
    let mut deps = deps_with(
        Arc::clone(&client),
        search_returning(&EIGHT[..5]),
        Arc::new(SystemClock),
    );
    let mut reranker = crate::traits::rerank::MockReranker::new();
    reranker.expect_rerank().times(0);
    deps.reranker = Some(Arc::new(reranker));

    let (result, _usage) = run(&deps, &params("does it hold?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(client.count_containing("extract falsifiable claims"), 5);
    assert_eq!(result.stats.sources_kept, 5);
    assert!(result.sources.iter().all(|s| s.relevance.is_none()));
}

/// 061 T008 (US1 scenario 3, FR-004): without a reranker research is
/// unchanged: every fetched page is extracted and nothing is scored.
#[tokio::test]
async fn without_a_reranker_research_is_unchanged() {
    let client = shared_claim_client();
    let deps = deps_with(
        Arc::clone(&client),
        search_returning(EIGHT),
        Arc::new(SystemClock),
    );

    let (result, _usage) = run(&deps, &params("does it hold?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(client.count_containing("extract falsifiable claims"), 8);
    assert_eq!(result.stats.sources_kept, 8);
    assert_eq!(result.stats.rerank_skipped, None);
    assert!(result.sources.iter().all(|s| s.relevance.is_none()));
}

/// 061 T008 (D7): a page with no text is not sent to the reranker and stays
/// dropped as empty, as before.
#[tokio::test]
async fn an_empty_page_is_not_sent_to_the_reranker() {
    let client = shared_claim_client();
    let mut search = search_hits(EIGHT);
    search.expect_contents().returning(|req| {
        Ok(ContentsResults {
            pages: req
                .urls
                .iter()
                .map(|u| {
                    let text = if u.ends_with("/u3") {
                        String::new()
                    } else {
                        format!("Content of {u}.")
                    };
                    page(u, &text, false)
                })
                .collect(),
            cost_usd: None,
        })
    });
    let mut deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));
    let mut reranker = crate::traits::rerank::MockReranker::new();
    reranker
        .expect_rerank()
        .withf(|_, documents| documents.len() == 7 && documents.iter().all(|d| !d.contains("/u3")))
        .times(1)
        .returning(|_, documents| {
            Ok(crate::traits::rerank::RerankResult {
                scores: (0..documents.len())
                    .map(|i| f32::from(u16::try_from(i).unwrap()))
                    .collect(),
                total_tokens: 10,
            })
        });
    reranker
        .expect_model_id()
        .return_const("rerank-3".to_string());
    deps.reranker = Some(Arc::new(reranker));

    let (result, _usage) = run(&deps, &params("does it hold?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(client.count_containing("extract falsifiable claims"), 5);
    assert_eq!(result.stats.sources_kept, 5);
}

// ---- 061 US2: a failed rerank never fails the run ----------------------------

/// 061 T011 (US2, FR-005): a reranker error leaves today's behavior: every
/// page is extracted, nothing is scored, and the reason is recorded.
#[tokio::test]
async fn a_rerank_error_falls_back_and_records_why() {
    let client = shared_claim_client();
    let mut deps = deps_with(
        Arc::clone(&client),
        search_returning(EIGHT),
        Arc::new(SystemClock),
    );
    let mut reranker = crate::traits::rerank::MockReranker::new();
    reranker
        .expect_rerank()
        .returning(|_, _| Err(AppError::EmbeddingProvider("HTTP 503".into())));
    reranker
        .expect_model_id()
        .return_const("rerank-3".to_string());
    deps.reranker = Some(Arc::new(reranker));

    let (result, _usage) = run(&deps, &params("does it hold?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(client.count_containing("extract falsifiable claims"), 8);
    assert_eq!(result.stats.sources_kept, 8);
    let reason = result.stats.rerank_skipped.clone().unwrap();
    assert!(reason.starts_with("provider_error"), "{reason}");
    assert!(result.sources.iter().all(|s| s.relevance.is_none()));
}

/// 061 T011 (US2, FR-005): a reply that does not score every page is a
/// malformed reply: the same fallback, with that reason.
#[tokio::test]
async fn a_malformed_rerank_reply_falls_back_and_records_why() {
    let client = shared_claim_client();
    let mut deps = deps_with(
        Arc::clone(&client),
        search_returning(EIGHT),
        Arc::new(SystemClock),
    );
    let mut reranker = crate::traits::rerank::MockReranker::new();
    reranker.expect_rerank().returning(|_, _| {
        Ok(crate::traits::rerank::RerankResult {
            scores: vec![0.5, 0.4, 0.3],
            total_tokens: 1,
        })
    });
    reranker
        .expect_model_id()
        .return_const("rerank-3".to_string());
    deps.reranker = Some(Arc::new(reranker));

    let (result, _usage) = run(&deps, &params("does it hold?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert_eq!(client.count_containing("extract falsifiable claims"), 8);
    let reason = result.stats.rerank_skipped.unwrap();
    assert!(reason.starts_with("malformed_reply"), "{reason}");
}

/// A reranker that never answers in time.
struct SlowReranker;

#[async_trait::async_trait]
impl crate::traits::rerank::Reranker for SlowReranker {
    async fn rerank(
        &self,
        _: &str,
        _: &[String],
    ) -> Result<crate::traits::rerank::RerankResult, AppError> {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        Err(AppError::EmbeddingProvider("too late".into()))
    }

    fn model_id(&self) -> &'static str {
        "rerank-3"
    }
}

/// 061 T011 (US2, FR-007): the rerank call is bounded by the run's deadline;
/// cut off, the run still completes and records `timeout`, never an error.
#[tokio::test]
async fn a_rerank_cut_off_by_the_deadline_is_recorded_not_fatal() {
    let client = scripted(
        scope_value(),
        |_| json!({ "claims": [] }),
        |_, _| supported(),
        |_| json!({ "answer": "n/a", "gaps": [], "gap_targets": [] }),
    );
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let passed = Arc::clone(&flag);
    let mut search = search_hits(EIGHT);
    search.expect_contents().returning(move |req| {
        // The deadline passes once the pages are in.
        passed.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(ContentsResults {
            pages: req
                .urls
                .iter()
                .map(|u| page(u, &format!("Content of {u}."), false))
                .collect(),
            cost_usd: None,
        })
    });
    let mut deps = deps_with(client, search, clock_passing_the_deadline_when(flag));
    deps.reranker = Some(Arc::new(SlowReranker));

    let started = std::time::Instant::now();
    let (result, _usage) = run(&deps, &with_deadline(30_000)).await.unwrap();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "waited for the reranker"
    );
    assert_eq!(result.stats.rerank_skipped.as_deref(), Some("timeout"));
    assert!(result.stats.stopped_early);
}

// ---- 061 US3: the cut is visible ---------------------------------------------

fn top_k_reranker() -> crate::traits::rerank::MockReranker {
    let mut reranker = crate::traits::rerank::MockReranker::new();
    reranker.expect_rerank().returning(|_, _| {
        Ok(crate::traits::rerank::RerankResult {
            scores: vec![0.1, 0.9, 0.2, 0.8, 0.3, 0.7, 0.6, 0.5],
            total_tokens: 100,
        })
    });
    reranker
        .expect_model_id()
        .return_const("rerank-3".to_string());
    reranker
}

/// 061 T013 (US3, D8): a reranked run's usage carries the rerank tokens
/// under `rerank-3` (input only), so they reach the record's usage and cost;
/// the model-token budget does not count them.
#[tokio::test]
async fn rerank_tokens_reach_the_runs_usage_under_the_rerank_model() {
    let client = shared_claim_client();
    let mut deps = deps_with(
        Arc::clone(&client),
        search_returning(EIGHT),
        Arc::new(SystemClock),
    );
    deps.reranker = Some(Arc::new(top_k_reranker()));
    let (result, usage) = run(&deps, &params("does it hold?", Some(Depth::Quick)))
        .await
        .unwrap();
    let rerank: Vec<_> = usage.iter().filter(|(m, _)| *m == "rerank-3").collect();
    assert_eq!(rerank.len(), 1, "{:?}", usage.models());
    assert_eq!(
        (rerank[0].1.input_tokens, rerank[0].1.output_tokens),
        (100, 0)
    );
    // stats.tokens is model tokens only: 1 scope + 5 extract + 1 verify (the
    // five identical claims dedup to one) + 1 synthesis = 8 calls at (10 + 5);
    // the 100 rerank tokens are not among them.
    assert_eq!(result.stats.tokens, 8 * 15);
}

/// 061 T013 (US3): a skipped rerank adds no rerank usage.
#[tokio::test]
async fn a_skipped_rerank_adds_no_rerank_usage() {
    let client = shared_claim_client();
    let mut deps = deps_with(
        Arc::clone(&client),
        search_returning(EIGHT),
        Arc::new(SystemClock),
    );
    let mut reranker = crate::traits::rerank::MockReranker::new();
    reranker
        .expect_rerank()
        .returning(|_, _| Err(AppError::EmbeddingProvider("HTTP 503".into())));
    reranker
        .expect_model_id()
        .return_const("rerank-3".to_string());
    deps.reranker = Some(Arc::new(reranker));
    let (_result, usage) = run(&deps, &params("does it hold?", Some(Depth::Quick)))
        .await
        .unwrap();
    assert!(!usage.models().contains(&"rerank-3".to_string()));
}

/// 061 T013 (US3, FR-006; analyze A2): the run reports how many pages came
/// back and how many reached extraction, for the invocation record.
#[tokio::test]
async fn the_run_reports_pages_fetched_and_kept() {
    let client = shared_claim_client();
    let mut deps = deps_with(
        Arc::clone(&client),
        search_returning(EIGHT),
        Arc::new(SystemClock),
    );
    deps.reranker = Some(Arc::new(top_k_reranker()));
    let pages = crate::telemetry::PageCounts::default();
    run_at(
        &deps,
        &params("does it hold?", Some(Depth::Quick)),
        4,
        &crate::telemetry::SearchCost::default(),
        &pages,
    )
    .await
    .unwrap();
    assert_eq!(pages.get(), Some((8, 5)));
}

// ---- 061 review findings -------------------------------------------------------

/// Review finding 1: a page that came back under a denied domain is dropped
/// before the rerank, so it takes no top-K slot and its text never reaches
/// the reranker; the next-best allowed pages fill the cut.
#[tokio::test]
async fn a_denied_page_takes_no_top_k_slot_and_is_not_reranked() {
    let client = shared_claim_client();
    let mut search = search_hits(EIGHT);
    search.expect_contents().returning(|req| {
        let pages = req
            .urls
            .iter()
            .enumerate()
            .map(|(i, u)| {
                if i < 2 {
                    let mut moved =
                        page(&format!("https://evil.example/{i}"), "Denied text.", false);
                    moved.requested_url = u.clone();
                    moved
                } else {
                    page(u, &format!("Content of {u}."), false)
                }
            })
            .collect();
        Ok(ContentsResults {
            pages,
            cost_usd: None,
        })
    });
    let mut deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));
    let seen = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen_in = Arc::clone(&seen);
    let mut reranker = crate::traits::rerank::MockReranker::new();
    reranker.expect_rerank().returning(move |_, documents| {
        seen_in.lock().unwrap().extend(documents.iter().cloned());
        // Denied pages, if sent, would win every slot.
        let scores = documents
            .iter()
            .map(|d| if d.contains("Denied") { 1.0 } else { 0.5 })
            .collect();
        Ok(crate::traits::rerank::RerankResult {
            scores,
            total_tokens: 1,
        })
    });
    reranker
        .expect_model_id()
        .return_const("rerank-3".to_string());
    deps.reranker = Some(Arc::new(reranker));
    let p = ResearchParams {
        constraints: Some(Constraints {
            domains_deny: Some(vec!["evil.example".into()]),
            ..Constraints::default()
        }),
        ..params("does it hold?", Some(Depth::Quick))
    };

    let (result, _usage) = run(&deps, &p).await.unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 6, "only the six allowed pages are reranked");
    assert!(seen.iter().all(|d| !d.contains("Denied")));
    assert_eq!(client.count_containing("extract falsifiable claims"), 5);
    assert_eq!(result.stats.sources_kept, 5);
}

/// Review finding 2: fetched counts pages that came back with text; kept
/// counts only those that went to extraction, so kept never exceeds fetched.
/// Here 8 candidates, 6 pages back, one of them empty: 5 and 5.
#[tokio::test]
async fn kept_never_exceeds_fetched() {
    let client = shared_claim_client();
    let mut search = search_hits(EIGHT);
    search.expect_contents().returning(|req| {
        let pages = req
            .urls
            .iter()
            .take(6)
            .enumerate()
            .map(|(i, u)| page(u, if i == 5 { "  " } else { "Some text." }, false))
            .collect();
        Ok(ContentsResults {
            pages,
            cost_usd: None,
        })
    });
    let deps = deps_with(Arc::clone(&client), search, Arc::new(SystemClock));
    let pages = crate::telemetry::PageCounts::default();
    let (result, _usage) = run_at(
        &deps,
        &params("does it hold?", Some(Depth::Quick)),
        4,
        &crate::telemetry::SearchCost::default(),
        &pages,
    )
    .await
    .unwrap();
    assert_eq!(pages.get(), Some((5, 5)));
    assert_eq!(result.stats.sources_kept, 5);
}
