//! Which env-style names `config.rs` uses only in its test module.

use super::source::strip_comments;

/// The attribute that opens test-only code.
const TEST_MARKER: &str = "#[cfg(test)]";

/// Env-style names `source` uses only in its test module: quoted `UPPER_SNAKE`
/// tokens after the **last** top-level `#[cfg(test)]` and in no literal before
/// it.
///
/// Derived, so a fixture key cannot outlive its fixture and a real variable
/// cannot inherit a fixture's exemption. Splitting at the last marker rather
/// than the first means a test helper above the module counts as production:
/// a misjudged split can only excuse fewer names, which fails loudly in a
/// document check, never more, which would pass silently.
///
/// # Panics
///
/// `TEST_MODULE_NOT_LAST` when the last marker does not open a block that
/// closes at the end of the file — production code after it would otherwise be
/// read as test code and excused. Clippy's `items_after_test_module` catches
/// the same shape at the gate and is the backstop: this check trusts
/// `strip_comments`, which misreads a `'"'` char literal, a raw string ending
/// in a backslash (`r"C:\"`), and a raw string containing a quote. A key
/// quoted only inside a raw-string snippet is not derived, though the
/// extractor still reads it; a document check then fails naming it. It also
/// panics, wrongly, when the marked item's header holds a `;` inside brackets
/// before its `{` (`fn f() -> [u8; 2] {`).
#[must_use]
pub fn test_only_keys(source: &str) -> Vec<String> {
    let source = strip_comments(source);
    let spans = literal_spans(&source);
    let mut code = vec![true; source.len()];
    for &(start, end) in &spans {
        code[start..end].iter_mut().for_each(|c| *c = false);
    }
    // Only a marker outside every brace opens an item at file level; one
    // nested inside the test module belongs to it.
    let mut depth = 0usize;
    let mut top_level = vec![false; source.len()];
    for (at, &b) in source.as_bytes().iter().enumerate() {
        if !code[at] {
            continue;
        }
        top_level[at] = depth == 0;
        match b {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let Some(split) = source
        .match_indices(TEST_MARKER)
        .map(|(at, _)| at)
        .filter(|&at| code[at] && top_level[at])
        .last()
    else {
        return Vec::new();
    };
    assert_block_ends_file(&source, &code, split + TEST_MARKER.len());

    let token = |&(start, end): &(usize, usize)| &source[start + 1..end - 1];
    let above: std::collections::HashSet<&str> = spans
        .iter()
        .filter(|(start, _)| *start < split)
        .map(token)
        .collect();
    let mut keys: Vec<String> = spans
        .iter()
        .filter(|(start, _)| *start > split)
        .map(token)
        .filter(|t| looks_like_env(t) && !above.contains(t))
        .map(str::to_string)
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// The `--help` scan's notion of an env-variable name.
fn looks_like_env(token: &str) -> bool {
    token.len() >= 4
        && token
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && token.contains('_')
}

/// Fail unless the first block opened after `from` closes at the end of
/// `source`, counting only braces where `code` is true.
fn assert_block_ends_file(source: &str, code: &[bool], from: usize) {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut close = None;
    for at in from..bytes.len() {
        if !code[at] {
            continue;
        }
        match bytes[at] {
            b';' if depth == 0 => {
                close = Some(at);
                break;
            }
            b'{' => depth += 1,
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    close = Some(at);
                    break;
                }
            }
            _ => {}
        }
    }
    let ends_block = close.is_some_and(|at| bytes[at] == b'}');
    assert!(
        ends_block,
        "TEST_MODULE_NOT_LAST: the last {TEST_MARKER} does not open a block that closes \
         (an item ending in `;`, or unbalanced braces)"
    );
    let rest = source[close.map_or(bytes.len(), |at| at + 1)..].trim();
    assert!(
        rest.is_empty(),
        "TEST_MODULE_NOT_LAST: code follows the test module, and would be read as test \
         code: `{}`",
        rest.lines().next().unwrap_or(rest)
    );
}

/// Byte ranges, quotes included, of every string, raw-string and char literal.
///
/// A brace or quote inside a literal is not code. Lifetimes (`'a`) are told
/// from char literals by requiring the closing quote right after one char.
fn literal_spans(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let ident_before =
            at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        match bytes[at] {
            b'"' => {
                let mut end = at + 1;
                while end < bytes.len() && bytes[end] != b'"' {
                    end += if bytes[end] == b'\\' { 2 } else { 1 };
                }
                let end = (end + 1).min(bytes.len());
                spans.push((at, end));
                at = end;
            }
            // `br"…"` and `cr"…"` are raw too: the prefix letter sits just
            // before the `r`.
            b'r' if !ident_before || matches!(bytes[at - 1], b'b' | b'c') => {
                let hashes = bytes[at + 1..].iter().take_while(|&&b| b == b'#').count();
                if bytes.get(at + 1 + hashes) == Some(&b'"') {
                    let open = at + 1 + hashes;
                    let close = format!("\"{}", "#".repeat(hashes));
                    let end = source[open + 1..]
                        .find(&close)
                        .map_or(bytes.len(), |rel| open + 1 + rel + close.len());
                    // The span starts at the opening quote, so the token
                    // closure strips exactly one quote at each end.
                    spans.push((open, end - hashes));
                    at = end;
                } else {
                    at += 1;
                }
            }
            b'\'' => {
                let end = if bytes.get(at + 1) == Some(&b'\\') {
                    // Skip the escaped character itself: in `'\''` it is the
                    // quote that would otherwise end the search one byte early.
                    bytes
                        .get(at + 3..)
                        .and_then(|rest| rest.iter().position(|&b| b == b'\''))
                        .map(|rel| at + 3 + rel + 1)
                } else {
                    source[at + 1..].chars().next().and_then(|c| {
                        let after = at + 1 + c.len_utf8();
                        (bytes.get(after) == Some(&b'\'')).then_some(after + 1)
                    })
                };
                if let Some(end) = end {
                    spans.push((at, end));
                    at = end;
                } else {
                    at += 1;
                }
            }
            _ => at += 1,
        }
    }
    spans
}
