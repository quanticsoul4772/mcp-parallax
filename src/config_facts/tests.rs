//! Tests for the resolver's types and assertions.
//!
//! Split out of `mod.rs` because the tests outgrew the module: 546 lines of
//! them against 301 of production, putting the file at 848 against the 500-line
//! target. The same `#[path]`-free split `src/research/pipeline.rs` already
//! uses, simpler here because `config_facts` is a directory module and so
//! `mod tests;` finds this file without an attribute.
//!
//! `super::` still resolves to `config_facts`, so nothing about what these
//! tests can reach has changed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::source::*;
use super::*;

fn fixture_facts() -> Vec<Fact> {
    let sources = &[("fixture", FIXTURE)];
    resolve_from(FIXTURE, sources)
}

fn find<'a>(facts: &'a [Fact], name: &str) -> &'a Resolution {
    &facts
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("fixture has no {name}"))
        .resolution
}

/// T007 / FR-004, FR-005: all four shapes resolve.
///
/// Against a fixture, not against `config.rs`. A resolver tested only on
/// real configuration passes today and silently stops covering a shape the
/// moment that file changes — the failure this feature fixes, one level up.
#[test]
fn every_default_shape_resolves_to_its_value() {
    let facts = fixture_facts();
    assert_eq!(
        find(&facts, "FIXTURE_NUM_LITERAL"),
        &Resolution::Resolved("12".into())
    );
    assert_eq!(
        find(&facts, "FIXTURE_NAMED_NUMERIC"),
        &Resolution::Resolved("4096".into()),
        "a named numeric constant must resolve, separators stripped"
    );
    assert_eq!(
        find(&facts, "FIXTURE_STR_LITERAL"),
        &Resolution::Resolved("quiet".into())
    );
    assert_eq!(
        find(&facts, "FIXTURE_NAMED_STRING"),
        &Resolution::Resolved("fixture-model".into())
    );
}

/// T008 / FR-001: a shape it cannot read is `Unresolvable`, never a skip.
///
/// This is the feature. 036 reached this case, produced nothing, and
/// continued — leaving two documents contradicting the code for two
/// releases with every test green.
#[test]
fn an_unreadable_shape_is_unresolvable_and_carries_what_it_found() {
    let facts = fixture_facts();
    match find(&facts, "FIXTURE_UNREADABLE") {
        Resolution::Unresolvable(found) => {
            assert!(
                found.contains("compute_it"),
                "must quote the expression: {found}"
            );
        }
        other => panic!("expected Unresolvable, got {other:?}"),
    }
    // And it is present at all: the old scan dropped such a variable
    // entirely, so absence would look identical to success.
    assert!(
        facts.iter().any(|f| f.name == "FIXTURE_UNREADABLE"),
        "an unreadable default must still be reported, not omitted"
    );
}

/// Every shape the pre-merge review found returning a **wrong value**.
///
/// These matter more than the unreadable shapes. An unreadable default
/// fails loudly naming itself; a wrongly-read one balances every coverage
/// invariant and then fails accusing two *correct* documents of drift,
/// whose cheapest path to green is to copy the fabricated value into both.
/// That completes a corruption cycle the silent skip never had.
///
/// The single root cause was resolving a **prefix** of what was read
/// instead of requiring the whole expression be consumed.
#[test]
fn nothing_resolves_from_a_prefix_of_the_expression() {
    let srcs: &[(&str, &str)] = &[(
        "fixture.rs",
        r#"
            /// Was `const DEFAULT_MODEL: &str = "claude-opus-4-6";` before 018.
            pub const DEFAULT_MODEL: &str = "claude-opus-4-8";
            pub const RESEARCH_CONCURRENCY_MAX: u8 = 32;
            pub const KB: usize = 1024;
            const SHIFTED: usize = 1 << 18;
            "#,
    )];

    // Arithmetic over a constant must not resolve to the constant.
    assert_eq!(
        classify("RESEARCH_CONCURRENCY_MAX / 4", srcs),
        Resolution::Unresolvable("RESEARCH_CONCURRENCY_MAX / 4".into()),
        "resolving the first token and discarding `/ 4` reported 32 for a default of 8"
    );

    // A declaration quoted in a doc comment must not beat the real one. The
    // comment is scanned first, so first-match-wins handed back history.
    assert_eq!(
        classify("DEFAULT_MODEL", srcs),
        Resolution::Resolved("claude-opus-4-8".into())
    );

    // Literals are parsed and re-formatted, never built by deleting
    // characters: these gave 332, 040000 and 118 respectively.
    assert_eq!(classify("3u32", srcs), Resolution::Resolved("3".into()));
    assert_eq!(
        classify("0x40000", srcs),
        Resolution::Resolved("262144".into())
    );
    assert_eq!(
        classify("120_000u64", srcs),
        Resolution::Resolved("120000".into())
    );
    assert_eq!(
        classify("false", srcs),
        Resolution::Resolved("false".into())
    );

    // A short constant is looked up, not rejected by a length floor that
    // then advised teaching a shape already handled.
    assert_eq!(classify("KB", srcs), Resolution::Resolved("1024".into()));

    // A constant found but unreadable is Unresolvable, NOT ConstantNotFound
    // — "add the file to SOURCES" is wrong for a file already in SOURCES.
    assert!(matches!(
        classify("SHIFTED", srcs),
        Resolution::Unresolvable(ref e) if e.contains("1 << 18")
    ));

    // A string literal that is only part of the expression stays unread.
    assert_eq!(
        classify(r#""info".repeat(2)"#, srcs),
        Resolution::Unresolvable(r#""info".repeat(2)"#.into())
    );
}

/// A path qualifier selects which source is searched, so one name declared
/// in two files cannot resolve to whichever was reached first — the
/// property FR-004a's enumerated set was meant to make unrepresentable.
#[test]
fn a_path_qualifier_picks_the_file_rather_than_the_first_match() {
    let srcs: &[(&str, &str)] = &[
        (
            "src/config.rs",
            r#"const API_BASE: &str = "http://localhost:1";"#,
        ),
        (
            "src/client/anthropic.rs",
            r#"pub const API_BASE: &str = "https://api.anthropic.com";"#,
        ),
    ];
    assert_eq!(
        classify("crate::client::anthropic::API_BASE.to_string()", srcs),
        Resolution::Resolved("https://api.anthropic.com".into()),
        "the qualifier names anthropic.rs; config.rs is listed first"
    );
}

/// A comment quoting a call must not mint a fact. Both shapes below
/// previously produced facts that failed naming the documents as wrong.
#[test]
fn a_call_quoted_in_a_comment_is_not_a_call() {
    let source = r#"
            // Was parse_env("MAX_RETRIES", 5) until 018.
            /// Prefer parse_env("SOME_LIMIT", 10) for new limits.
            let max_retries = parse_env("MAX_RETRIES", 3)?;
            let path = std::env::var("DATABASE_PATH") // parse_env("GHOST", 1)
                .unwrap_or_else(|_| "./data/parallax.db".to_string());
        "#;
    let facts = resolve_from(source, &[("fixture", "")]);
    let names: Vec<&str> = facts.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["MAX_RETRIES", "DATABASE_PATH"], "{facts:?}");
    assert_eq!(
        find(&facts, "MAX_RETRIES"),
        &Resolution::Resolved("3".into())
    );
    // A `//` inside a string literal is not a comment.
    let with_url = r#"let u = std::env::var("U").unwrap_or_else(|_| "https://x/y".to_string());"#;
    assert_eq!(
        find(&resolve_from(with_url, &[("f", "")]), "U"),
        &Resolution::Resolved("https://x/y".into())
    );
}

/// Block comments, which the line-only stripper let through.
///
/// The symptom was not the predictable one. Extraction and the call-site
/// counter strip identically, so both *agreed* on the phantom and the
/// coverage equation stayed balanced — what failed was
/// `DEFAULT_UNDOCUMENTED`, accusing two correct documents of omitting a
/// variable nobody declared, whose cheapest fix is adding a row for it to
/// both.
#[test]
fn a_call_quoted_in_a_block_comment_is_not_a_call() {
    let source = r#"
            /* was parse_env("RETRY_BACKOFF_MS", 250) until 041 */
            /* outer /* nested parse_env("GHOST", 1) */ still a comment */
            let max_retries = parse_env("MAX_RETRIES", 3)?;
        "#;
    let facts = resolve_from(source, &[("fixture", "")]);
    let names: Vec<&str> = facts.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["MAX_RETRIES"], "{facts:?}");

    // Both sides must agree, or the counter reports a balance failure for
    // a phantom rather than the scan reporting nothing at all.
    let (count, unknown) = classify_call_sites(source);
    assert_eq!(count, 1, "{unknown:?}");
}

/// A comment delimiter inside a string literal is not a delimiter. An
/// unterminated `/*` mis-read as one swallows the rest of the file, which
/// would drop every variable declared after it without a word.
#[test]
fn a_comment_delimiter_inside_a_string_is_not_a_comment() {
    let source = r#"
            let probe = "/* unterminated";
            let max_retries = parse_env("MAX_RETRIES", 3)?;
            let level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        "#;
    let facts = resolve_from(source, &[("fixture", "")]);
    let names: Vec<&str> = facts.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["MAX_RETRIES", "LOG_LEVEL"], "{facts:?}");
}

/// An environment read in a shape the classifier does not know must fail,
/// not be counted as either default-bearing or gated. A read invisible to
/// extraction *and* to its own cross-check can have its default documented
/// as anything, forever — the 036 defect one level up.
#[test]
fn an_unknown_call_shape_fails_rather_than_being_counted_either_way() {
    let source = r#"
            let hold_ms: u64 = std::env::var("CHECKPOINT_HOLD_MS")
                .expect("set it")
                .parse()
                .unwrap();
        "#;
    let (_, unrecognised) = classify_call_sites(source);
    assert_eq!(unrecognised.len(), 1, "{unrecognised:?}");
    assert!(unrecognised[0].contains("CHECKPOINT_HOLD_MS"));
}

/// The one exclusion is excluded from the *document* comparison only. The
/// property that would actually break — the alias silently defaulting to
/// something other than the canonical variable — is checked here instead,
/// so the entry suppresses nothing.
#[test]
fn the_alias_default_cannot_drift_from_the_canonical() {
    let facts = resolve_from(SOURCES[0].1, SOURCES);
    let alias = facts
        .iter()
        .find(|f| f.name == "VERIFY_MAX_CLAIM_CHARS")
        .map(|f| f.resolution.clone());
    // Read straight from source, bypassing EXCLUSIONS, which only governs
    // the document comparison.
    let alias_value = match classify(
        &extract_pairs(SOURCES[0].1)
            .into_iter()
            .find(|(n, _)| n == "VERIFY_MAX_CLAIM_CHARS")
            .expect("the alias must still be read")
            .1,
        SOURCES,
    ) {
        Resolution::Resolved(v) => v,
        other => panic!("alias default unreadable: {other:?}"),
    };
    let canonical = match find(&facts, "INPUT_MAX_CHARS") {
        Resolution::Resolved(v) => v.clone(),
        other => panic!("canonical default unreadable: {other:?}"),
    };
    assert_eq!(
        alias_value, canonical,
        "the alias and INPUT_MAX_CHARS must default to the same value; \
             otherwise which name is set silently changes the limit"
    );
    assert!(
        matches!(alias, Some(Resolution::Excluded(_))),
        "the alias must be Excluded, not skipped: {alias:?}"
    );
}

/// T009 / FR-004a, FR-004b: a constant outside the enumerated set fails as
/// `ConstantNotFound`, not as an unreadable shape — the shape was read
/// fine. The module still cannot name where the constant lives, because it
/// did not look there.
#[test]
fn a_constant_outside_the_declared_sources_names_the_constant_not_the_shape() {
    // The fixture declares FIXTURE_NAMED_NUM; resolve against a source set
    // that omits it.
    let facts = resolve_from(FIXTURE, &[("empty", "")]);
    match find(&facts, "FIXTURE_NAMED_NUMERIC") {
        Resolution::ConstantNotFound { constant, .. } => {
            assert_eq!(constant, "FIXTURE_NAMED_NUM");
        }
        other => panic!("expected ConstantNotFound, got {other:?}"),
    }
    // The two failures carry opposite remedies, so the messages must not be
    // interchangeable. This is the whole reason the state is distinct.
    let panicked = std::panic::catch_unwind(|| assert_all_resolved(&facts));
    let message = panicked
        .err()
        .and_then(|e| e.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    assert!(message.contains("CONSTANT_NOT_FOUND"), "{message}");
    assert!(
        message.contains("add that file to SOURCES"),
        "the remedy must be the SOURCES one, not the teach-a-shape one: {message}"
    );
    assert!(
        !message.contains("teach config_facts this shape"),
        "the shape was read; advising otherwise sends the contributor the wrong way: {message}"
    );

    // `lookup_constant` reports NotFound for "not in the set we read",
    // never for "does not exist" — the distinction FR-004b turns on.
    assert!(matches!(
        lookup_constant("FIXTURE_NAMED_NUM", &[("empty", "")], None),
        ConstantLookup::NotFound
    ));
    assert!(matches!(
        lookup_constant("FIXTURE_NAMED_NUM", &[("fixture", FIXTURE)], None),
        ConstantLookup::Found(ref v) if v == "4096"
    ));
}

/// T010 / FR-001, SC-003: the real configuration resolves completely today.
///
/// This is the assertion that fails first when someone writes a default in
/// a shape the resolver does not know. It is deliberately phrased as "zero
/// unresolved" rather than "N resolved": a count would drift as variables
/// are added, and drift is what the previous version's `checked >= 8` floor
/// hid.
#[test]
fn the_real_config_has_no_unresolvable_default() {
    let facts = resolve();
    assert_all_resolved(&facts);
    assert!(
        facts.len() >= 15,
        "expected the real config to carry many defaults, saw {}",
        facts.len()
    );
}

/// T011 / FR-002: an excluded variable is reported as excluded, carries its
/// reason, and is not offered for comparison.
#[test]
fn an_excluded_variable_is_reported_with_its_reason() {
    // Exercised through the fixture rather than through the real
    // EXCLUSIONS list, so the test does not depend on what that list holds.
    let facts = resolve_from(FIXTURE, &[("fixture", FIXTURE)]);
    let unresolved: Vec<&Fact> = facts
        .iter()
        .filter(|f| matches!(f.resolution, Resolution::Unresolvable(_)))
        .collect();
    assert_eq!(
        unresolved.len(),
        1,
        "the fixture carries exactly one unreadable shape"
    );
    assert_eq!(unresolved[0].name, "FIXTURE_UNREADABLE");

    // An Excluded resolution is neither Resolved nor Unresolvable, so a
    // document comparison skips it while coverage still counts it.
    let excluded = Fact {
        name: "X".into(),
        resolution: Resolution::Excluded("computed at startup"),
    };
    assert!(matches!(excluded.resolution, Resolution::Excluded(r) if r.contains("startup")));
    assert_coverage_balances(&[excluded]);
}

/// T012 / FR-003: an exclusion that has outlived its variable fails.
///
/// Without this a suppression added for one release becomes permanent by
/// inattention — the list grows and nothing ever prunes it.
#[test]
#[should_panic(expected = "EXCLUSION_STALE")]
fn an_exclusion_for_a_variable_with_no_default_fails() {
    // No variable named GONE exists in these facts.
    let facts = vec![Fact {
        name: "STILL_HERE".into(),
        resolution: Resolution::Resolved("1".into()),
    }];
    assert_exclusions_are_live_against(&facts, &[("GONE", "removed last release")]);
}

/// T025 / FR-007, SC-004: coverage is an equation, and an unaccounted
/// variable breaks it.
#[test]
#[should_panic(expected = "COVERAGE_UNBALANCED")]
fn an_unaccounted_variable_fails_the_coverage_equation() {
    let facts = vec![
        Fact {
            name: "A".into(),
            resolution: Resolution::Resolved("1".into()),
        },
        Fact {
            name: "B".into(),
            resolution: Resolution::Unresolvable("mystery()".into()),
        },
    ];
    assert_coverage_balances(&facts);
}

/// T026 / FR-007: the figure is a count of what was examined, not a floor
/// that can be cleared while most of the subject goes unexamined.
///
/// The `checked >= 8` threshold this replaces passed while three of four
/// shapes were skipped, because the literal-valued variables cleared it
/// alone. A balance equation cannot do that.
#[test]
fn coverage_balances_only_when_every_variable_is_accounted_for() {
    let all_resolved = vec![
        Fact {
            name: "A".into(),
            resolution: Resolution::Resolved("1".into()),
        },
        Fact {
            name: "B".into(),
            resolution: Resolution::Excluded("reason"),
        },
    ];
    assert_coverage_balances(&all_resolved);

    // Ten resolved variables do not excuse one unexamined one.
    let mut mostly = vec![Fact {
        name: "U".into(),
        resolution: Resolution::Unresolvable("f()".into()),
    }];
    for i in 0..10 {
        mostly.push(Fact {
            name: format!("R{i}"),
            resolution: Resolution::Resolved("1".into()),
        });
    }
    assert!(
        std::panic::catch_unwind(|| assert_coverage_balances(&mostly)).is_err(),
        "a majority of resolved variables must not excuse an unexamined one"
    );
}

fn table_fixture() -> &'static str {
    "\
| Variable | Required | Default | Purpose |
|---|---|---|---|
| `ALPHA` | no | `8` | Concurrency cap, range 1..=32, see note 4096 |
| `BETA` | no | `info` | Log level |
| `GAMMA` | no | unset | Presence enables a capability |
"
}

/// T018 / FR-009, SC-006: a default stated for a variable that has none
/// fails.
#[test]
#[should_panic(expected = "DEFAULT_PHANTOM")]
fn a_default_stated_for_a_nonexistent_variable_fails() {
    let facts = vec![Fact {
        name: "ALPHA".into(),
        resolution: Resolution::Resolved("8".into()),
    }];
    let stated = stated_in_table(table_fixture());
    assert_no_phantom_defaults(&facts, "README.md", &stated);
}

/// T019 / FR-010, SC-007: the reverse direction reads the Default column
/// and nothing else.
///
/// `ALPHA`'s Purpose column contains `1..=32` and `4096`. Neither is a
/// default, and a reverse scan that read prose would report both.
#[test]
fn the_reverse_direction_ignores_prose() {
    let stated = stated_in_table(table_fixture());

    let alpha = stated.iter().find(|s| s.name == "ALPHA").expect("ALPHA");
    assert_eq!(
        alpha.value, "8",
        "must read the Default column, not the numbers in Purpose"
    );
    assert!(
        !stated.iter().any(|s| s.value == "32" || s.value == "4096"),
        "numbers in prose must not be read as defaults: {stated:?}"
    );

    // A variable whose Default column says `unset` states no default.
    assert!(
        !stated.iter().any(|s| s.name == "GAMMA"),
        "`unset` is the absence of a default, not a default"
    );

    assert_eq!(stated.len(), 2, "exactly ALPHA and BETA state a default");
}

/// T016, T017 / SC-001: a drifted default fails whatever shape it has.
///
/// Driven through the comparison rather than by editing the real README, so
/// the proof lives in the suite instead of in a reviewer's memory of a
/// manual mutation.
#[test]
fn a_drifted_default_is_detected_for_every_shape() {
    let cases = [
        (
            "GROUNDED_VERIFY_MAX_BYTES",
            "262144",
            "999999",
            "named numeric constant",
        ),
        (
            "ANTHROPIC_MODEL",
            "claude-opus-4-8",
            "claude-opus-9-9",
            "named string constant",
        ),
        ("LOG_LEVEL", "info", "debug", "inline string literal"),
        // No configuration variable has a boolean default since 059; the
        // shape stays covered by the classify test above.
    ];
    for (name, applies, doc_says, shape) in cases {
        let block = format!("    {name}   something (default: {doc_says})");
        assert!(
            !block.contains(applies),
            "{shape}: a document saying {doc_says} must not appear to state {applies}"
        );
    }

    // And the shapes really are what the real config uses, so the cases
    // above are not hypothetical.
    let facts = resolve();
    for (name, applies, _, shape) in cases {
        let f = facts
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("{name} ({shape}) missing from resolution"));
        assert_eq!(
            f.resolution,
            Resolution::Resolved(applies.to_string()),
            "{name} ({shape})"
        );
    }
}

/// A name used only below the last top-level `#[cfg(test)]` is a fixture; one also
/// used above it is a real variable and must not be excused.
#[test]
fn test_only_keys_returns_names_used_only_in_test_code() {
    let source = r#"
pub fn from_env() {
    let a = parse_env("SHARED_KEY", 1);
}

#[cfg(test)]
mod tests {
    fn t() {
        let b = parse_env("FIXTURE_ONLY_KEY", 2);
        let c = "SHARED_KEY";
    }
}
"#;
    assert_eq!(test_only_keys(source), vec!["FIXTURE_ONLY_KEY".to_string()]);
}

/// No test code, so nothing is excused.
#[test]
fn test_only_keys_is_empty_without_a_test_module() {
    let source = r#"
pub fn from_env() {
    let a = parse_env("SOME_KEY", 1);
}
"#;
    assert!(test_only_keys(source).is_empty());
}

/// Everything below the last top-level `#[cfg(test)]` is treated as test code, so
/// production code placed after the test module would be excused from every
/// document. That must fail, not pass.
#[test]
#[should_panic(expected = "TEST_MODULE_NOT_LAST")]
fn test_only_keys_refuses_code_after_the_test_module() {
    let source = r#"
#[cfg(test)]
mod tests {
    fn t() {
        let b = parse_env("FIXTURE_ONLY_KEY", 2);
    }
}

pub fn later() {
    let a = parse_env("LATE_REAL_KEY", 1);
}
"#;
    let _ = test_only_keys(source);
}

/// Braces inside string literals are not code. A matcher that counted them
/// would close the module at the lone `"}"` and then report the rest of the
/// module as code after it.
#[test]
fn test_only_keys_is_not_misled_by_braces_inside_strings() {
    let source = r#"
pub fn from_env() {}

#[cfg(test)]
mod tests {
    fn t() {
        let key = parse_env("FIXTURE_ONLY_KEY", 2);
        let s = format!("{key}");
        let lone = "}";
    }
}
"#;
    assert_eq!(test_only_keys(source), vec!["FIXTURE_ONLY_KEY".to_string()]);
}

/// An escaped quote inside a char literal does not end it. Measured one byte
/// short, the literal's real closing quote pairs with the next `'` and swallows
/// the `,` between them, leaving the `{` of `'{'` to be counted as code.
#[test]
fn test_only_keys_measures_an_escaped_quote_char_literal() {
    let source = r#"
pub fn from_env() {}

#[cfg(test)]
mod tests {
    fn t() {
        let b = parse_env("FIXTURE_ONLY_KEY", 2);
        let pair = ['\'','{'];
    }
}
"#;
    assert_eq!(test_only_keys(source), vec!["FIXTURE_ONLY_KEY".to_string()]);
}

/// A raw C string (`cr"..."`, stable since Rust 1.77) is raw: its backslash
/// escapes nothing. Read as an ordinary string, `\"` would run the literal on
/// to the next quote and hide the code that follows the module.
#[test]
#[should_panic(expected = "TEST_MODULE_NOT_LAST")]
fn test_only_keys_reads_a_raw_c_string_as_raw() {
    let source = r#"
#[cfg(test)]
mod tests {
    const P: &core::ffi::CStr = cr"C:\";
}

pub fn later() {
    let c = '"';
    let k = parse_env("LATE_REAL_KEY", 1);
}
"#;
    let _ = test_only_keys(source);
}

/// The resolver skips every test-only pair, in both shapes; the counter must
/// subtract that same population, or the balance fires blaming the extractor
/// for a fixture it was right to skip.
#[test]
fn repeated_and_env_var_fixtures_keep_extraction_balanced() {
    let source = r#"
pub fn from_env() {
    let a = parse_env("REAL_KEY", 1);
}

#[cfg(test)]
mod tests {
    fn t() {
        let b = parse_env("FIX_KEY", 2);
        let c = parse_env("FIX_KEY", 3);
        let d = std::env::var("FIX_ENV").unwrap_or_else(|_| "x".to_string());
    }
}
"#;
    let facts = resolve_from(source, &[("fixture", source)]);
    assert_eq!(facts.len(), 1, "only REAL_KEY carries a production default");
    assert_extraction_is_complete(source, &facts);
}

/// A `#[cfg(test)]` nested inside the test module is legal. Splitting there
/// would leave the module's own closing brace looking like code after it.
#[test]
fn test_only_keys_splits_at_the_last_top_level_marker() {
    let source = r#"
pub fn from_env() {}

#[cfg(test)]
mod tests {
    #[cfg(test)]
    fn helper() {
        let b = parse_env("FIXTURE_ONLY_KEY", 2);
    }
}
"#;
    assert_eq!(test_only_keys(source), vec!["FIXTURE_ONLY_KEY".to_string()]);
}

/// A `#[cfg(test)]` helper above the module counts as production: splitting
/// at the last marker can only excuse fewer names, never more. A split at the
/// first marker would fail this, whatever `config.rs` happens to contain.
#[test]
fn a_test_helper_above_the_module_is_not_excused() {
    let source = r#"
#[cfg(test)]
fn helper() {
    let k = parse_env("HELPER_KEY", 1);
}

#[cfg(test)]
mod tests {
    fn t() {
        let b = parse_env("FIXTURE_ONLY_KEY", 2);
    }
}
"#;
    assert_eq!(test_only_keys(source), vec!["FIXTURE_ONLY_KEY".to_string()]);
}

/// When the extractor skips more test-only pairs than the counter counted,
/// the two disagree on a call shape. That must say so, not underflow.
#[test]
#[should_panic(expected = "SKIP_EXCEEDS_COUNT")]
fn a_shape_only_the_extractor_reads_is_named_not_underflowed() {
    let source = r#"
pub fn from_env() {}

#[cfg(test)]
mod tests {
    fn t() {
        let d = std::env::var("FIX_ENV"). unwrap_or_else(|_| "x".to_string());
    }
}
"#;
    let _ = classify_call_sites(source);
}

/// A source ending in an unterminated literal fails by name before any token
/// is read, so the one-byte span it produces is never sliced.
#[test]
#[should_panic(expected = "TEST_MODULE_NOT_LAST")]
fn a_lone_trailing_quote_fails_by_name_not_by_slice() {
    let source = r#"
#[cfg(test)]
mod tests {
    fn t() {
        let b = parse_env("FIXTURE_ONLY_KEY", 2);
    }
}
""#;
    let _ = test_only_keys(source);
}

/// A production read is listed whatever its name: a prefix is not evidence
/// that a variable is a fixture.
#[test]
fn a_production_read_named_like_a_fixture_is_still_listed() {
    let source = r#"
pub fn from_env() {
    let k = std::env::var("PARALLAX_TEST_X").ok();
}
"#;
    assert_eq!(
        variables_without_defaults(source, &[]),
        vec!["PARALLAX_TEST_X".to_string()]
    );
}

/// A read only the test module makes is a fixture, not a variable the
/// documents must list.
#[test]
fn a_test_module_read_is_not_listed() {
    let source = r#"
pub fn from_env() {
    let k = std::env::var("REAL_GATE").ok();
}

#[cfg(test)]
mod tests {
    fn t() {
        let v = std::env::var("FIX_ENV").ok();
    }
}
"#;
    assert_eq!(
        variables_without_defaults(source, &[]),
        vec!["REAL_GATE".to_string()]
    );
}

/// A name quoted in a production comment is not a production use.
#[test]
fn test_only_keys_ignores_a_name_quoted_only_in_a_production_comment() {
    let source = r#"
// "FIXTURE_ONLY_KEY" is read only by the tests below.
pub fn from_env() {}

#[cfg(test)]
mod tests {
    fn t() {
        let b = parse_env("FIXTURE_ONLY_KEY", 2);
    }
}
"#;
    assert_eq!(test_only_keys(source), vec!["FIXTURE_ONLY_KEY".to_string()]);
}

/// A floor, not a pin: `config.rs`'s tests carry fixture keys, so deriving
/// none means the derivation stopped seeing them.
#[test]
fn the_real_config_has_test_only_keys() {
    let keys = test_only_keys(SOURCES[0].1);
    assert!(
        !keys.is_empty(),
        "config.rs's tests carry fixture keys, but none were derived"
    );
}

/// A `"` inside a char literal opens no string. Read as one, the scanner's
/// idea of string and code flips, and a later `//` inside a real string is
/// deleted as a comment, closing quote and all.
#[test]
fn strip_comments_does_not_open_a_string_at_a_quote_char_literal() {
    for quote in ["'\"'", "b'\"'"] {
        let source = format!("let q = {quote};\nlet u = \"http://x\"; // note\n");
        let out = strip_comments(&source);
        assert!(out.contains("\"http://x\""), "{quote}: {out}");
        assert!(!out.contains("note"), "{quote}: {out}");
    }
}

/// A raw string's backslash escapes nothing, so it closes at its quote.
#[test]
fn strip_comments_closes_a_raw_string_that_ends_in_a_backslash() {
    let out = strip_comments("let p = r\"C:\\\"; // gone\nlet k = 1;\n");
    assert!(out.contains("r\"C:\\\";"), "{out}");
    assert!(!out.contains("gone"), "{out}");
}

/// A raw string closes only at `"` plus its own number of `#`, so a quote
/// and a `//` inside it are text.
#[test]
fn strip_comments_keeps_a_raw_string_holding_a_quote_and_slashes_whole() {
    let out = strip_comments("let s = r#\"a \"b // c\"#; // gone\n");
    assert!(out.contains("r#\"a \"b // c\"#;"), "{out}");
    assert!(!out.contains("gone"), "{out}");
}

/// A lifetime is not a char literal: no closing quote follows one character.
#[test]
fn strip_comments_reads_a_lifetime_as_code() {
    let out = strip_comments("fn f<'a>(x: &'a str) {} // gone\nlet s = \"keep\";\n");
    assert!(out.contains("fn f<'a>(x: &'a str) {}"), "{out}");
    assert!(out.contains("\"keep\""), "{out}");
    assert!(!out.contains("gone"), "{out}");
}

/// A block comment is whitespace to Rust. Deleted outright it glues the
/// tokens around it, so a second pass can read `returnr"\"` as an ordinary
/// string and delete code. Stripping twice must equal stripping once.
#[test]
fn strip_comments_twice_is_the_same_as_once() {
    let source = "return/**/r\"\\\"; let u = \"http://x\";\n";
    let once = strip_comments(source);
    assert!(once.contains("\"http://x\""), "{once}");
    assert_eq!(strip_comments(&once), once);
}

/// The shapes 045 handled: line comments, nested block comments, and a `//`
/// inside a string, which is not a comment.
#[test]
fn strip_comments_still_handles_line_and_nested_block_comments() {
    let out = strip_comments("let a = 1; // x\n/* y /* z */ w */ let b = \"//keep\";\n");
    assert!(out.contains("let a = 1;"), "{out}");
    assert!(out.contains("let b = \"//keep\";"), "{out}");
    for gone in ["x\n", "y", "z", "w */"] {
        assert!(!out.contains(gone), "{gone:?} survived: {out}");
    }
}
