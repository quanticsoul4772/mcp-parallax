//! What `config.rs` actually applies, read out of its source (040).
//!
//! Every operator-facing document that states a configuration default is
//! checked against this module rather than against a list somebody wrote by
//! hand. It exists because three previous attempts each closed part of the loop
//! and reported it closed:
//!
//! - **034** listed every variable in `--help` and pinned five defaults by hand.
//! - **036** replaced those five with a scan that read defaults out of source —
//!   but only the ones written as a bare numeric literal. When its digit filter
//!   came up empty it moved on **without recording that it had skipped one**.
//! - **039** applied 036's scan to the README table, inheriting the blind spot
//!   with the code.
//!
//! Configuration writes defaults five ways — numeric literal, string literal,
//! boolean, named constant, path-qualified constant — and that scan handled
//! one. Setting both documents to a wrong `GROUNDED_VERIFY_MAX_BYTES` of
//! `999999` left every test green. (The spec said four; the fifth turned up as
//! an `Unresolvable`, which is the design working.)
//!
//! **The silent skip is the defect, not the missing coverage.** So
//! [`Resolution`] has no state meaning *skipped*: a default this module cannot
//! read is [`Resolution::Unresolvable`], which fails the suite and names the
//! variable. Handling the remaining shapes follows from that; it is not the
//! point.
//!
//! # Layout
//!
//! [`source`] reads `config.rs`; [`documents`] reads `--help` and the README
//! table; this file holds the types and the assertions both feed. The two sides
//! share nothing else, which is why the split is here and not elsewhere.
//!
//! The tests for those types and assertions are in `tests.rs` — they had grown
//! to 546 lines against 301 of production, which is a file organised around its
//! tests rather than its subject.
//!
//! **The rule that governs [`source`]: nothing resolves from a *prefix* of what
//! was read.** A confidently wrong value is worse than the skip this module
//! replaced — it balances every invariant, then fails accusing two *correct*
//! documents, and the cheapest way to green is to copy the fabricated value
//! into both.
//!
//! # Crate placement
//!
//! Declared from `main.rs`, so this belongs to the **binary** crate even though
//! its subject `config.rs` is a library module. That is forced: a `#[cfg(test)]`
//! item in the library is not visible to the binary's tests, because the
//! binary's test build links the library compiled *without* `cfg(test)`. A probe
//! module proved it — `cannot find probe_cfgtest in mcp_parallax`. Since
//! `--help` lives in the binary, the resolver must too. See
//! `specs/040-unresolvable-default-fails/plan.md`.

pub mod documents;
pub mod source;

pub use documents::{
    assert_no_contradictory_defaults, assert_no_phantom_defaults, stated_in_help, stated_in_table,
};
pub use source::{resolve, strip_comments, variables_without_defaults};

/// Source files this module reads to resolve a named constant.
///
/// **Enumerated, never searched** (FR-004a). Rust permits one constant name in
/// several modules, so a crate-wide search resolves to whichever declaration it
/// reaches first and compares a document against the wrong value — a wrong
/// answer that looks like success, which is this module's own failure mode. An
/// enumerated set makes that unrepresentable rather than merely detectable.
pub const SOURCES: &[(&str, &str)] = &[
    ("src/config.rs", include_str!("../config.rs")),
    (
        "src/client/anthropic.rs",
        include_str!("../client/anthropic.rs"),
    ),
];

/// Names that look like configuration variables but exist only to test
/// handling of absent or invalid values.
///
/// **These were three separate literals, and two of them were skip rules that
/// had to agree.** `resolve_from` skipped one name; the call-site counter
/// subtracted the same name in a different function; `--help`'s
/// variable-presence scan excluded both names in a third. Adding a second
/// fixture key and updating only one of those makes the counter and the
/// resolver disagree, and `COVERAGE_UNBALANCED` then fires naming a count
/// mismatch rather than the fixture nobody excluded — a failure pointing away
/// from its cause, inside the module written to stop exactly that.
///
/// Which of these is a `parse_env` call is **derived** rather than listed
/// again: a second list would reintroduce the problem one level up.
/// [`assert_test_keys_are_live`] fails if an entry outlives the fixture it
/// names.
pub const TEST_ONLY_KEYS: &[&str] = &[
    "PARALLAX_TEST_DEFINITELY_UNSET_KEY",
    "PARALLAX_MODEL_NOT_A_CALL_SITE",
];

/// Fail if a [`TEST_ONLY_KEYS`] entry names a fixture that no longer exists.
///
/// Without this an exclusion added for one test becomes permanent by
/// inattention, and a real variable could later take that name and be excused
/// from every document silently.
///
/// # Panics
///
/// Panics naming the stale entry.
pub fn assert_test_keys_are_live(sources: &[(&str, &str)]) {
    for key in TEST_ONLY_KEYS {
        assert!(
            sources.iter().any(|(_, text)| text.contains(key)),
            "TEST_KEY_STALE: TEST_ONLY_KEYS names `{key}`, which no source contains. Remove the \
             entry — an exclusion that outlives its fixture would silently excuse a real \
             variable that later takes the name."
        );
    }
}

/// Variables deliberately not resolved, each with the reason.
///
/// An entry is a decision someone made and wrote down, not a suppression. A
/// reason like "does not parse" is the defect this module exists to prevent
/// wearing the fix as a costume.
///
/// Empty today, and [`assert_exclusions_are_live`] fails if an entry outlives
/// the variable it excuses — that is how a suppression list quietly becomes
/// permanent.
pub const EXCLUSIONS: &[(&str, &str)] = &[(
    "VERIFY_MAX_CLAIM_CHARS",
    "the 002-era alias of INPUT_MAX_CHARS, read only when the canonical name is \
     unset. Both documents describe it as an alias rather than giving it a row \
     of its own, which is the right shape for a reader: it has no default a \
     reader can choose independently. Excluded from the document comparison, \
     not from checking — `the_alias_default_cannot_drift_from_the_canonical` \
     asserts the two defaults are equal, which is the property that would \
     actually break.",
)];

/// What examining one variable produced. **No state means *skipped*** — the
/// scan this replaces had one and did not report it. Every variable carrying a
/// default lands in exactly one of these, and two of the four fail the suite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// A comparable value was obtained.
    Resolved(String),
    /// Named in [`EXCLUSIONS`], carrying its reason.
    Excluded(&'static str),
    /// The shape itself could not be read — carries the expression text, so the
    /// failure message can quote it.
    Unresolvable(String),
    /// The shape *was* read: a named constant, whose declaration is in none of
    /// the files this module reads.
    ///
    /// Distinct from [`Self::Unresolvable`] because the remedy is the opposite
    /// one. "Teach the resolver this shape" is wrong advice here — the shape is
    /// already handled, and what is missing is the file from [`SOURCES`].
    ConstantNotFound {
        /// The identifier whose declaration was not found.
        constant: String,
        /// The full default expression, for quoting.
        expr: String,
    },
}

/// One configuration variable and what its default resolved to.
#[derive(Debug, Clone)]
pub struct Fact {
    /// Environment variable name.
    pub name: String,
    /// Outcome of resolving its default.
    pub resolution: Resolution,
}

/// The default shapes this module can read, for failure messages.
pub const HANDLED: &str =
    "numeric literal, string literal, boolean, named constant (possibly path-qualified)";

/// Fail if any variable's default could not be read (FR-001).
///
/// The whole feature. There is no path from here that examines fewer variables
/// than exist and still returns.
///
/// # Panics
///
/// Panics naming every unresolved variable, the expression found, the shapes
/// handled, and both remedies.
pub fn assert_all_resolved(facts: &[Fact]) {
    // Reported first and separately: its remedy is the opposite one, so folding
    // it into DEFAULT_UNRESOLVED would tell a contributor to teach the resolver
    // a shape it already handles.
    let missing: Vec<String> = facts
        .iter()
        .filter_map(|f| match &f.resolution {
            Resolution::ConstantNotFound { constant, expr } => {
                Some(format!("  `{}` <- {expr} (constant `{constant}`)", f.name))
            }
            _ => None,
        })
        .collect();
    let searched: Vec<&str> = SOURCES.iter().map(|(path, _)| *path).collect();
    assert!(
        missing.is_empty(),
        "CONSTANT_NOT_FOUND: these defaults name a constant declared in none of the files this \
         check reads.\n{}\n  searched: {}\n  If the constant lives elsewhere, add that file to \
         SOURCES. The shape is already handled — what is missing is the file.",
        missing.join("\n"),
        searched.join(", ")
    );

    let unresolved: Vec<String> = facts
        .iter()
        .filter_map(|f| match &f.resolution {
            Resolution::Unresolvable(found) => Some(format!("  `{}` <- {found}", f.name)),
            _ => None,
        })
        .collect();
    assert!(
        unresolved.is_empty(),
        "DEFAULT_UNRESOLVED: these defaults could not be read.\n{}\n  handled: {HANDLED}\n  \
         Either teach config_facts this shape, or add the variable to EXCLUSIONS with a \
         reason. It must not be skipped: a scan that skipped what it could not read is why \
         this module exists.",
        unresolved.join("\n")
    );
}

/// Fail if an exclusion has outlived the variable it excuses (FR-003).
///
/// # Panics
///
/// Panics naming the stale entry.
pub fn assert_exclusions_are_live(facts: &[Fact]) {
    assert_exclusions_are_live_against(facts, EXCLUSIONS);
}

/// [`assert_exclusions_are_live`] against an explicit list, so the stale case
/// is testable without adding a real exclusion to the production list.
///
/// # Panics
///
/// Panics naming the stale entry.
pub fn assert_exclusions_are_live_against(facts: &[Fact], exclusions: &[(&str, &str)]) {
    for (name, _) in exclusions {
        assert!(
            facts.iter().any(|f| f.name == *name),
            "EXCLUSION_STALE: EXCLUSIONS names `{name}`, which no longer carries a default. Remove the entry — a suppression that outlives its subject is how suppression becomes permanent."
        );
    }
}

/// Fail unless every variable is accounted for (FR-007).
///
/// An equation, not a floor. The `checked >= 8` threshold this replaces was
/// cleared by the literal-valued variables alone, so it measured effort while
/// three of four shapes went unexamined.
///
/// # Panics
///
/// Panics when the counts do not balance.
pub fn assert_coverage_balances(facts: &[Fact]) {
    let resolved = facts
        .iter()
        .filter(|f| matches!(f.resolution, Resolution::Resolved(_)))
        .count();
    let excluded = facts
        .iter()
        .filter(|f| matches!(f.resolution, Resolution::Excluded(_)))
        .count();
    assert_eq!(
        resolved + excluded,
        facts.len(),
        "COVERAGE_UNBALANCED: resolved {resolved} + excluded {excluded} != {} variables carrying a default.",
        facts.len()
    );
}

/// Fail if extraction found fewer variables than `source` contains (FR-007).
///
/// [`assert_coverage_balances`] compares the fact vector against itself, so an
/// extractor that dropped a variable shrinks both sides equally and it stays
/// balanced. That is exactly the early-exit case, and exactly what this feature
/// exists to catch — so the count it is compared against has to come from
/// somewhere the extractor cannot influence.
///
/// # Panics
///
/// Panics when the two counts disagree, naming both.
pub fn assert_extraction_is_complete(source: &str, facts: &[Fact]) {
    let (expected, unrecognised) = source::classify_call_sites(source);
    assert!(
        unrecognised.is_empty(),
        "UNKNOWN_CALL_SHAPE: these environment reads match no shape this check knows, so it \
         cannot tell whether they supply a default.\n  {}\n  Teach `classify_call_sites` the \
         shape. Counting one either way is the silent skip this module exists to remove, one \
         level up: a read invisible to both extraction and its own cross-check can have its \
         default documented as anything, forever.",
        unrecognised.join("\n  ")
    );
    assert_eq!(
        facts.len(),
        expected,
        "COVERAGE_UNBALANCED: extracted {} variables but the source contains {expected} calls \
         that supply a default. Extraction dropped one — a scan that examined fewer variables \
         than exist is the defect this module was written to make impossible.",
        facts.len()
    );
}

#[cfg(test)]
mod tests;
