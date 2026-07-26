//! One model client per distinct routed model (018 D10).
//!
//! The pool lives here rather than in [`crate::routing`], which stays free of
//! any client dependency so its resolution tests need no client fixture, and
//! rather than in [`crate::server`], which is already the largest file in the
//! tree. Its one job: dedupe the routing table's model ids and build a client
//! for each.

use crate::routing::{CallSite, Effort, RoutingTable};
use crate::traits::client::ModelClient;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Live model clients, one per **distinct** model in the routing table.
///
/// Call sites routed to the same model share one client (FR-004). Lookup is
/// total: the pool holds one `Arc` per call site, resolved at construction, so
/// asking for a client can neither miss nor fall back.
pub struct ClientPool {
    by_site: [Arc<dyn ModelClient>; CallSite::ALL.len()],
    /// Every `(routed model, effort state)` pair, built eagerly (028 D1).
    ///
    /// A per-call effort has no pooled client under 022's keying, and the two
    /// options 028's spec named were both worse than this: giving the
    /// completion seam a parameter reverses 018 D2 and touches every mock,
    /// while building a client per call pays forever for a bound that is
    /// small and static. Effort has exactly six states (five levels plus
    /// absent) and the routed model set is at most twelve, so the whole domain
    /// is ≤72 entries and in practice under a dozen — cheap enough to
    /// materialise up front, which keeps this a plain immutable value with no
    /// lock and no factory retained.
    by_effort: BTreeMap<(String, Option<Effort>), Arc<dyn ModelClient>>,
    /// The model each call site resolved to, so an override can find its
    /// site's row in `by_effort` without a back-reference to the table.
    site_models: [String; CallSite::ALL.len()],
    distinct: usize,
}

impl ClientPool {
    /// Build with a caller-supplied client factory — the seam that lets tests
    /// assert pooling behavior without constructing real clients or a `Config`.
    #[must_use]
    pub fn from_factory<F>(routing: &RoutingTable, mut factory: F) -> Self
    where
        F: FnMut(&str, Option<Effort>) -> Arc<dyn ModelClient>,
    {
        let mut by_key: BTreeMap<(String, Option<Effort>), Arc<dyn ModelClient>> = BTreeMap::new();
        for (model, effort) in routing.distinct_clients() {
            let client = factory(&model, effort);
            by_key.insert((model, effort), client);
        }
        let distinct = by_key.len();

        // `distinct_models` is drawn from the same table `model_for` reads, so
        // every lookup below hits. The miss arm cannot fire; it exists because
        // `get` returns an `Option` and building a duplicate client is the only
        // behavior-preserving thing to do with a branch that never runs.
        let by_site = std::array::from_fn(|index| {
            let site = CallSite::ALL[index];
            let model = routing.model_for(site);
            let effort = routing.effort_for(site);
            by_key
                .get(&(model.to_string(), effort))
                .map_or_else(|| factory(model, effort), Arc::clone)
        });

        // Complete the cross product (028 D1). `distinct_clients` covers only
        // the pairs the *configuration* produces; a caller may name any level
        // for any routed model, so every remaining combination is built now.
        // `distinct` deliberately keeps counting configured clients alone —
        // it reports what routing resolved to, and the override entries are
        // not part of that answer.
        let mut by_effort = by_key;
        let mut models: Vec<String> = CallSite::ALL
            .iter()
            .map(|site| routing.model_for(*site).to_string())
            .collect();
        models.sort_unstable();
        models.dedup();
        for model in models {
            for effort in Effort::ALL.map(Some).into_iter().chain([None]) {
                by_effort
                    .entry((model.clone(), effort))
                    .or_insert_with(|| factory(&model, effort));
            }
        }

        let site_models =
            std::array::from_fn(|index| routing.model_for(CallSite::ALL[index]).to_string());

        Self {
            by_site,
            by_effort,
            site_models,
            distinct,
        }
    }

    /// The client a call site runs on.
    #[must_use]
    pub fn for_site(&self, site: CallSite) -> Arc<dyn ModelClient> {
        Arc::clone(&self.by_site[site.index()])
    }

    /// The client a call site runs on at a caller-supplied effort (028 FR-002).
    ///
    /// `None` takes the site's configured binding unchanged — the default path
    /// is the same array index it was before this feature, so a deployment
    /// where no caller supplies an effort behaves identically and allocates
    /// nothing extra.
    ///
    /// Total, like [`Self::for_site`]: the map holds every
    /// `(routed model, effort state)` pair, so a lookup for a site's own model
    /// at any level cannot miss. The fallback exists only because `get`
    /// returns an `Option`.
    #[must_use]
    pub fn for_site_with_effort(
        &self,
        site: CallSite,
        effort: Option<Effort>,
    ) -> Arc<dyn ModelClient> {
        let Some(effort) = effort else {
            return self.for_site(site);
        };
        self.by_effort
            .get(&(self.site_models[site.index()].clone(), Some(effort)))
            .map_or_else(|| self.for_site(site), Arc::clone)
    }

    /// How many distinct clients were built — one per distinct model.
    #[must_use]
    pub const fn distinct(&self) -> usize {
        self.distinct
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::traits::client::Completion;
    use serde_json::Value;
    use std::sync::Mutex;

    /// A client that remembers which model it was built for, so tests can see
    /// which one a call site received.
    struct Tagged(String);

    #[async_trait::async_trait]
    impl ModelClient for Tagged {
        async fn complete(&self, _prompt: &str, _schema: &Value) -> Result<Completion, AppError> {
            Err(AppError::Client(self.0.clone()))
        }
    }

    /// Build a pool, recording the **distinct** models the factory was asked
    /// for.
    ///
    /// 028 D1 pre-builds every `(model, effort state)` pair, so the factory is
    /// now called six times per model rather than once. The 018 guarantee
    /// these tests protect was never about the call count — it is that a model
    /// id appears once, so two call sites routed alike cannot end up on
    /// separate clients. Counting distinct models states that guarantee
    /// directly instead of via a proxy the cross product invalidates.
    fn pool_for(routing: RoutingTable) -> (ClientPool, Vec<String>) {
        let built = Mutex::new(Vec::new());
        let pool = ClientPool::from_factory(&routing, |model, _effort| {
            built.lock().unwrap().push(model.to_string());
            Arc::new(Tagged(model.to_string())) as Arc<dyn ModelClient>
        });
        let mut models = built.lock().unwrap().clone();
        models.sort_unstable();
        models.dedup();
        (pool, models)
    }

    /// Which model a call site's client was built for.
    async fn model_of(pool: &ClientPool, site: CallSite) -> String {
        match pool.for_site(site).complete("", &Value::Null).await {
            Err(AppError::Client(model)) => model,
            other => panic!("tagged client should report its model, got {other:?}"),
        }
    }

    // T011 / FR-002: unrouted means one model everywhere and one client.
    #[tokio::test]
    async fn unrouted_builds_exactly_one_client() {
        let (pool, built) = pool_for(RoutingTable::single("claude-opus-4-8"));
        assert_eq!(pool.distinct(), 1);
        assert_eq!(built, vec!["claude-opus-4-8".to_string()]);
        for site in CallSite::ALL {
            assert_eq!(model_of(&pool, site).await, "claude-opus-4-8");
        }
    }

    // T009 / FR-004: call sites routed alike share one client; distinct models
    // get distinct clients.
    #[tokio::test]
    async fn one_client_per_distinct_model() {
        let routing = RoutingTable::resolve(
            vec![(
                "PARALLAX_MODEL_BULK".to_string(),
                "claude-haiku-4-5".to_string(),
            )],
            "claude-opus-5",
        )
        .unwrap();
        let (pool, built) = pool_for(routing);

        // Two models in the table, so exactly two clients — not twelve.
        assert_eq!(pool.distinct(), 2);
        assert_eq!(built.len(), 2, "two models, however many effort states");

        // The eleven judgment sites share one Arc.
        let verify = pool.for_site(CallSite::Verify);
        let decide = pool.for_site(CallSite::Decide);
        assert!(Arc::ptr_eq(&verify, &decide));

        // The bulk site does not share with them.
        let extract = pool.for_site(CallSite::ResearchExtract);
        assert!(!Arc::ptr_eq(&verify, &extract));
    }

    /// 028 T007 / FR-002: an override reaches a client built for the site's
    /// own model at the caller's level — a different client from the one the
    /// site runs on by default, and the *same* one when the override happens
    /// to match what was configured.
    #[tokio::test]
    async fn an_override_selects_a_client_for_the_sites_model_at_that_level() {
        let routing = RoutingTable::resolve(
            vec![
                (
                    "PARALLAX_MODEL_BULK".to_string(),
                    "claude-haiku-4-5".to_string(),
                ),
                ("PARALLAX_EFFORT_VERIFY".to_string(), "high".to_string()),
            ],
            "claude-opus-5",
        )
        .unwrap();
        let (pool, models) = pool_for(routing);

        // Still two models, whatever the effort states.
        assert_eq!(models.len(), 2);

        let default = pool.for_site_with_effort(CallSite::Verify, None);
        assert!(
            Arc::ptr_eq(&default, &pool.for_site(CallSite::Verify)),
            "no override must take the configured binding unchanged"
        );

        // An override differing from the configured level is a different client.
        let low = pool.for_site_with_effort(CallSite::Verify, Some(Effort::Low));
        assert!(!Arc::ptr_eq(&low, &default));

        // ...but still the site's own model. Routing stays operator-owned.
        assert_eq!(
            model_of_client(&low).await,
            "claude-opus-5",
            "an effort override must not move the call to another model"
        );

        // An override equal to the configured level lands on the same entry.
        let high = pool.for_site_with_effort(CallSite::Verify, Some(Effort::High));
        assert!(
            Arc::ptr_eq(&high, &default),
            "the configured pair is already in the map; naming it must not build a second"
        );

        // Every level is available for a bulk-routed site too, on its model.
        for effort in Effort::ALL {
            let client = pool.for_site_with_effort(CallSite::ResearchExtract, Some(effort));
            assert_eq!(model_of_client(&client).await, "claude-haiku-4-5");
        }
    }

    /// 028 T018 + T019 / FR-003, SC-002: with nothing configured and nothing
    /// supplied, every call site hands back **the same `Arc`** it did before
    /// this feature — not a new client that happens to be configured alike.
    ///
    /// The cross product exists in the map either way; what this pins is that
    /// the default path never consults it. `distinct()` therefore still counts
    /// configured clients alone, which is what the 018 startup report means by
    /// the number it prints.
    #[tokio::test]
    async fn the_default_path_returns_the_very_same_client_as_before() {
        let routing = RoutingTable::single("claude-opus-4-8");
        let (pool, models) = pool_for(routing);

        assert_eq!(models.len(), 1, "one model, however many effort states");
        assert_eq!(
            pool.distinct(),
            1,
            "the override entries are not part of what routing resolved to"
        );

        for site in CallSite::ALL {
            let before = pool.for_site(site);
            let after = pool.for_site_with_effort(site, None);
            assert!(
                Arc::ptr_eq(&before, &after),
                "{site:?}: no override must be the identical Arc, not an equal one"
            );
        }
    }

    /// Which model an arbitrary pooled client was built for.
    async fn model_of_client(client: &Arc<dyn ModelClient>) -> String {
        match client.complete("", &Value::Null).await {
            Err(AppError::Client(model)) => model,
            other => panic!("tagged client should report its model, got {other:?}"),
        }
    }

    /// A client that always fails, counting how many times it was asked.
    struct Failing(Arc<std::sync::atomic::AtomicUsize>);

    #[async_trait::async_trait]
    impl ModelClient for Failing {
        async fn complete(&self, _prompt: &str, _schema: &Value) -> Result<Completion, AppError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(AppError::Client("provider unreachable".into()))
        }
    }

    // T048 / FR-015a, FR-015b: a failing call site is never retried on another
    // model. The pool binds one client per call site at construction, so there
    // is no path by which a failure reaches a second one — this test pins that
    // structurally, because "no cross-model fallback" is a guarantee an
    // optimization could quietly break later.
    #[tokio::test]
    async fn a_failing_call_site_never_reaches_another_model() {
        let routing = RoutingTable::resolve(
            vec![(
                "PARALLAX_MODEL_BULK".to_string(),
                "unreachable-model".to_string(),
            )],
            "claude-opus-5",
        )
        .unwrap();

        let failing_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let healthy_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (f, h) = (Arc::clone(&failing_calls), Arc::clone(&healthy_calls));
        let pool = ClientPool::from_factory(&routing, move |model, _effort| {
            if model == "unreachable-model" {
                Arc::new(Failing(Arc::clone(&f))) as Arc<dyn ModelClient>
            } else {
                Arc::new(Tagged({
                    h.fetch_add(0, std::sync::atomic::Ordering::SeqCst);
                    model.to_string()
                })) as Arc<dyn ModelClient>
            }
        });

        // The routed call site fails...
        let result = pool
            .for_site(CallSite::ResearchExtract)
            .complete("", &Value::Null)
            .await;
        assert!(matches!(result, Err(AppError::Client(_))));
        assert_eq!(failing_calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        // ...and nothing re-ran it anywhere else. The judgment-tier client is
        // a different Arc entirely and was never consulted on its behalf.
        let judgment = pool.for_site(CallSite::Verify);
        let extract = pool.for_site(CallSite::ResearchExtract);
        assert!(!Arc::ptr_eq(&judgment, &extract));
        assert_eq!(
            failing_calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "exactly one attempt: no cross-model retry"
        );
    }

    // T010 / FR-001: each call site receives the client for its own model.
    #[tokio::test]
    async fn each_site_gets_its_own_resolved_model() {
        let routing = RoutingTable::resolve(
            vec![
                (
                    "PARALLAX_MODEL_BULK".to_string(),
                    "claude-haiku-4-5".to_string(),
                ),
                (
                    "PARALLAX_MODEL_RESEARCH_SYNTHESIZE".to_string(),
                    "claude-sonnet-5".to_string(),
                ),
            ],
            "claude-opus-5",
        )
        .unwrap();
        let (pool, _) = pool_for(routing);

        assert_eq!(
            model_of(&pool, CallSite::ResearchExtract).await,
            "claude-haiku-4-5"
        );
        assert_eq!(
            model_of(&pool, CallSite::ResearchSynthesize).await,
            "claude-sonnet-5"
        );
        assert_eq!(model_of(&pool, CallSite::Verify).await, "claude-opus-5");
        assert_eq!(
            model_of(&pool, CallSite::CheckpointReview).await,
            "claude-opus-5"
        );
    }
}
