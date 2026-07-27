//! Reading `config.rs`: what default each variable actually applies.
//!
//! Hand-written scanning of Rust source. Every assumption here is a way this
//! can go wrong later, so the rule throughout is that **nothing resolves from a
//! prefix of what was read**. An expression must be consumed entirely or it is
//! `Unresolvable`. A pre-merge review found four shapes returning confidently
//! wrong values — `RESEARCH_CONCURRENCY_MAX / 4` resolving to `32`, `3u32` to
//! `332`, a doc comment quoting an old constant beating the real declaration —
//! and each shared that one root cause. A wrong value is worse than the silent
//! skip this feature replaced: it balances every coverage invariant and then
//! fails accusing two *correct* documents, whose cheapest green is to copy the
//! fabricated value into both.

use super::{Fact, Resolution, EXCLUSIONS, SOURCES};

/// A fixture carrying every shape, so the resolver is tested against known
/// input rather than against whatever `config.rs` contains today.
///
/// Testing only against real configuration would mean the resolver passes now
/// and silently stops covering a shape the moment `config.rs` changes — the
/// failure being fixed, rebuilt one level up.
#[cfg(test)]
pub const FIXTURE: &str = r#"
    pub const FIXTURE_NAMED_NUM: usize = 4_096;
    pub const FIXTURE_NAMED_STR: &str = "fixture-model";
    let a = parse_env("FIXTURE_NUM_LITERAL", 12)?;
    let b = parse_env("FIXTURE_NAMED_NUMERIC", FIXTURE_NAMED_NUM)?;
    let c = std::env::var("FIXTURE_STR_LITERAL").unwrap_or_else(|_| "quiet".to_string());
    let d = std::env::var("FIXTURE_NAMED_STRING").unwrap_or_else(|_| FIXTURE_NAMED_STR.to_string());
    let e = parse_env("FIXTURE_UNREADABLE", compute_it())?;
"#;

/// Read every variable that carries a default out of `source`, resolving
/// constants against `constant_sources`.
///
/// Variables read with `.ok()` and no default never appear here: absence gates
/// a capability rather than selecting a value, so there is nothing to compare.
/// They remain subject to the documents' must-be-listed assertions.
#[must_use]
pub fn resolve_from(source: &str, constant_sources: &[(&str, &str)]) -> Vec<Fact> {
    let mut facts = Vec::new();
    for (name, expr) in extract_pairs(source) {
        if name == "PARALLAX_TEST_DEFINITELY_UNSET_KEY" {
            continue;
        }
        let resolution = if let Some((_, reason)) = EXCLUSIONS.iter().find(|(n, _)| *n == name) {
            Resolution::Excluded(reason)
        } else {
            classify(&expr, constant_sources)
        };
        facts.push(Fact { name, resolution });
    }
    facts
}

/// [`resolve_from`] over the real configuration.
#[must_use]
pub fn resolve() -> Vec<Fact> {
    resolve_from(SOURCES[0].1, SOURCES)
}

/// Every `(variable, default-expression)` pair in `source`, for both the
/// `parse_env` and `unwrap_or_else` shapes.
///
/// Returns the raw expression rather than a filtered value, so classification
/// happens in exactly one place. 036 filtered at extraction time, which is why
/// an expression with no digits vanished before anything could notice.
pub(super) fn extract_pairs(source: &str) -> Vec<(String, String)> {
    let source = &strip_line_comments(source);
    let mut pairs = Vec::new();

    // parse_env("NAME", <expr>) — the name may sit on its own line.
    let mut rest = source.as_str();
    while let Some(at) = rest.find("parse_env(") {
        rest = &rest[at + "parse_env(".len()..];
        let Some((name, after_name)) = quoted_first(rest) else {
            continue;
        };
        let Some(comma) = after_name.find(',') else {
            continue;
        };
        let Some(end) = matching_paren(&after_name[comma + 1..]) else {
            continue;
        };
        pairs.push((
            name,
            after_name[comma + 1..comma + 1 + end].trim().to_string(),
        ));
    }

    // env::var("NAME") followed, WITHIN THE SAME STATEMENT, by
    // `.unwrap_or_else(|_| <expr>)`.
    //
    // Bounding to the statement is load-bearing. A first version searched
    // forward to the next semicolon-newline, which on multi-line statements
    // ran past the end and paired a variable with a later, unrelated closure —
    // ANTHROPIC_API_KEY, which has no default at all, was reported as holding
    // the API base URL. A resolver that confidently reports the wrong value is
    // worse than the silent skip this feature replaces.
    let mut rest = source.as_str();
    while let Some(at) = rest.find("env::var(") {
        rest = &rest[at + "env::var(".len()..];
        let Some((name, after_name)) = quoted_first(rest) else {
            continue;
        };
        let Some(stmt_end) = statement_end(after_name) else {
            continue;
        };
        let stmt = &after_name[..stmt_end];
        let marker = "unwrap_or_else(|_|";
        let Some(u) = stmt.find(marker) else { continue };
        let tail = &stmt[u + marker.len()..];
        if let Some(end) = matching_paren(tail) {
            pairs.push((name, tail[..end].trim().to_string()));
        }
    }

    pairs
}

/// Offset of the `;` ending the statement that starts at `s`, ignoring any
/// inside parentheses or closures.
pub(super) fn statement_end(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ';' if depth <= 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// The first `"..."` in `s` and the text after it, skipping whitespace and
/// newlines so a multi-line call is read the same as a single-line one.
pub(super) fn quoted_first(s: &str) -> Option<(String, &str)> {
    let open = s.find('"')?;
    // Anything other than whitespace before the quote means this is not the
    // call's first argument.
    if s[..open].chars().any(|c| !c.is_whitespace()) {
        return None;
    }
    let after_open = &s[open + 1..];
    let close = after_open.find('"')?;
    Some((after_open[..close].to_string(), &after_open[close + 1..]))
}

/// Offset of the paren closing the expression starting at `s`, accounting for
/// nesting so `f(g(x))` is read whole rather than truncated at the first `)`.
pub(super) fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => return Some(i),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Turn a default expression into a comparable value, or record that it cannot
/// be read.
pub(super) fn classify(expr: &str, constant_sources: &[(&str, &str)]) -> Resolution {
    // A trailing comma is Rust's multi-line call style, not part of the
    // expression: `parse_env(\n  "X",\n  DEFAULT_X,\n)` yields `DEFAULT_X,`.
    let expr = expr
        .trim()
        .trim_end_matches('?')
        .trim_end_matches(',')
        .trim();

    if let Some(value) = literal_value(expr) {
        return Resolution::Resolved(value);
    }

    // Named constant, possibly path-qualified: `crate::client::anthropic::NAME`
    // is how `ANTHROPIC_API_BASE` refers to its default.
    //
    // **The whole expression must be the constant.** Resolving the first token
    // and discarding the rest is how `RESEARCH_CONCURRENCY_MAX / 4` resolved to
    // `32` — a wrong value that balanced every coverage invariant and then
    // failed accusing two correct documents, whose cheapest green is to copy
    // the fabricated number into both. Succeeding on a prefix of what was read
    // is strictly worse than the silent skip this module replaced.
    let bare = expr
        .trim_end_matches(".to_string()")
        .trim_end_matches(".to_owned()")
        .trim_end_matches(".into()")
        .trim();
    let (path, last_segment) = match bare.rsplit_once("::") {
        Some((path, last)) => (Some(path), last),
        None => (None, bare),
    };
    let is_whole_constant = !last_segment.is_empty()
        && last_segment
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && last_segment.chars().next().is_some_and(char::is_alphabetic);
    if is_whole_constant {
        // A path qualifier says which file to read. Honouring it is what makes
        // "resolves to whichever declaration it reaches first" unrepresentable
        // (FR-004a); enumerating the file set bounds the search but does not
        // disambiguate a name declared in two of them.
        match lookup_constant(last_segment, constant_sources, path) {
            ConstantLookup::Found(value) => return Resolution::Resolved(value),
            ConstantLookup::Unreadable(value) => {
                return Resolution::Unresolvable(format!(
                    "{expr} (constant declared as `{value}`, which this check cannot read)"
                ));
            }
            ConstantLookup::NotFound => {
                return Resolution::ConstantNotFound {
                    constant: last_segment.to_string(),
                    expr: expr.to_string(),
                };
            }
        }
    }

    Resolution::Unresolvable(expr.to_string())
}

/// Blank out `//`-to-end-of-line comments, preserving line structure.
///
/// A comment quoting a call — `// Was parse_env("MAX_RETRIES", 5) until 018.` —
/// otherwise mints a second fact for a real variable, or a whole fact for a
/// variable that does not exist. Both then fail naming the *documents* as
/// wrong. A `//` inside a string literal is left alone.
pub(super) fn strip_line_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let mut in_string = false;
        let mut escaped = false;
        let mut cut = line.len();
        let bytes: Vec<char> = line.chars().collect();
        for i in 0..bytes.len() {
            let c = bytes[i];
            if escaped {
                escaped = false;
                continue;
            }
            match c {
                '\\' if in_string => escaped = true,
                '"' => in_string = !in_string,
                '/' if !in_string && bytes.get(i + 1) == Some(&'/') => {
                    cut = line
                        .char_indices()
                        .nth(i)
                        .map_or(line.len(), |(byte_index, _)| byte_index);
                    break;
                }
                _ => {}
            }
        }
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

/// Value of a literal that is the **entire** expression, or `None`.
///
/// Never built by deleting characters. The digit-scavenging this replaces
/// turned `3u32` into `332`, `0x40000` into `040000`, and `1 << 18` into `118`
/// — each a confident wrong answer that passed every invariant.
pub(super) fn literal_value(expr: &str) -> Option<String> {
    // String literal, possibly converted.
    if let Some(inner) = expr.strip_prefix('"') {
        let close = inner.find('"')?;
        let rest = inner[close + 1..].trim();
        let rest = rest
            .trim_start_matches(".to_string()")
            .trim_start_matches(".to_owned()")
            .trim_start_matches(".into()")
            .trim();
        // Anything left over means the literal was only part of the expression.
        return rest.is_empty().then(|| inner[..close].to_string());
    }

    if expr == "true" || expr == "false" {
        return Some(expr.to_string());
    }

    // Integer literal: optional radix prefix, `_` separators, optional type
    // suffix. Parsed, then re-formatted from the parsed value.
    let body = expr.replace('_', "");
    let (radix, digits) = [("0x", 16), ("0b", 2), ("0o", 8)]
        .iter()
        .find_map(|(prefix, radix)| body.strip_prefix(prefix).map(|rest| (*radix, rest)))
        .unwrap_or((10, body.as_str()));
    let digits = [
        "usize", "isize", "u128", "i128", "u64", "i64", "u32", "i32", "u16", "i16", "u8", "i8",
    ]
    .iter()
    .find_map(|s| digits.strip_suffix(s))
    .unwrap_or(digits);
    if digits.is_empty() {
        return None;
    }
    u128::from_str_radix(digits, radix)
        .ok()
        .map(|n| n.to_string())
}

/// Outcome of looking for a constant's declaration.
pub(super) enum ConstantLookup {
    /// Declared in the searched files, with a value this check could read.
    Found(String),
    /// Declared, but its value is in a form this check cannot read. Distinct
    /// from [`Self::NotFound`] because "add the file to SOURCES" is the wrong
    /// remedy for a file already there.
    Unreadable(String),
    /// Not declared in any file searched — never *does not exist*, because this
    /// function only looked where it was told (FR-004b).
    NotFound,
}

/// Find `const NAME: _ = <value>;` in the enumerated sources.
///
/// When `path` is `Some`, only the source whose path matches its last module
/// segment is searched, so one name declared in two files cannot resolve to
/// whichever was reached first.
pub(super) fn lookup_constant(
    name: &str,
    sources: &[(&str, &str)],
    path: Option<&str>,
) -> ConstantLookup {
    let module = path.and_then(|p| p.rsplit("::").next());
    let mut unreadable = None;
    for (file, text) in sources {
        if let Some(module) = module {
            // `crate::client::anthropic::NAME` -> the source ending `anthropic.rs`.
            if !module.is_empty() && !file.ends_with(&format!("{module}.rs")) {
                continue;
            }
        }
        for line in text.lines() {
            // A declaration must open its line. `split_once("const ")` matched
            // a doc comment that quoted an old declaration, and comments are
            // scanned before the code they describe — so first-match-wins
            // handed back the historical value.
            let line = line.trim();
            let Some(decl) = ["pub const ", "const ", "pub(crate) const "]
                .iter()
                .find_map(|kw| line.strip_prefix(kw))
            else {
                continue;
            };
            let Some(decl) = decl.strip_prefix(name) else {
                continue;
            };
            if !decl.starts_with(':') {
                continue;
            }
            let Some((_, value)) = decl.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_end_matches(';').trim();
            if let Some(read) = literal_value(value) {
                return ConstantLookup::Found(read);
            }
            unreadable = Some(value.to_string());
        }
    }
    unreadable.map_or(ConstantLookup::NotFound, ConstantLookup::Unreadable)
}

/// Classify every environment read in `source` as default-bearing or not.
///
/// Returns `(default-bearing count, statements matching no known shape)`.
///
/// **Every read is classified by a positive rule, and one matching no rule is
/// returned rather than counted either way.** Counting only the two shapes
/// extraction recognises would make this independent of the argument parser but
/// not of the *call-site definition* — and the call-site definition is the half
/// that decides the population. A third shape such as
/// `.ok().and_then(|v| v.parse().ok()).unwrap_or(5_000)` would then be invisible
/// to extraction and to its own cross-check at once, which is the 036 defect
/// rebuilt one level up.
pub(super) fn classify_call_sites(source: &str) -> (usize, Vec<String>) {
    let source = strip_line_comments(source);
    let mut bearing = 0;
    let mut unrecognised = Vec::new();

    let mut rest = source.as_str();
    while let Some(at) = rest.find("env::var(") {
        rest = &rest[at + "env::var(".len()..];
        let stmt_end = statement_end(rest).unwrap_or(rest.len());
        let stmt = &rest[..stmt_end];
        // The helper's own body reads a key passed in, not a literal — it is one
        // read belonging to no variable.
        if !stmt.trim_start().starts_with('"') {
            continue;
        }
        if stmt.contains(".unwrap_or_else(") || stmt.contains(".unwrap_or(") {
            bearing += 1;
        } else if stmt.contains(".ok()") || stmt.contains(".is_ok()") || stmt.contains(".map_err(")
        {
            // Absence gates a capability or fails startup: no default to state.
        } else {
            unrecognised.push(stmt.split('\n').next().unwrap_or(stmt).trim().to_string());
        }
    }

    // `parse_env` call sites, minus the helper's declaration and the one
    // test-only key `resolve_from` skips, so both sides count the same
    // population.
    let parse_env_calls =
        source.matches("parse_env(").count() - source.matches("fn parse_env(").count();
    let skipped = source
        .matches("PARALLAX_TEST_DEFINITELY_UNSET_KEY")
        .count()
        .min(1);
    (bearing + parse_env_calls - skipped, unrecognised)
}

/// Every variable `source` reads that carries **no** default.
///
/// The complement of [`resolve_from`]: absence gates a capability or fails
/// startup rather than selecting a value, so there is nothing to compare — but
/// both documents must still list them, and that list has to be derived or a
/// new gate is checked by nothing.
#[must_use]
pub fn variables_without_defaults(source: &str, facts: &[Fact]) -> Vec<String> {
    let source = strip_line_comments(source);
    let mut out: Vec<String> = Vec::new();
    let mut rest = source.as_str();
    while let Some(at) = rest.find("env::var(") {
        rest = &rest[at + "env::var(".len()..];
        let Some((name, _)) = quoted_first(rest) else {
            continue;
        };
        if name.starts_with("PARALLAX_TEST")
            || facts.iter().any(|f| f.name == name)
            || out.contains(&name)
        {
            continue;
        }
        out.push(name);
    }
    out
}
