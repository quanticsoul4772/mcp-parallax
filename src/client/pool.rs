//! One model client per distinct routed model (018 D10).
//!
//! The pool lives here rather than in [`crate::routing`], which stays free of
//! any client dependency so its resolution tests need no client fixture, and
//! rather than in [`crate::server`], which is already the largest file in the
//! tree. Its one job: dedupe the routing table's model ids and build a client
//! for each.

use crate::client::AnthropicClient;
use crate::config::Config;
use crate::routing::{CallSite, RoutingTable};
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
    distinct: usize,
}

impl ClientPool {
    /// Build one client per distinct routed model and bind each call site to
    /// the one it resolved to.
    #[must_use]
    pub fn build(config: &Config) -> Self {
        Self::from_factory(&config.routing, |model| {
            Arc::new(AnthropicClient::for_model(config, model)) as Arc<dyn ModelClient>
        })
    }

    /// Build with a caller-supplied client factory — the seam that lets tests
    /// assert pooling behavior without constructing real clients or a `Config`.
    #[must_use]
    pub fn from_factory<F>(routing: &RoutingTable, mut factory: F) -> Self
    where
        F: FnMut(&str) -> Arc<dyn ModelClient>,
    {
        let mut by_model: BTreeMap<String, Arc<dyn ModelClient>> = BTreeMap::new();
        for model in routing.distinct_models() {
            let client = factory(&model);
            by_model.insert(model, client);
        }
        let distinct = by_model.len();

        // `distinct_models` is drawn from the same table `model_for` reads, so
        // every lookup below hits. The miss arm cannot fire; it exists because
        // `get` returns an `Option` and building a duplicate client is the only
        // behavior-preserving thing to do with a branch that never runs.
        let by_site = std::array::from_fn(|index| {
            let model = routing.model_for(CallSite::ALL[index]);
            by_model
                .get(model)
                .map_or_else(|| factory(model), Arc::clone)
        });

        Self { by_site, distinct }
    }

    /// The client a call site runs on.
    #[must_use]
    pub fn for_site(&self, site: CallSite) -> Arc<dyn ModelClient> {
        Arc::clone(&self.by_site[site.index()])
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

    /// Build a pool, recording every model the factory was asked for.
    fn pool_for(routing: RoutingTable) -> (ClientPool, Vec<String>) {
        let built = Mutex::new(Vec::new());
        let pool = ClientPool::from_factory(&routing, |model| {
            built.lock().unwrap().push(model.to_string());
            Arc::new(Tagged(model.to_string())) as Arc<dyn ModelClient>
        });
        let models = built.lock().unwrap().clone();
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
        assert_eq!(built.len(), 2);

        // The eleven judgment sites share one Arc.
        let verify = pool.for_site(CallSite::Verify);
        let decide = pool.for_site(CallSite::Decide);
        assert!(Arc::ptr_eq(&verify, &decide));

        // The bulk site does not share with them.
        let extract = pool.for_site(CallSite::ResearchExtract);
        assert!(!Arc::ptr_eq(&verify, &extract));
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
        let pool = ClientPool::from_factory(&routing, move |model| {
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
