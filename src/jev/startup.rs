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
    }
}

const fn mode(settings: &JevSettings, site: JevSite) -> JevMode {
    match site {
        JevSite::TurnReview => settings.turn_review,
        JevSite::ResearchClaims => settings.research_claims,
    }
}

/// The startup lines: [`NO_KEY_LINE`] without a key; otherwise one line per
/// site in shadow, naming the host and the data sent. The key never appears.
#[must_use]
pub fn startup_lines(settings: &JevSettings) -> Vec<String> {
    if settings.api_key.is_none() {
        return vec![NO_KEY_LINE.to_string()];
    }
    JevSite::ALL
        .into_iter()
        .filter(|site| mode(settings, *site) == JevMode::Shadow)
        .map(|site| {
            format!(
                "Jev shadow on for {}: sends {} to api.typesafe.ai; answers are recorded only",
                site.as_str(),
                data_sent(site)
            )
        })
        .collect()
}

/// Whether to build the TypeSafe client: a key is set and some site is in
/// shadow.
#[must_use]
pub fn wants_client(settings: &JevSettings) -> bool {
    settings.api_key.is_some()
        && JevSite::ALL
            .into_iter()
            .any(|site| mode(settings, site) == JevMode::Shadow)
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
}
