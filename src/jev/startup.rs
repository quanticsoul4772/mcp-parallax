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
            "pending tool calls as redacted projections (the Bash command with heredoc bodies \
             dropped; Write and Edit file paths; Read, Grep and Glob file_path, path and pattern \
             values; other tools' names and argument names)"
        }
        JevSite::TurnSignals => {
            "the redacted turn activity summary, transcript statement pairs and final assistant \
             text (claims with dates or numbers removed for unverified_fact), and that turn's \
             user text for pushback_reversal only with PARALLAX_JEV_SEND_USER_TEXT=true"
        }
    }
}

const fn mode(settings: &JevSettings, site: JevSite) -> JevMode {
    settings.mode(site)
}

/// The startup lines: [`NO_KEY_LINE`] without a key; otherwise one line per
/// site in shadow or on, naming the host and the data sent. The key never
/// appears.
#[must_use]
pub fn startup_lines(settings: &JevSettings) -> Vec<String> {
    if settings.api_key.is_none() {
        return vec![NO_KEY_LINE.to_string()];
    }
    let mut lines = JevSite::ALL
        .into_iter()
        .filter_map(|site| {
            let effect = match mode(settings, site) {
                JevMode::Off => return None,
                // 067: the tool-risk projection also goes to the label model.
                JevMode::Shadow if site == JevSite::ToolRisk => {
                    "answers are recorded only, and the same projection goes to the configured \
                     model backend for the label"
                }
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
                    JevSite::ResearchClaims | JevSite::ToolRisk | JevSite::TurnSignals => {
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
        .collect::<Vec<_>>();
    if !lines.is_empty() {
        lines.push(TERMS_LINE.to_string());
    }
    lines
}

/// TypeSafe's terms (roadmap §8), logged once after the site lines whenever
/// any site is on.
pub const TERMS_LINE: &str = "TypeSafe's terms: no training on inputs; retention as long as \
                              reasonably necessary; telemetry processed without restriction; \
                              some data kept indefinitely; zero retention for enterprise only.";

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
            turn_signals: JevMode::Off,
            send_user_text: false,
        }
    }

    // 067 review finding 11: the tool-risk line appears in shadow, names the
    // host, and is absent when the site is off.
    #[test]
    fn the_tool_risk_line_appears_in_shadow_and_not_when_off() {
        let mut s = settings(Some("k"), JevMode::Off, JevMode::Off);
        s.tool_risk = JevMode::Shadow;
        let lines = startup_lines(&s);
        // 068: the terms line follows the site lines.
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[1], super::TERMS_LINE);
        assert!(
            lines[0].starts_with("Jev shadow on for tool_risk:"),
            "{lines:?}"
        );
        assert!(lines[0].contains("api.typesafe.ai"), "{lines:?}");
        assert!(wants_client(&s));
        // 067 review finding 4: the line names every value sent and both
        // destinations.
        assert_eq!(
            lines[0],
            "Jev shadow on for tool_risk: sends pending tool calls as redacted projections \
             (the Bash command with heredoc bodies dropped; Write and Edit file paths; Read, \
             Grep and Glob file_path, path and pattern values; other tools' names and argument \
             names) to api.typesafe.ai; answers are recorded only, and the same projection goes \
             to the configured model backend for the label"
        );
        s.tool_risk = JevMode::Off;
        assert!(startup_lines(&s).is_empty());
        assert!(!wants_client(&s));
    }

    // 068 T002 (roadmap §8): TypeSafe's terms appear once, last, whenever
    // any site is on; never without a key or with every site off.
    #[test]
    fn the_terms_line_follows_the_site_lines_when_any_site_is_on() {
        let mut s = settings(Some("k"), JevMode::Off, JevMode::Off);
        assert!(startup_lines(&s).is_empty());
        s.turn_signals = JevMode::Shadow;
        let lines = startup_lines(&s);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("Jev shadow on for turn_signals:"));
        assert_eq!(
            lines[1],
            "TypeSafe's terms: no training on inputs; retention as long as reasonably \
             necessary; telemetry processed without restriction; some data kept indefinitely; \
             zero retention for enterprise only."
        );
        assert_eq!(lines[1], super::TERMS_LINE);
        let no_key = settings(None, JevMode::Shadow, JevMode::Off);
        assert_eq!(startup_lines(&no_key), [NO_KEY_LINE]);
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
        // 068: two site lines, then the terms line.
        assert_eq!(lines.len(), 3);
        assert!(lines[..2].iter().all(|l| l.contains("api.typesafe.ai")));
        assert!(lines.iter().all(|l| !l.contains(&key)));
        assert_eq!(lines[2], super::TERMS_LINE);
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
            // 068: the terms line follows the site line.
            assert_eq!(lines.len(), 2, "{lines:?}");
            assert_eq!(lines[1], super::TERMS_LINE);
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
