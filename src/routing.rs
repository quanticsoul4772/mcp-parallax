//! Per-call-site model routing (018 D1).
//!
//! Each of the server's model call sites can run on a model chosen for the work
//! it does, instead of all of them sharing one `ANTHROPIC_MODEL`. Resolution is
//! most-specific-first: the call site's own setting, else its tier's setting,
//! else the server-wide default.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    /// no `Option`, no unreachable fallback branch. Kept in lockstep with
    /// `ALL` by `index_matches_all_order`.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Verify => 0,
            Self::Unstick => 1,
            Self::Diverge => 2,
            Self::Decide => 3,
            Self::Elicit => 4,
            Self::GroundedVerify => 5,
            Self::CheckTranslate => 6,
            Self::ResearchScope => 7,
            Self::ResearchExtract => 8,
            Self::ResearchVerify => 9,
            Self::ResearchSynthesize => 10,
            Self::CheckpointReview => 11,
        }
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

/// One call site's resolved model and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoute {
    /// The call site.
    pub site: CallSite,
    /// The model id it will use.
    pub model: String,
    /// Which setting supplied that model.
    pub source: RouteSource,
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
        let mut settings: Vec<(String, String)> = vars
            .into_iter()
            .filter(|(name, _)| name.starts_with(PREFIX))
            .collect();
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
                lookup(site.suffix()).map_or_else(
                    || {
                        lookup(site.tier().suffix()).map_or_else(
                            || ResolvedRoute {
                                site,
                                model: default_model.to_string(),
                                source: RouteSource::Default,
                            },
                            |model| ResolvedRoute {
                                site,
                                model,
                                source: RouteSource::Tier,
                            },
                        )
                    },
                    |model| ResolvedRoute {
                        site,
                        model,
                        source: RouteSource::Site,
                    },
                )
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
                })
                .collect(),
        }
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
    fn index_matches_all_order() {
        for (position, site) in CallSite::ALL.iter().enumerate() {
            assert_eq!(site.index(), position, "{}", site.id());
        }
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
