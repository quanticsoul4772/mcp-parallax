//! What the server says about Jev at startup (064, contracts/config.md).
//!
//! One line when no key is set; one line per site in shadow, naming the
//! destination host and the data sent. The key never appears.

use crate::config::JevSettings;
use crate::traits::classifier::{JevMode, JevSite};

/// The one line logged when no key is set.
pub const NO_KEY_LINE: &str = "Jev off: TYPESAFE_API_KEY is not set, so no Jev site runs";

const fn data_sent(site: JevSite) -> &'static str {
    match site {
        JevSite::TurnReview => {
            "the redacted turn activity summary, transcript statement pairs and final \
             assistant text (never stored memories)"
        }
        JevSite::ResearchClaims => {
            "each research claim with redacted excerpts of its public sources (claims with \
             numbers, dates or flagged sources are never sent)"
        }
        JevSite::ResearchInjection => "kept research pages (URL and capped text, redacted)",
        JevSite::ToolRisk => {
            "pending tool calls as redacted projections: the Bash command with heredoc bodies \
             dropped, Write and Edit file paths, other tools' names and argument names"
        }
    }
}

const fn mode(settings: &JevSettings, site: JevSite) -> JevMode {
    match site {
        JevSite::TurnReview => settings.turn_review,
        JevSite::ResearchClaims => settings.research_claims,
        JevSite::ResearchInjection => settings.research_injection,
        JevSite::ToolRisk => settings.tool_risk,
    }
}

/// The startup lines: [`NO_KEY_LINE`] without a key; otherwise one line per
/// site in shadow or on, naming the host and the data sent. The key never
/// appears.
#[must_use]
pub fn startup_lines(settings: &JevSettings) -> Vec<String> {
    if settings.api_key.is_none() {
        return vec![NO_KEY_LINE.to_string()];
    }
    JevSite::ALL
        .into_iter()
        .filter_map(|site| {
            let effect = match mode(settings, site) {
                JevMode::Off => return None,
                JevMode::Shadow => "answers are recorded only",
                JevMode::On => match site {
                    JevSite::TurnReview => {
                        "the end-of-turn review is skipped when both answers are below their \
                         pinned thresholds and nothing blocks the skip"
                    }
                    JevSite::ResearchInjection => {
                        "pages at or above the pinned threshold carry an advisory warning"
                    }
                    // Config refuses `on` for tool risk (067); the wording
                    // is the generic one should that change.
                    JevSite::ResearchClaims | JevSite::ToolRisk => {
                        "answers are used against the site's pinned thresholds"
                    }
                },
            };
            // The shadow wording is 064's, unchanged.
            let lead = if mode(settings, site) == JevMode::Shadow {
                "Jev shadow on"
            } else {
                "Jev on"
            };
            Some(format!(
                "{lead} for {}: sends {} to api.typesafe.ai; {effect}",
                site.as_str(),
                data_sent(site)
            ))
        })
        .collect()
}

/// Whether to build the TypeSafe client: a key is set and some site is not
/// off.
#[must_use]
pub fn wants_client(settings: &JevSettings) -> bool {
    settings.api_key.is_some()
        && JevSite::ALL
            .into_iter()
            .any(|site| mode(settings, site) != JevMode::Off)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{startup_lines, wants_client, NO_KEY_LINE};
    use crate::config::{JevSettings, DEFAULT_JEV_MODEL};
    use crate::traits::classifier::JevMode;

    fn settings(key: Option<&str>, turn: JevMode, research: JevMode) -> JevSettings {
        JevSettings {
            api_key: key.map(String::from),
            model: DEFAULT_JEV_MODEL.into(),
            turn_review: turn,
            research_claims: research,
            research_injection: JevMode::Off,
            tool_risk: JevMode::Off,
        }
    }

    #[test]
    fn no_key_gives_exactly_one_line_and_no_client() {
        let s = settings(None, JevMode::Off, JevMode::Off);
        assert_eq!(startup_lines(&s), [NO_KEY_LINE]);
        assert_eq!(
            NO_KEY_LINE,
            "Jev off: TYPESAFE_API_KEY is not set, so no Jev site runs"
        );
        assert!(!wants_client(&s));
    }

    #[test]
    fn a_key_with_every_site_off_says_nothing_and_builds_no_client() {
        let s = settings(Some("ts-key-value-123"), JevMode::Off, JevMode::Off);
        assert!(startup_lines(&s).is_empty());
        assert!(!wants_client(&s));
    }

    #[test]
    fn each_shadowed_site_gets_a_line_naming_the_host_and_never_the_key() {
        let key = ["ts-", "secret", "-value-", "987"].concat();
        let s = settings(Some(&key), JevMode::Shadow, JevMode::Shadow);
        let lines = startup_lines(&s);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.contains("api.typesafe.ai")));
        assert!(lines.iter().all(|l| !l.contains(&key)));
        assert!(lines[0].contains("turn_review"));
        assert!(lines[1].contains("research_claims"));
        assert!(wants_client(&s));
    }

    // 066 T025: an `on` turn-review site describes the review skip, not the
    // injection warning (`on` is refused at startup in 066; the line is for
    // when it is allowed).
    #[test]
    fn the_turn_review_on_line_describes_the_skip() {
        let s = settings(Some("ts-key-value-123"), JevMode::On, JevMode::Off);
        let lines = startup_lines(&s);
        assert!(
            lines[0].starts_with("Jev on for turn_review"),
            "{}",
            lines[0]
        );
        assert!(
            lines[0].contains("end-of-turn review is skipped"),
            "{}",
            lines[0]
        );
        assert!(!lines[0].contains("advisory warning"), "{}", lines[0]);
    }

    #[test]
    fn the_injection_site_gets_a_line_in_shadow_and_on_and_on_builds_the_client() {
        for (mode, lead) in [
            (JevMode::Shadow, "Jev shadow on for"),
            (JevMode::On, "Jev on for"),
        ] {
            let mut s = settings(Some("ts-key-value-123"), JevMode::Off, JevMode::Off);
            s.research_injection = mode;
            let lines = startup_lines(&s);
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert!(lines[0].contains("research_injection"), "{}", lines[0]);
            assert!(lines[0].starts_with(lead), "{}", lines[0]);
            assert!(
                lines[0].contains("kept research pages (URL and capped text, redacted)"),
                "{}",
                lines[0]
            );
            assert!(lines[0].contains("api.typesafe.ai"), "{}", lines[0]);
            assert!(wants_client(&s));
        }
    }
}
