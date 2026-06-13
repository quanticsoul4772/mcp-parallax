//! Extended-glob → regex translator (009 D1): the full grammar, incl. extglob.
//!
//! No Rust crate provides bash extglob, so each pattern is compiled to a
//! backtracking regex (`fancy-regex`, for the lookahead `!(...)` needs).
//! Matching is case-sensitive against a root-relative, `/`-separated path;
//! `*`/`**` match dotfiles; extglob groups are segment-scoped (FR-010).
//!
//! `!(p)` is context-dependent — its excluded extent reaches to the end of the
//! current segment — so when one is encountered the rest of the segment is
//! compiled first and folded into the negative lookahead (the picomatch
//! "expression after the closing paren" case).

// The translator builds a regex string; `format!` interpolation of the
// fragments reads far clearer than incremental `write!` calls.
#![allow(clippy::format_push_string)]

use crate::error::AppError;
use fancy_regex::Regex;
use std::iter::Peekable;
use std::str::Chars;

/// A compiled glob, matched against a root-relative (`/`-separated) path.
#[derive(Debug)]
pub struct GlobMatcher {
    re: Regex,
}

impl GlobMatcher {
    /// True when `rel_path` matches the pattern.
    #[must_use]
    pub fn matches(&self, rel_path: &str) -> bool {
        self.re.is_match(rel_path).unwrap_or(false)
    }
}

/// Compile an extended-glob pattern.
///
/// # Errors
///
/// [`AppError::InvalidInput`] for a malformed pattern (unbalanced group/class,
/// empty alternation, trailing escape, or a regex the engine rejects).
pub fn translate(pattern: &str) -> Result<GlobMatcher, AppError> {
    let (negate, body) = pattern
        .strip_prefix('!')
        .map_or((false, pattern), |rest| (true, rest));
    if body.is_empty() {
        return Err(malformed(pattern, "empty pattern"));
    }
    let mut chars = body.chars().peekable();
    let frag = compile_seq(&mut chars, &[], pattern)?;
    if chars.next().is_some() {
        return Err(malformed(pattern, "unbalanced ')' or '}'"));
    }
    let src = if negate {
        format!("^(?!(?:{frag})$).+$")
    } else {
        format!("^(?:{frag})$")
    };
    let re = Regex::new(&src).map_err(|e| malformed(pattern, &e.to_string()))?;
    Ok(GlobMatcher { re })
}

fn malformed(pattern: &str, why: &str) -> AppError {
    AppError::InvalidInput(format!("malformed glob pattern: {pattern} ({why})"))
}

/// Compile a run of glob tokens until end or a `stops` char (not consumed).
fn compile_seq(
    chars: &mut Peekable<Chars>,
    stops: &[char],
    pattern: &str,
) -> Result<String, AppError> {
    let mut out = String::new();
    while let Some(&c) = chars.peek() {
        if stops.contains(&c) {
            break;
        }
        chars.next();
        match c {
            '*' => match chars.peek() {
                Some('*') => {
                    chars.next();
                    // `**/` absorbs its slash so it matches zero or more whole
                    // segments (`src/**/x` matches `src/x`); bare `**` → `.*`.
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        out.push_str("(?:.*/)?");
                    } else {
                        out.push_str(".*");
                    }
                }
                Some('(') => {
                    chars.next();
                    let alts = compile_group(chars, '|', pattern)?;
                    out.push_str(&format!("(?:{alts})*"));
                }
                _ => out.push_str("[^/]*"),
            },
            '?' => {
                if chars.peek() == Some(&'(') {
                    chars.next();
                    let alts = compile_group(chars, '|', pattern)?;
                    out.push_str(&format!("(?:{alts})?"));
                } else {
                    out.push_str("[^/]");
                }
            }
            '@' if chars.peek() == Some(&'(') => {
                chars.next();
                let alts = compile_group(chars, '|', pattern)?;
                out.push_str(&format!("(?:{alts})"));
            }
            '+' if chars.peek() == Some(&'(') => {
                chars.next();
                let alts = compile_group(chars, '|', pattern)?;
                out.push_str(&format!("(?:{alts})+"));
            }
            '!' if chars.peek() == Some(&'(') => {
                chars.next();
                let excluded = compile_group(chars, '|', pattern)?;
                // The negation's extent reaches the end of this segment. Compile
                // the rest of the segment (suffix), then: the remaining segment
                // is structurally `[^/]*SUFFIX`, but must NOT be `EXCLUDED·SUFFIX`.
                let suffix = compile_seq_segment_rest(chars, stops, pattern)?;
                out.push_str(&format!(
                    "(?=[^/]*(?:{suffix})(?:/|$))(?!(?:{excluded})(?:{suffix})(?:/|$))[^/]*(?:{suffix})"
                ));
                // compile_seq_segment_rest consumed the rest of the segment.
            }
            '{' => {
                let alts = compile_group(chars, ',', pattern)?;
                out.push_str(&format!("(?:{alts})"));
            }
            '[' => out.push_str(&compile_class(chars, pattern)?),
            '/' => out.push('/'),
            '\\' => match chars.next() {
                Some(n) => out.push_str(&escape(n)),
                None => return Err(malformed(pattern, "trailing backslash")),
            },
            other => out.push_str(&escape(other)),
        }
    }
    Ok(out)
}

/// Compile the remainder of the current path segment (until `/`, end, or a
/// `stops` char), for folding into a preceding `!(...)`.
fn compile_seq_segment_rest(
    chars: &mut Peekable<Chars>,
    stops: &[char],
    pattern: &str,
) -> Result<String, AppError> {
    let mut seg_stops: Vec<char> = stops.to_vec();
    seg_stops.push('/');
    compile_seq(chars, &seg_stops, pattern)
}

/// Compile a group's alternatives, separated by `sep` (`,` brace / `|` extglob),
/// until the closing `)` (extglob) or `}` (brace).
fn compile_group(
    chars: &mut Peekable<Chars>,
    sep: char,
    pattern: &str,
) -> Result<String, AppError> {
    let close = if sep == ',' { '}' } else { ')' };
    let mut alts: Vec<String> = Vec::new();
    loop {
        let frag = compile_seq(chars, &[sep, close], pattern)?;
        alts.push(frag);
        match chars.next() {
            Some(c) if c == sep => {}
            Some(c) if c == close => break,
            _ => return Err(malformed(pattern, "unbalanced group")),
        }
    }
    if alts.len() == 1 && alts[0].is_empty() {
        return Err(malformed(pattern, "empty alternation"));
    }
    Ok(alts.join("|"))
}

/// Compile a `[...]` / `[!...]` character class.
fn compile_class(chars: &mut Peekable<Chars>, pattern: &str) -> Result<String, AppError> {
    let mut cls = String::from("[");
    if chars.peek() == Some(&'!') {
        chars.next();
        cls.push('^');
    }
    if chars.peek() == Some(&']') {
        // A leading ']' is a literal member.
        chars.next();
        cls.push_str("\\]");
    }
    let mut closed = false;
    while let Some(c) = chars.next() {
        if c == ']' {
            closed = true;
            break;
        }
        if c == '\\' {
            match chars.next() {
                Some(n) => {
                    cls.push('\\');
                    cls.push(n);
                }
                None => return Err(malformed(pattern, "trailing backslash in class")),
            }
        } else {
            cls.push(c);
        }
    }
    if !closed {
        return Err(malformed(pattern, "unbalanced '['"));
    }
    cls.push(']');
    Ok(cls)
}

/// Regex-escape a literal character.
fn escape(c: char) -> String {
    if "\\.+*?()|[]{}^$".contains(c) {
        format!("\\{c}")
    } else {
        c.to_string()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn m(pattern: &str, path: &str) -> bool {
        translate(pattern).unwrap().matches(path)
    }

    #[test]
    fn star_does_not_cross_slash_but_doublestar_does() {
        assert!(m("src/*.rs", "src/main.rs"));
        assert!(!m("src/*.rs", "src/a/main.rs"));
        assert!(m("src/**/*.rs", "src/a/b/main.rs"));
        assert!(m("src/**/*.rs", "src/main.rs"));
        assert!(!m("src/**/*.rs", "tests/main.rs"));
    }

    #[test]
    fn question_class_and_brace() {
        assert!(m("a?.rs", "ab.rs"));
        assert!(!m("a?.rs", "abc.rs"));
        assert!(m("[ab]z.rs", "az.rs"));
        assert!(!m("[!ab]z.rs", "az.rs"));
        assert!(m("src/{verify,grounded_verify}.rs", "src/verify.rs"));
        assert!(m(
            "src/{verify,grounded_verify}.rs",
            "src/grounded_verify.rs"
        ));
        assert!(!m("src/{verify,grounded_verify}.rs", "src/unstick.rs"));
        // nested brace
        assert!(m("{a,b{c,d}}.rs", "bd.rs"));
    }

    #[test]
    fn dotfiles_match_case_is_sensitive() {
        assert!(m(".github/**/*.yml", ".github/workflows/ci.yml"));
        assert!(m("*", ".gitignore"));
        assert!(!m("README", "readme")); // case-sensitive
    }

    #[test]
    fn extglob_alternation_and_repetition() {
        assert!(m("@(foo|bar).rs", "foo.rs"));
        assert!(!m("@(foo|bar).rs", "baz.rs"));
        assert!(m("a+(b).rs", "abbb.rs"));
        assert!(!m("a+(b).rs", "a.rs"));
        assert!(m("a*(b).rs", "a.rs"));
        assert!(m("a?(b).rs", "ab.rs"));
        assert!(m("a?(b).rs", "a.rs"));
        assert!(!m("a?(b).rs", "abb.rs"));
    }

    #[test]
    fn negation_extglob_whole_segment_and_with_suffix() {
        // whole-segment
        assert!(m("src/!(mod).rs", "src/verify.rs"));
        assert!(!m("src/!(mod).rs", "src/mod.rs"));
        // with a suffix after the closing paren (the hard case)
        assert!(m("tests/!(*_helpers).rs", "tests/api.rs"));
        assert!(!m("tests/!(*_helpers).rs", "tests/io_helpers.rs"));
        assert!(m("tests/!(*_helpers).rs", "tests/helpers.rs")); // no leading _ ⇒ not *_helpers
    }

    #[test]
    fn leading_bang_negates_whole_pattern() {
        assert!(m("!src/*.rs", "tests/a.rs"));
        assert!(!m("!src/*.rs", "src/a.rs"));
    }

    #[test]
    fn malformed_patterns_are_rejected_named() {
        assert!(translate("@(a").is_err()); // unbalanced extglob
        assert!(translate("{a,b").is_err()); // unbalanced brace
        assert!(translate("[abc").is_err()); // unbalanced class
        assert!(translate("@()").is_err()); // empty alternation
        assert!(translate("").is_err());
        // A bare '(' is a literal, not an error.
        assert!(m("a(b).rs", "a(b).rs"));
    }
}
