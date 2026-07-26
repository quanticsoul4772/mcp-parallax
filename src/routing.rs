//! Per-call-site model routing (018 D1).
//!
//! Each of the server's model call sites can run on a model chosen for the work
//! it does, instead of all of them sharing one `ANTHROPIC_MODEL`. Resolution is
//! most-specific-first: the call site's own setting, else its tier's setting,
//! else the server-wide default.
//!
//! **Effort resolves across four layers, not two (028).** A caller may supply
//! an effort on the invocation itself, which beats everything here:
//!
//! ```text
//! per-call argument                     <- 028, resolved in `server`
//!   else PARALLAX_EFFORT_<SITE>         <- 022, resolved here
//!     else PARALLAX_EFFORT_<TIER>       <- 022, resolved here
//!       else absent -> no effort field on the wire
//! ```
//!
//! Only the lower three are this module's; the top layer never reaches it,
//! because a per-call value is not part of the *table* — the table is what the
//! deployment resolved to, and it stays true even while a call overrides it.
//! That split is why [`RoutingTable::effort_for`] remains the right answer to
//! "what did configuration decide" and is not the right answer to "what did
//! this invocation send". The record answers the second (028 FR-007).
//!
//! Model selection has no fourth layer and must not grow one: which model runs
//! a call site sets the rate the operator is billed at, which is theirs to
//! decide. How much reasoning one task deserves is the caller's. The two were
//! conflated by 022 because the machinery was shared; they are not the same
//! kind of setting.
//!
//! This module is deliberately **pure** — it resolves and validates strings and
//! knows nothing about clients, networks, or providers. Building the clients a
//! resolved table implies is [`crate::client::pool`]'s job (018 D10), which is
//! what keeps these tests free of any client fixture.

use crate::error::ConfigError;
use std::collections::BTreeSet;

/// Reserved environment prefix for every routing variable.
///
/// The namespace is owned by routing: an unrecognised suffix is a startup error
/// rather than an ignored setting, which is what makes a misspelled route
/// visible (FR-006a).
pub const PREFIX: &str = "PARALLAX_MODEL_";
/// Reserved namespace for per-call-site reasoning effort (022).
pub const EFFORT_PREFIX: &str = "PARALLAX_EFFORT_";

/// How much reasoning a call site should spend (022).
///
/// Maps to the provider's `output_config.effort`. It governs *all* output
/// tokens, thinking included, and is a behavioural signal rather than a hard
/// token budget — `MAX_TOKENS` remains the ceiling.
///
/// Unset is not `High`: an unset call site sends no `effort` field at all, so
/// the request body is byte-identical to before this feature. The provider's
/// own default is `high`, so unset and `High` behave the same, but only unset
/// is provably unchanged on the wire.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
#[schemars(inline)] // flat + closed: no $ref/$defs in a tool input schema
pub enum Effort {
    /// Minimal reasoning; thinking skipped on simple tasks.
    Low,
    /// Moderate reasoning; thinking may be skipped on simple queries.
    Medium,
    /// Deep reasoning on complex tasks. The provider's default.
    High,
    /// Always thinks, no constraint on depth.
    Max,
    /// Always thinks deeply, with extended exploration.
    XHigh,
}

impl Effort {
    /// Every level, cheapest first.
    pub const ALL: [Self; 5] = [Self::Low, Self::Medium, Self::High, Self::Max, Self::XHigh];

    /// The wire spelling the provider expects.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
            Self::XHigh => "xhigh",
        }
    }

    /// Parse an operator-supplied value, case-insensitively.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_lowercase();
        Self::ALL.into_iter().find(|e| e.as_str() == value)
    }
}

/// A work-kind grouping of call sites. Membership is fixed by the server; the
/// model a tier uses is the operator's to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// Transcription work whose volume scales with the size of a run.
    Bulk,
    /// Everything that exercises judgment.
    Judgment,
}

impl Tier {
    /// Every tier, for validation and reporting.
    pub const ALL: [Self; 2] = [Self::Bulk, Self::Judgment];

    /// The environment-variable suffix naming this tier.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Bulk => "BULK",
            Self::Judgment => "JUDGMENT",
        }
    }

    /// Lowercase id, used in the startup routing table.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Bulk => "bulk",
            Self::Judgment => "judgment",
        }
    }
}

/// One named place in the server that asks a model for a schema-constrained
/// answer — the routable unit.
///
/// **Declaration order is load-bearing** (035): [`Self::index`] is the
/// discriminant, and `ClientPool` keys its per-site array on it. Reordering
/// these variants reorders that array; `ALL` must be reordered to match, and
/// `index_matches_all_order` fails if it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(usize)]
pub enum CallSite {
    /// The `verify` corrective.
    Verify,
    /// The `unstick` corrective.
    Unstick,
    /// The `diverge` corrective.
    Diverge,
    /// The `decide` corrective.
    Decide,
    /// The `elicit` corrective.
    Elicit,
    /// The `grounded_verify` corrective.
    GroundedVerify,
    /// The deterministic layer's claim-to-formal-target translation.
    CheckTranslate,
    /// Research phase 1: decompose the question into angles.
    ResearchScope,
    /// Research phase 3: per-source claim extraction. The only call site whose
    /// volume scales with the number of fetched sources.
    ResearchExtract,
    /// Research phase 4: refute-biased per-claim verification.
    ResearchVerify,
    /// Research phase 5: write the answer prose.
    ResearchSynthesize,
    /// The end-of-turn checkpoint review hop.
    CheckpointReview,
}

impl CallSite {
    /// Every call site. The complete routable set (data-model.md §1).
    pub const ALL: [Self; 12] = [
        Self::Verify,
        Self::Unstick,
        Self::Diverge,
        Self::Decide,
        Self::Elicit,
        Self::GroundedVerify,
        Self::CheckTranslate,
        Self::ResearchScope,
        Self::ResearchExtract,
        Self::ResearchVerify,
        Self::ResearchSynthesize,
        Self::CheckpointReview,
    ];

    /// Position in [`Self::ALL`].
    ///
    /// Lets a caller key a fixed-size array by call site, so lookup is total —
    /// no `Option`, no unreachable fallback branch. `ClientPool` keys
    /// `by_site` on it, so a wrong answer here does not fail loudly: it hands
    /// one call site another's client, and the invocation record attributes
    /// the cost to the model that did not run.
    ///
    /// **Derived from the discriminant, not hand-written** (035). This was a
    /// twelve-arm `match` mapping each variant to a literal, which meant two
    /// orderings — this one and `ALL` — that had to agree and could only be
    /// kept agreeing by a test. Both drift directions were in fact caught
    /// (reordering by `index_matches_all_order`, adding a variant by
    /// exhaustiveness), so this is not a bug fix. It removes the class instead
    /// of guarding it: a fieldless enum casts to its declaration order, so
    /// there is no longer a second ordering that *can* disagree.
    ///
    /// The remaining invariant is narrower — `ALL` must be written in
    /// declaration order — and `index_matches_all_order` now guards exactly
    /// that, which is why the test is kept rather than retired.
    ///
    /// A linear `ALL.iter().position(..)` would also work and was rejected:
    /// it returns an `Option` whose `None` arm is unreachable, and Principle
    /// III's ban on `unwrap` in production paths would make that arm either a
    /// silent fallback or a panic. The cast has neither problem and is free.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Stable lowercase id, used in the startup routing table and in tests.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Unstick => "unstick",
            Self::Diverge => "diverge",
            Self::Decide => "decide",
            Self::Elicit => "elicit",
            Self::GroundedVerify => "grounded_verify",
            Self::CheckTranslate => "check_translate",
            Self::ResearchScope => "research_scope",
            Self::ResearchExtract => "research_extract",
            Self::ResearchVerify => "research_verify",
            Self::ResearchSynthesize => "research_synthesize",
            Self::CheckpointReview => "checkpoint_review",
        }
    }

    /// The environment-variable suffix overriding this call site alone.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Verify => "VERIFY",
            Self::Unstick => "UNSTICK",
            Self::Diverge => "DIVERGE",
            Self::Decide => "DECIDE",
            Self::Elicit => "ELICIT",
            Self::GroundedVerify => "GROUNDED_VERIFY",
            Self::CheckTranslate => "CHECK_TRANSLATE",
            Self::ResearchScope => "RESEARCH_SCOPE",
            Self::ResearchExtract => "RESEARCH_EXTRACT",
            Self::ResearchVerify => "RESEARCH_VERIFY",
            Self::ResearchSynthesize => "RESEARCH_SYNTHESIZE",
            Self::CheckpointReview => "CHECKPOINT_REVIEW",
        }
    }

    /// Which tier this call site belongs to. Only research extraction is
    /// `Bulk`: it is the one call site that runs once per fetched source, so it
    /// is the only one where a cheaper model changes the bill materially.
    #[must_use]
    pub const fn tier(self) -> Tier {
        match self {
            Self::ResearchExtract => Tier::Bulk,
            _ => Tier::Judgment,
        }
    }
}

/// Which setting supplied a call site's model — reported in the startup table
/// so an operator can tell a deliberate route from a fall-through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    /// The call site's own `PARALLAX_MODEL_<SITE>` setting.
    Site,
    /// The call site's tier setting.
    Tier,
    /// `ANTHROPIC_MODEL`.
    Default,
}

impl RouteSource {
    /// The variable name that supplied the model, for the startup table.
    #[must_use]
    pub fn variable(self, site: CallSite) -> String {
        match self {
            Self::Site => format!("{PREFIX}{}", site.suffix()),
            Self::Tier => format!("{PREFIX}{}", site.tier().suffix()),
            Self::Default => "ANTHROPIC_MODEL".to_string(),
        }
    }
}

/// Reject an unrecognised suffix or an unparseable level in the effort
/// namespace (022).
///
/// Same treatment the model namespace gets, for the same reason: a misspelled
/// setting that silently does nothing leaves a call site at the provider
/// default while the operator believes it was changed.
fn validate_effort_namespace(efforts: &[(String, String)]) -> Result<(), ConfigError> {
    let known: BTreeSet<String> = Tier::ALL
        .iter()
        .map(|tier| format!("{EFFORT_PREFIX}{}", tier.suffix()))
        .chain(
            CallSite::ALL
                .iter()
                .map(|site| format!("{EFFORT_PREFIX}{}", site.suffix())),
        )
        .collect();
    for (name, value) in efforts {
        if !known.contains(name) {
            return Err(ConfigError::Routing(format!(
                "unknown variable `{name}` in the reserved `{EFFORT_PREFIX}*` namespace                  — check the spelling against the call-site and tier names"
            )));
        }
        if Effort::parse(value).is_none() {
            let levels: Vec<&str> = Effort::ALL.iter().map(|e| e.as_str()).collect();
            return Err(ConfigError::Routing(format!(
                "`{name}` is `{value}` — expected one of {}",
                levels.join(", ")
            )));
        }
    }
    Ok(())
}

/// One call site's resolved model and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoute {
    /// The call site.
    pub site: CallSite,
    /// The model id it will use.
    pub model: String,
    /// Which setting supplied that model.
    pub source: RouteSource,
    /// The reasoning effort this call site will request, if the operator set
    /// one (022). `None` means no `effort` field is sent at all.
    pub effort: Option<Effort>,
    /// Which setting supplied the effort, when one was.
    pub effort_source: Option<RouteSource>,
}

/// Every call site's resolved route. Complete by construction — all twelve
/// always resolve, because the server-wide default is the final fall-through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingTable {
    routes: Vec<ResolvedRoute>,
}

impl RoutingTable {
    /// Resolve every call site from the process environment.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Routing`] when a `PARALLAX_MODEL_*` variable has an
    /// unrecognised suffix (FR-006a) or is present but empty (FR-006).
    pub fn from_env(default_model: &str) -> Result<Self, ConfigError> {
        Self::resolve(std::env::vars(), default_model)
    }

    /// Resolve from an arbitrary variable set — the testable form. Pure: no
    /// process environment, no I/O.
    ///
    /// # Errors
    ///
    /// As [`Self::from_env`].
    pub fn resolve<I>(vars: I, default_model: &str) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let vars: Vec<(String, String)> = vars.into_iter().collect();
        let mut settings: Vec<(String, String)> = vars
            .iter()
            .filter(|(name, _)| name.starts_with(PREFIX))
            .cloned()
            .collect();
        let mut efforts: Vec<(String, String)> = vars
            .iter()
            .filter(|(name, _)| name.starts_with(EFFORT_PREFIX))
            .cloned()
            .collect();
        efforts.sort_by(|(a, _), (b, _)| a.cmp(b));
        // Deterministic error ordering: with two bad variables the message must
        // not depend on environment iteration order.
        settings.sort_by(|(a, _), (b, _)| a.cmp(b));

        let known: BTreeSet<String> = Tier::ALL
            .iter()
            .map(|tier| format!("{PREFIX}{}", tier.suffix()))
            .chain(
                CallSite::ALL
                    .iter()
                    .map(|site| format!("{PREFIX}{}", site.suffix())),
            )
            .collect();

        for (name, value) in &settings {
            if !known.contains(name) {
                return Err(ConfigError::Routing(format!(
                    "unknown variable `{name}` in the reserved `{PREFIX}*` namespace \
                     — check the spelling against the call-site and tier names"
                )));
            }
            if value.trim().is_empty() {
                return Err(ConfigError::Routing(format!(
                    "`{name}` is present but empty — set a model id or unset the variable"
                )));
            }
        }

        validate_effort_namespace(&efforts)?;

        let effort_lookup = |suffix: &str| -> Option<Effort> {
            let key = format!("{EFFORT_PREFIX}{suffix}");
            efforts
                .iter()
                .find(|(name, _)| *name == key)
                .and_then(|(_, value)| Effort::parse(value))
        };

        let lookup = |suffix: &str| -> Option<String> {
            let key = format!("{PREFIX}{suffix}");
            settings
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.trim().to_string())
        };

        let routes = CallSite::ALL
            .iter()
            .map(|&site| {
                // Most specific wins, for both namespaces independently: a
                // call site may take its model from a tier and its effort from
                // its own variable, or either from the default.
                let (model, source) = lookup(site.suffix()).map_or_else(
                    || {
                        lookup(site.tier().suffix()).map_or_else(
                            || (default_model.to_string(), RouteSource::Default),
                            |model| (model, RouteSource::Tier),
                        )
                    },
                    |model| (model, RouteSource::Site),
                );
                let (effort, effort_source) = effort_lookup(site.suffix()).map_or_else(
                    || {
                        effort_lookup(site.tier().suffix())
                            .map_or((None, None), |e| (Some(e), Some(RouteSource::Tier)))
                    },
                    |e| (Some(e), Some(RouteSource::Site)),
                );
                ResolvedRoute {
                    site,
                    model,
                    source,
                    effort,
                    effort_source,
                }
            })
            .collect();

        Ok(Self { routes })
    }

    /// A table with every call site on one model — the unrouted shape.
    ///
    /// Equivalent to resolving an empty environment, and the convenient form
    /// for fixtures that do not exercise routing.
    #[must_use]
    pub fn single(model: &str) -> Self {
        Self {
            routes: CallSite::ALL
                .iter()
                .map(|&site| ResolvedRoute {
                    site,
                    model: model.to_string(),
                    source: RouteSource::Default,
                    effort: None,
                    effort_source: None,
                })
                .collect(),
        }
    }

    /// The reasoning effort a call site requests, if any (022).
    #[must_use]
    pub fn effort_for(&self, site: CallSite) -> Option<Effort> {
        self.routes
            .iter()
            .find(|route| route.site == site)
            .and_then(|route| route.effort)
    }

    /// Every distinct `(model, effort)` pair in use, sorted — one client is
    /// built per entry (022 extends 018 FR-004: effort is part of the request
    /// body, so two call sites sharing a model but not an effort need two
    /// clients).
    #[must_use]
    pub fn distinct_clients(&self) -> Vec<(String, Option<Effort>)> {
        let mut pairs: Vec<(String, Option<Effort>)> = self
            .routes
            .iter()
            .map(|route| (route.model.clone(), route.effort))
            .collect();
        pairs.sort();
        pairs.dedup();
        pairs
    }

    /// The model a call site resolved to.
    #[must_use]
    pub fn model_for(&self, site: CallSite) -> &str {
        self.routes
            .iter()
            .find(|route| route.site == site)
            .map_or("", |route| route.model.as_str())
    }

    /// Every resolved route, in [`CallSite::ALL`] order.
    #[must_use]
    pub fn routes(&self) -> &[ResolvedRoute] {
        &self.routes
    }

    /// The startup report, one row per call site:
    /// `(call site, tier, resolved model, supplying setting)`.
    ///
    /// Pure, so what the operator is told at startup is assertable without a
    /// tracing capture harness (018 T012). The point of the report is that a
    /// misrouted call site is visible before the bill arrives, which only
    /// holds if **every** site appears with the setting that decided it.
    #[must_use]
    pub fn report(&self) -> Vec<(&'static str, &'static str, String, String)> {
        self.routes
            .iter()
            .map(|route| {
                (
                    route.site.id(),
                    route.site.tier().id(),
                    route.model.clone(),
                    route.source.variable(route.site),
                )
            })
            .collect()
    }

    /// The distinct models in use, sorted — one client is built per entry
    /// (FR-004).
    #[must_use]
    pub fn distinct_models(&self) -> Vec<String> {
        self.routes
            .iter()
            .map(|route| route.model.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// 018 T012: the startup report must name every call site with the model
    /// it resolved to and the setting that decided it. A misrouted site is
    /// only visible before the bill arrives if the report is complete — a
    /// report that silently omits sites is worse than none, because it reads
    /// as confirmation.

    /// 022: unset means unset. The whole feature is off by default, and the
    /// proof is that no route carries an effort when the namespace is empty —
    /// which is what keeps the request body byte-identical to pre-022.
    #[test]
    fn an_empty_effort_namespace_leaves_every_call_site_without_one() {
        let table = RoutingTable::resolve(vars(&[]), "claude-opus-4-8").unwrap();
        for route in table.routes() {
            assert_eq!(route.effort, None, "{}", route.site.id());
            assert_eq!(route.effort_source, None);
        }
        // One model, one effort (none) — still exactly one client.
        assert_eq!(table.distinct_clients().len(), 1);
    }

    /// 028 T020 / FR-002: the per-call layer sits above both configured ones,
    /// and the table below it is unchanged by an override.
    ///
    /// The table answers "what did configuration decide" and must keep
    /// answering that truthfully while a call overrides it — an override is
    /// not a mutation. This pins the composition the server performs
    /// (`override.or(configured)`) at the level where both halves are visible,
    /// because the server-side test can only observe the result.
    #[test]
    fn a_per_call_effort_outranks_both_configured_layers() {
        let table = RoutingTable::resolve(
            vars(&[
                ("PARALLAX_MODEL_BULK", "claude-haiku-4-5"),
                ("PARALLAX_EFFORT_JUDGMENT", "medium"),
                ("PARALLAX_EFFORT_VERIFY", "high"),
            ]),
            "claude-opus-5",
        )
        .unwrap();

        // Configured: site beats tier, tier covers the rest, bulk has neither.
        assert_eq!(table.effort_for(CallSite::Verify), Some(Effort::High));
        assert_eq!(table.effort_for(CallSite::Decide), Some(Effort::Medium));
        assert_eq!(table.effort_for(CallSite::ResearchExtract), None);

        // The server composes `override.or(configured)`. Each layer wins in
        // turn, and absent at every layer stays absent.
        let resolve = |site, over: Option<Effort>| over.or(table.effort_for(site));
        assert_eq!(
            resolve(CallSite::Verify, Some(Effort::Low)),
            Some(Effort::Low),
            "per-call beats a site setting"
        );
        assert_eq!(
            resolve(CallSite::Decide, Some(Effort::Low)),
            Some(Effort::Low),
            "per-call beats a tier setting"
        );
        assert_eq!(
            resolve(CallSite::ResearchExtract, Some(Effort::Max)),
            Some(Effort::Max),
            "per-call needs no configuration to take effect"
        );
        assert_eq!(resolve(CallSite::Verify, None), Some(Effort::High));
        assert_eq!(resolve(CallSite::ResearchExtract, None), None);

        // An override changes nothing about what the table reports, which is
        // what keeps the startup report true for the deployment's lifetime.
        assert_eq!(table.effort_for(CallSite::Verify), Some(Effort::High));
    }

    /// Model and effort resolve independently: a call site may take its model
    /// from a tier and its effort from its own variable, or either from the
    /// default. Collapsing them into one lookup would make the cheap tier
    /// unable to carry a cheap effort without also naming a model.
    #[test]
    fn effort_and_model_resolve_most_specific_first_and_independently() {
        let table = RoutingTable::resolve(
            vars(&[
                ("PARALLAX_MODEL_BULK", "claude-haiku-4-5"),
                ("PARALLAX_EFFORT_BULK", "low"),
                ("PARALLAX_EFFORT_VERIFY", "max"),
            ]),
            "claude-opus-4-8",
        )
        .unwrap();

        let route = |id: &str| {
            table
                .routes()
                .iter()
                .find(|r| r.site.id() == id)
                .expect("call site")
                .clone()
        };

        // Bulk tier: model and effort both from the tier variables.
        let extract = route("research_extract");
        assert_eq!(extract.model, "claude-haiku-4-5");
        assert_eq!(extract.effort, Some(Effort::Low));
        assert_eq!(extract.effort_source, Some(RouteSource::Tier));

        // Per-site effort on a default-model site: the two are independent.
        let verify = route("verify");
        assert_eq!(verify.model, "claude-opus-4-8");
        assert_eq!(verify.source, RouteSource::Default);
        assert_eq!(verify.effort, Some(Effort::Max));
        assert_eq!(verify.effort_source, Some(RouteSource::Site));

        // Untouched site: neither.
        let unstick = route("unstick");
        assert_eq!(unstick.model, "claude-opus-4-8");
        assert_eq!(unstick.effort, None);
    }

    /// A per-site effort beats its tier's, the same precedence the model
    /// namespace uses.
    #[test]
    fn a_per_site_effort_overrides_its_tier() {
        let table = RoutingTable::resolve(
            vars(&[
                ("PARALLAX_EFFORT_JUDGMENT", "low"),
                ("PARALLAX_EFFORT_DECIDE", "xhigh"),
            ]),
            "claude-opus-4-8",
        )
        .unwrap();
        let effort = |id: &str| {
            table
                .routes()
                .iter()
                .find(|r| r.site.id() == id)
                .and_then(|r| r.effort)
        };
        assert_eq!(effort("decide"), Some(Effort::XHigh));
        assert_eq!(effort("verify"), Some(Effort::Low));
        // Bulk is a different tier and the judgment setting must not reach it.
        assert_eq!(effort("research_extract"), None);
    }

    /// A typo in the effort namespace is a startup error naming the variable —
    /// the same treatment the model namespace gets, and for the same reason: a
    /// misspelled setting that silently does nothing is worse than a refusal
    /// to start.
    #[test]
    fn an_unknown_or_unparseable_effort_variable_is_a_startup_error() {
        let unknown = RoutingTable::resolve(
            vars(&[("PARALLAX_EFFORT_VERFIY", "low")]),
            "claude-opus-4-8",
        )
        .unwrap_err();
        assert!(
            unknown.to_string().contains("PARALLAX_EFFORT_VERFIY"),
            "{unknown}"
        );

        let bad_level = RoutingTable::resolve(
            vars(&[("PARALLAX_EFFORT_VERIFY", "cheap")]),
            "claude-opus-4-8",
        )
        .unwrap_err();
        let message = bad_level.to_string();
        assert!(message.contains("PARALLAX_EFFORT_VERIFY"), "{message}");
        assert!(message.contains("cheap"), "{message}");
        // The message must list what was expected, not just reject.
        for level in ["low", "medium", "high", "max", "xhigh"] {
            assert!(message.contains(level), "{message} is missing {level}");
        }

        // An empty value is caught by the model namespace's own rule; the
        // effort namespace rejects it as unparseable rather than silently
        // treating it as unset.
        let empty =
            RoutingTable::resolve(vars(&[("PARALLAX_EFFORT_VERIFY", "  ")]), "claude-opus-4-8")
                .unwrap_err();
        assert!(empty.to_string().contains("PARALLAX_EFFORT_VERIFY"));
    }

    /// Effort is part of the request body, so two call sites on one model at
    /// different efforts cannot share a client. Getting this wrong would send
    /// one site's effort on the other's calls.
    #[test]
    fn distinct_clients_keys_on_model_and_effort_together() {
        let table = RoutingTable::resolve(
            vars(&[
                ("PARALLAX_EFFORT_VERIFY", "max"),
                ("PARALLAX_EFFORT_DECIDE", "low"),
            ]),
            "claude-opus-4-8",
        )
        .unwrap();
        // One model, three effort states (max, low, none) → three clients.
        assert_eq!(table.distinct_models().len(), 1);
        assert_eq!(table.distinct_clients().len(), 3);

        // Same effort on the same model collapses to one entry.
        let shared = RoutingTable::resolve(
            vars(&[
                ("PARALLAX_EFFORT_VERIFY", "low"),
                ("PARALLAX_EFFORT_DECIDE", "low"),
            ]),
            "claude-opus-4-8",
        )
        .unwrap();
        assert_eq!(shared.distinct_clients().len(), 2); // low, and none
    }

    #[test]
    fn effort_parses_case_insensitively_and_round_trips() {
        assert_eq!(Effort::parse("LOW"), Some(Effort::Low));
        assert_eq!(Effort::parse(" xhigh "), Some(Effort::XHigh));
        assert_eq!(Effort::parse("enormous"), None);
        for level in Effort::ALL {
            assert_eq!(Effort::parse(level.as_str()), Some(level));
        }
    }

    #[test]
    fn the_startup_report_names_every_call_site_with_its_model_and_source() {
        let table = RoutingTable::resolve(
            vars(&[
                ("PARALLAX_MODEL_BULK", "claude-haiku-4-5"),
                ("PARALLAX_MODEL_VERIFY", "claude-opus-5"),
            ]),
            "claude-opus-4-8",
        )
        .unwrap();
        let report = table.report();

        // Every site, exactly once, in the canonical order.
        assert_eq!(report.len(), CallSite::ALL.len());
        let listed: Vec<&str> = report.iter().map(|row| row.0).collect();
        let expected: Vec<&str> = CallSite::ALL.iter().map(|s| s.id()).collect();
        assert_eq!(listed, expected);

        // Each row names a non-empty model and the setting that supplied it,
        // and the setting named is one that actually exists.
        for (site, tier, model, source) in &report {
            assert!(!model.is_empty(), "{site}: empty model");
            assert!(
                source == "ANTHROPIC_MODEL" || source.starts_with(PREFIX),
                "{site}: source {source} is not a real setting"
            );
            assert!(*tier == "bulk" || *tier == "judgment", "{site}: {tier}");
        }

        // The three resolution paths each appear, named by their own variable.
        let row = |id: &str| {
            report
                .iter()
                .find(|r| r.0 == id)
                .map(|r| (r.2.clone(), r.3.clone()))
                .unwrap()
        };
        assert_eq!(
            row("verify"),
            ("claude-opus-5".to_string(), "PARALLAX_MODEL_VERIFY".into()),
            "a per-site override must be reported as itself, not as its tier"
        );
        assert_eq!(
            row("research_extract"),
            ("claude-haiku-4-5".to_string(), "PARALLAX_MODEL_BULK".into()),
            "a tier route must name the tier variable"
        );
        assert_eq!(
            row("unstick"),
            ("claude-opus-4-8".to_string(), "ANTHROPIC_MODEL".into()),
            "an unrouted site must name the default, never blank"
        );
    }

    #[test]
    fn unset_namespace_resolves_every_site_to_the_default() {
        let table = RoutingTable::resolve(vars(&[]), "claude-opus-4-8").unwrap();
        assert_eq!(table.routes().len(), CallSite::ALL.len());
        for route in table.routes() {
            assert_eq!(route.model, "claude-opus-4-8");
            assert_eq!(route.source, RouteSource::Default);
        }
        // FR-002: one model everywhere means exactly one client.
        assert_eq!(table.distinct_models(), vec!["claude-opus-4-8".to_string()]);
    }

    #[test]
    fn tier_setting_fans_out_to_its_members_only() {
        let table = RoutingTable::resolve(
            vars(&[("PARALLAX_MODEL_BULK", "claude-haiku-4-5")]),
            "claude-opus-5",
        )
        .unwrap();

        assert_eq!(
            table.model_for(CallSite::ResearchExtract),
            "claude-haiku-4-5"
        );
        for site in CallSite::ALL {
            if site == CallSite::ResearchExtract {
                continue;
            }
            assert_eq!(table.model_for(site), "claude-opus-5", "{}", site.id());
        }
        assert_eq!(table.distinct_models().len(), 2);
    }

    #[test]
    fn site_setting_beats_tier_beats_default() {
        let table = RoutingTable::resolve(
            vars(&[
                ("PARALLAX_MODEL_JUDGMENT", "claude-sonnet-5"),
                ("PARALLAX_MODEL_VERIFY", "claude-opus-5"),
            ]),
            "claude-opus-4-8",
        )
        .unwrap();

        // Site override wins.
        assert_eq!(table.model_for(CallSite::Verify), "claude-opus-5");
        // Tier governs the rest of its members.
        assert_eq!(table.model_for(CallSite::Decide), "claude-sonnet-5");
        // The untouched tier still falls through to the default.
        assert_eq!(
            table.model_for(CallSite::ResearchExtract),
            "claude-opus-4-8"
        );

        let verify = table
            .routes()
            .iter()
            .find(|r| r.site == CallSite::Verify)
            .unwrap();
        assert_eq!(verify.source, RouteSource::Site);
        assert_eq!(
            verify.source.variable(CallSite::Verify),
            "PARALLAX_MODEL_VERIFY"
        );
    }

    #[test]
    fn unknown_suffix_is_rejected_by_name() {
        // The misspelled-route case: without namespace validation this would
        // be silently ignored and the only symptom is a bill that never drops.
        let error = RoutingTable::resolve(
            vars(&[("PARALLAX_MODEL_EXTRCT", "claude-haiku-4-5")]),
            "claude-opus-4-8",
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("PARALLAX_MODEL_EXTRCT"), "{message}");
        assert!(message.contains("unknown variable"), "{message}");
    }

    #[test]
    fn empty_value_is_rejected_by_name_never_defaulted() {
        for value in ["", "   "] {
            let error =
                RoutingTable::resolve(vars(&[("PARALLAX_MODEL_BULK", value)]), "claude-opus-4-8")
                    .unwrap_err();
            let message = error.to_string();
            assert!(message.contains("PARALLAX_MODEL_BULK"), "{message}");
            assert!(message.contains("present but empty"), "{message}");
        }
    }

    #[test]
    fn non_routing_variables_are_ignored() {
        let table = RoutingTable::resolve(
            vars(&[
                ("ANTHROPIC_MODEL", "ignored-here"),
                ("PATH", "/usr/bin"),
                ("PARALLAX_UNRELATED", "not-in-the-namespace"),
            ]),
            "claude-opus-4-8",
        )
        .unwrap();
        assert_eq!(table.model_for(CallSite::Verify), "claude-opus-4-8");
    }

    #[test]
    fn values_are_trimmed() {
        let table = RoutingTable::resolve(
            vars(&[("PARALLAX_MODEL_BULK", "  claude-haiku-4-5  ")]),
            "claude-opus-4-8",
        )
        .unwrap();
        assert_eq!(
            table.model_for(CallSite::ResearchExtract),
            "claude-haiku-4-5"
        );
    }

    #[test]
    fn every_site_routed_alike_collapses_to_one_client() {
        let mut pairs: Vec<(String, String)> = CallSite::ALL
            .iter()
            .map(|site| {
                (
                    format!("{PREFIX}{}", site.suffix()),
                    "claude-sonnet-5".to_string(),
                )
            })
            .collect();
        pairs.push(("PARALLAX_MODEL_BULK".into(), "claude-sonnet-5".into()));

        let table = RoutingTable::resolve(pairs, "claude-opus-4-8").unwrap();
        assert_eq!(table.distinct_models(), vec!["claude-sonnet-5".to_string()]);
    }

    #[test]
    fn ids_and_suffixes_are_unique_and_paired() {
        let ids: BTreeSet<&str> = CallSite::ALL.iter().map(|s| s.id()).collect();
        let suffixes: BTreeSet<&str> = CallSite::ALL.iter().map(|s| s.suffix()).collect();
        assert_eq!(ids.len(), CallSite::ALL.len());
        assert_eq!(suffixes.len(), CallSite::ALL.len());
        // The suffix is the id uppercased — the contract config.md documents.
        for site in CallSite::ALL {
            assert_eq!(site.suffix(), site.id().to_uppercase(), "{}", site.id());
        }
        for tier in Tier::ALL {
            assert_eq!(tier.suffix(), tier.id().to_uppercase());
        }
    }

    // `index` keys a fixed-size array in the client pool; if it drifts from
    // ALL, a call site silently gets another site's client.
    #[test]
    /// `ALL` must be written in declaration order, because [`CallSite::index`]
    /// is the discriminant and `ClientPool` keys its per-site array on it.
    ///
    /// Narrower than it used to be, and deliberately kept. Before 035 this
    /// guarded two hand-written orderings against each other; now the index is
    /// derived, so the only thing left that *can* disagree is `ALL` itself.
    /// A failure here means a call site would receive another site's client —
    /// silent, since both are valid clients, and visible only as an invocation
    /// record attributing cost to a model that never ran.
    #[test]
    fn all_is_in_declaration_order_so_index_keys_it_correctly() {
        for (position, site) in CallSite::ALL.iter().enumerate() {
            assert_eq!(site.index(), position, "{}", site.id());
        }
        // Every site is present exactly once: a duplicate would give two sites
        // the same index and leave a slot unreachable.
        let mut seen: Vec<usize> = CallSite::ALL.iter().map(|s| s.index()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), CallSite::ALL.len(), "ALL contains a duplicate");
        assert_eq!(
            seen.last(),
            Some(&(CallSite::ALL.len() - 1)),
            "index escapes ALL"
        );
    }

    #[test]
    fn only_research_extract_is_bulk() {
        let bulk: Vec<&str> = CallSite::ALL
            .iter()
            .filter(|s| s.tier() == Tier::Bulk)
            .map(|s| s.id())
            .collect();
        assert_eq!(bulk, vec!["research_extract"]);
    }
}
