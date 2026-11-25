//! The mandatory redactor in front of every Jev request (064, research D2).
//!
//! The client accepts only a [`RedactedState`], and the only way to make one
//! is through [`Redactor`], so no request can skip redaction. If redaction
//! fails, there is no state and nothing is sent.

use fancy_regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// What every removed span becomes.
pub const MARKER: &str = "[REDACTED]";

/// The largest state redacted, in characters. Larger input is refused before
/// any matching; it would exceed Jev's 64k-token request limit anyway.
pub const MAX_INPUT_CHARS: usize = 200_000;

/// Secret values shorter than this are not matched: a short value such as
/// `1` would redact ordinary text.
const MIN_SECRET_CHARS: usize = 8;

/// A high-entropy run is at least this long.
const MIN_ENTROPY_RUN: usize = 20;

/// And has at least this many bits of entropy per character.
const MIN_ENTROPY_BITS: f64 = 3.5;

/// Why redaction produced nothing.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RedactError {
    /// The input was over [`MAX_INPUT_CHARS`].
    #[error("state is {chars} characters, over the {max} limit")]
    TooLarge {
        /// The input's length in characters.
        chars: usize,
        /// The limit.
        max: usize,
    },
    /// A pattern failed to compile or to run.
    #[error("redaction pattern failed: {0}")]
    Pattern(String),
}

/// Text that has been through the redactor. Its fields are private, so it
/// can only come from [`Redactor`]:
///
/// ```compile_fail
/// let state = mcp_parallax::jev::redact::RedactedState {
///     text: String::new(),
///     json: None,
///     redactions: 0,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactedState {
    text: String,
    json: Option<serde_json::Value>,
    redactions: usize,
}

impl RedactedState {
    /// The redacted text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The redacted JSON tree, when the state was redacted from JSON; it is
    /// sent as an object so Jev's questions can point into it by path.
    #[must_use]
    pub const fn as_json(&self) -> Option<&serde_json::Value> {
        self.json.as_ref()
    }

    /// How many spans were replaced.
    #[must_use]
    pub const fn redactions(&self) -> usize {
        self.redactions
    }
}

/// The values of secret-shaped environment variables: names ending `_KEY`,
/// `_TOKEN` or `_SECRET`, or containing `PASSWORD`. Empty values are skipped.
pub fn secret_values(vars: impl Iterator<Item = (String, String)>) -> Vec<String> {
    vars.filter(|(name, value)| {
        let name = name.to_ascii_uppercase();
        !value.is_empty()
            && (name.ends_with("_KEY")
                || name.ends_with("_TOKEN")
                || name.ends_with("_SECRET")
                || name.contains("PASSWORD"))
    })
    .map(|(_, value)| value)
    .collect()
}

/// The pattern rules, in the order they run. A rule with a `v` group
/// replaces only that group; otherwise the whole match.
const PATTERNS: [&str; 7] = [
    // PEM blocks.
    r"(?s)-----BEGIN [A-Z0-9 ]+-----.*?-----END [A-Z0-9 ]+-----",
    // JWTs.
    r"\beyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
    // Authorization header values.
    r#"(?i)\bauthorization["']?\s*[:=]\s*["']?(?:(?:bearer|basic|token|digest)\s+)?(?P<v>[^\s,;"'\\]+)"#,
    // URL and connection-string passwords (user:password@).
    r#"(?i)\b[a-z][a-z0-9+.-]*://[^\s/:@"'\\]+:(?P<v>[^\s/@"'\\]+)@"#,
    // -p<value>.
    r#"(?:^|\s)-p(?P<v>[^\s"'\\-][^\s"'\\]*)"#,
    // -u user:pass.
    r#"(?:^|\s)-u\s+[^\s:"'\\]+:(?P<v>[^\s"'\\]+)"#,
    // Values of sensitive key names.
    r#"(?i)\b[\w.-]*(?:pass|pwd|secret|token|key|auth|credential|cookie|session)[\w.-]*["']?\s*[:=]\s*(?:(?:bearer|basic|token)\s+)?(?P<v>"[^"]*"|'[^']*'|[^\s;,&"'\\]+)"#,
];

const ENTROPY_PATTERN: &str = r"[A-Za-z0-9+/=_-]{20,}";

struct Rules {
    patterns: Vec<Regex>,
    entropy: Regex,
}

static RULES: LazyLock<Result<Rules, String>> = LazyLock::new(|| {
    let compile = |p: &str| Regex::new(p).map_err(|e| e.to_string());
    Ok(Rules {
        patterns: PATTERNS
            .iter()
            .map(|p| compile(p))
            .collect::<Result<_, _>>()?,
        entropy: compile(ENTROPY_PATTERN)?,
    })
});

/// The redactor: the patterns plus the secret values to remove verbatim.
#[derive(Debug, Clone)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    /// A redactor for these secret values. Values shorter than 8 characters
    /// are dropped; longer values are matched first.
    #[must_use]
    pub fn new(secrets: Vec<String>) -> Self {
        let mut secrets: Vec<String> = secrets
            .into_iter()
            .filter(|s| s.chars().count() >= MIN_SECRET_CHARS)
            .collect();
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
        Self { secrets }
    }

    /// A redactor for the secret values in this process's environment.
    #[must_use]
    pub fn from_env() -> Self {
        Self::new(secret_values(std::env::vars()))
    }

    /// Redact plain text.
    ///
    /// # Errors
    ///
    /// [`RedactError::TooLarge`] over [`MAX_INPUT_CHARS`];
    /// [`RedactError::Pattern`] when a pattern fails.
    pub fn redact(&self, text: &str) -> Result<RedactedState, RedactError> {
        check_size(text)?;
        let mut redactions = 0;
        let text = self.redact_text(text, &mut redactions)?;
        Ok(RedactedState {
            text,
            json: None,
            redactions,
        })
    }

    /// Redact every string in a JSON tree, keeping the JSON valid, and
    /// return it serialized.
    ///
    /// # Errors
    ///
    /// As [`Redactor::redact`], judged on the serialized size.
    pub fn redact_json(&self, value: &serde_json::Value) -> Result<RedactedState, RedactError> {
        check_size(&value.to_string())?;
        let mut redactions = 0;
        let value = self.redact_value(value, &mut redactions)?;
        Ok(RedactedState {
            text: value.to_string(),
            json: Some(value),
            redactions,
        })
    }

    fn redact_value(
        &self,
        value: &serde_json::Value,
        count: &mut usize,
    ) -> Result<serde_json::Value, RedactError> {
        use serde_json::Value;
        Ok(match value {
            Value::String(s) => Value::String(self.redact_text(s, count)?),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|v| self.redact_value(v, count))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| Ok((k.clone(), self.redact_value(v, count)?)))
                    .collect::<Result<_, RedactError>>()?,
            ),
            other => other.clone(),
        })
    }

    fn redact_text(&self, text: &str, count: &mut usize) -> Result<String, RedactError> {
        let rules = RULES
            .as_ref()
            .map_err(|e| RedactError::Pattern(e.clone()))?;
        let mut text = text.to_string();
        // Exact secrets first: a pattern rule that stops at a character inside
        // a secret (`;`, `,`, a quote) would otherwise leave its tail (064 B).
        for secret in &self.secrets {
            let found = text.matches(secret.as_str()).count();
            if found > 0 {
                text = text.replace(secret.as_str(), MARKER);
                *count += found;
            }
        }
        for re in &rules.patterns {
            text = replace(re, &text, count, |_| true)?;
        }
        replace(&rules.entropy, &text, count, is_high_entropy)
    }
}

fn check_size(text: &str) -> Result<(), RedactError> {
    let chars = text.chars().count();
    if chars > MAX_INPUT_CHARS {
        return Err(RedactError::TooLarge {
            chars,
            max: MAX_INPUT_CHARS,
        });
    }
    Ok(())
}

/// Replace each match (its `v` group when it has one) for which `redact`
/// holds. A span that already holds the marker is left alone and not counted.
fn replace(
    re: &Regex,
    text: &str,
    count: &mut usize,
    redact: fn(&str) -> bool,
) -> Result<String, RedactError> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for caps in re.captures_iter(text) {
        let caps = caps.map_err(|e| RedactError::Pattern(e.to_string()))?;
        let Some(span) = caps.name("v").or_else(|| caps.get(0)) else {
            continue;
        };
        if span.start() < last || span.as_str().contains(MARKER) || !redact(span.as_str()) {
            continue;
        }
        out.push_str(&text[last..span.start()]);
        out.push_str(MARKER);
        last = span.end();
        *count += 1;
    }
    out.push_str(&text[last..]);
    Ok(out)
}

/// A run of at least 20 characters with a letter, a digit and at least 3.5
/// bits of entropy per character.
fn is_high_entropy(run: &str) -> bool {
    let len = run.chars().count();
    if len < MIN_ENTROPY_RUN
        || !run.chars().any(|c| c.is_ascii_alphabetic())
        || !run.chars().any(|c| c.is_ascii_digit())
    {
        return false;
    }
    let mut counts: BTreeMap<char, usize> = BTreeMap::new();
    for c in run.chars() {
        *counts.entry(c).or_default() += 1;
    }
    #[allow(clippy::cast_precision_loss)] // lengths here are far below 2^52
    let entropy: f64 = counts
        .values()
        .map(|&n| {
            let p = n as f64 / len as f64;
            -p * p.log2()
        })
        .sum();
    entropy >= MIN_ENTROPY_BITS
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{secret_values, RedactError, Redactor, MARKER, MAX_INPUT_CHARS};

    /// Joins its parts, so no secret-shaped literal sits in this file.
    fn cat(parts: &[&str]) -> String {
        parts.concat()
    }

    fn redactor() -> Redactor {
        Redactor::new(vec![cat(&["sk-", "test", "Env", "Value", "123"])])
    }

    /// Redacts `text` and checks that `secret` is gone and a marker is in.
    fn assert_removed(text: &str, secret: &str) {
        let out = redactor().redact(text).unwrap();
        assert!(
            !out.as_str().contains(secret),
            "{secret} left in {}",
            out.as_str()
        );
        assert!(
            out.as_str().contains(MARKER),
            "no marker in {}",
            out.as_str()
        );
        assert!(out.redactions() >= 1);
    }

    #[test]
    fn exact_secret_environment_values_are_removed() {
        let secret = cat(&["sk-", "test", "Env", "Value", "123"]);
        assert_removed(&format!("the key is {secret} here"), &secret);
    }

    #[test]
    fn values_of_sensitive_key_names_are_removed() {
        for key in [
            "db_password",
            "pwd",
            "client_secret",
            "auth_token",
            "api_key",
            "cookie",
            "session",
        ] {
            let value = cat(&["hun", "ter", "2x"]);
            assert_removed(&format!("{key}={value}"), &value);
        }
    }

    #[test]
    fn command_line_password_forms_are_removed() {
        let p = cat(&["s3", "cr3t"]);
        assert_removed(&format!("mysql -u root -p{p} db"), &p);
        let up = cat(&["pa5", "5word"]);
        assert_removed(&format!("curl -u admin:{up} https://example.com"), &up);
    }

    #[test]
    fn url_user_info_is_removed() {
        let pw = cat(&["pa5", "5word"]);
        let url = format!("fetch https://user:{pw}@example.com/x");
        assert_removed(&url, &pw);
        assert!(redactor()
            .redact(&url)
            .unwrap()
            .as_str()
            .contains("example.com/x"));
    }

    #[test]
    fn authorization_headers_are_removed() {
        let token = cat(&["abc", "Def", "Ghi"]);
        assert_removed(&format!("Authorization: Bearer {token}"), &token);
        assert_removed(&format!("authorization: Basic {token}"), &token);
    }

    #[test]
    fn connection_strings_are_removed() {
        let pw = cat(&["db", "Pw", "99"]);
        assert_removed(&format!("mongodb+srv://admin:{pw}@cluster0/x"), &pw);
        assert_removed(&format!("Data Source=srv;User Id=u;Password={pw};"), &pw);
    }

    #[test]
    fn pem_blocks_are_removed() {
        let body = cat(&["MIIBOgIBAAJBAK", "j34GkxFhD90vcNLYLInFEX6Ppy1t"]);
        let pem = cat(&[
            "-----BEGIN ",
            "PRIVATE KEY-----\n",
            &body,
            "\n-----END ",
            "PRIVATE KEY-----",
        ]);
        assert_removed(&format!("key:\n{pem}\ndone"), &body);
    }

    #[test]
    fn jwts_are_removed() {
        let jwt = cat(&[
            "eyJ",
            "hbGciOiJIUzI1NiJ9",
            ".",
            "eyJ",
            "zdWIiOiIxIn0",
            ".",
            "c2lnbmF0",
            "dXJl",
        ]);
        assert_removed(&format!("token {jwt} end"), &jwt);
    }

    #[test]
    fn high_entropy_runs_of_twenty_or_more_are_removed() {
        let run = cat(&["x7Kq9", "Lm2Pz8", "Rt4Vw6", "Yb3Nc1"]);
        assert_removed(&format!("id {run} end"), &run);
    }

    #[test]
    fn ordinary_text_is_unchanged() {
        let text = "Run cargo test --all-features, then read src/jev/policy.rs:42. \
                    The test help_lists_every_variable_the_config_reads passed; \
                    internationalization works on 2026-11-24.";
        let out = redactor().redact(text).unwrap();
        assert_eq!(out.as_str(), text);
        assert_eq!(out.redactions(), 0);
    }

    /// Review finding A: a quoted key (JSON, Python dicts) still marks its
    /// value as secret.
    #[test]
    fn values_of_quoted_sensitive_keys_are_removed() {
        let value = cat(&["hun", "ter", "2x"]);
        assert_removed(&format!("{{\"password\": \"{value}\"}}"), &value);
        assert_removed(&format!("{{'api_key': '{value}'}}"), &value);
        let token = cat(&["abc", "Def", "Ghi"]);
        assert_removed(
            &format!("{{\"Authorization\": \"Bearer {token}\"}}"),
            &token,
        );
    }

    /// Review finding B: a secret holding a pattern stop character (`;`) is
    /// removed whole, not cut by a pattern first and missed by the exact step.
    #[test]
    fn an_exact_secret_with_a_stop_character_is_removed_whole() {
        let secret = cat(&["p@ss", ";word", "123"]);
        let out = Redactor::new(vec![secret.clone()])
            .redact(&format!("password={secret} rest"))
            .unwrap();
        assert!(!out.as_str().contains(";word123"), "{}", out.as_str());
        assert!(!out.as_str().contains("word123"), "{}", out.as_str());
    }

    #[test]
    fn the_count_matches_the_redactions_made() {
        let a = cat(&["sk-", "test", "Env", "Value", "123"]);
        let b = cat(&["hun", "ter", "2x"]);
        let c = cat(&["pa5", "5word"]);
        let text = format!("{a} and password={b} and https://u:{c}@example.com");
        assert_eq!(redactor().redact(&text).unwrap().redactions(), 3);
    }

    #[test]
    fn input_over_the_cap_is_an_error_and_produces_nothing() {
        let at_cap = "a ".repeat(MAX_INPUT_CHARS / 2);
        assert!(redactor().redact(&at_cap).is_ok());
        let over = format!("{at_cap}a");
        assert!(matches!(
            redactor().redact(&over),
            Err(RedactError::TooLarge { .. })
        ));
    }

    #[test]
    fn json_state_is_redacted_everywhere_in_the_tree() {
        let secret = cat(&["sk-", "test", "Env", "Value", "123"]);
        let state = serde_json::json!({
            "claim": "x",
            "sources": [{ "excerpts": [format!("leaked {secret}")] }],
        });
        let out = redactor().redact_json(&state).unwrap();
        assert!(!out.as_str().contains(&secret));
        assert_eq!(out.redactions(), 1);
    }

    #[test]
    fn json_state_keeps_its_tree_and_text_state_has_none() {
        let state = serde_json::json!({ "claim": "x", "sources": [] });
        let out = redactor().redact_json(&state).unwrap();
        assert_eq!(out.as_json(), Some(&state));
        assert!(redactor().redact("plain").unwrap().as_json().is_none());
    }

    #[test]
    fn secret_values_come_from_secret_shaped_names_only() {
        let vars = [
            ("TYPESAFE_API_KEY", "v1"),
            ("GH_TOKEN", "v2"),
            ("APP_SECRET", "v3"),
            ("DB_PASSWORD_FILE", "v4"),
            ("PATH", "not-a-secret"),
            ("EMPTY_KEY", ""),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        let mut found = secret_values(vars.into_iter());
        found.sort();
        assert_eq!(found, ["v1", "v2", "v3", "v4"]);
    }

    #[test]
    fn secret_values_shorter_than_eight_characters_are_not_matched() {
        let short = cat(&["123", "4567"]);
        let long = cat(&["1234", "5678"]);
        let r = Redactor::new(vec![short.clone(), long.clone()]);
        let kept = r.redact(&format!("pin {short} ok")).unwrap();
        assert_eq!(kept.as_str(), format!("pin {short} ok"));
        assert_eq!(kept.redactions(), 0);
        let gone = r.redact(&format!("pin {long} ok")).unwrap();
        assert!(!gone.as_str().contains(&long));
        assert_eq!(gone.redactions(), 1);
    }
}
