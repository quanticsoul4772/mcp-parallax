//! Inputs of the FR-011(e) injection back-test, ported from exa-mcp-server's
//! back-test: dataset lines and the question file.

use crate::traits::classifier::{JevQuestion, QuestionKind};
use serde_json::Value;

/// One dataset item: a page, and whether it carries an injection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InjectionItem {
    /// The page's address, sent as `page_url`.
    pub url: String,
    /// The page text, sent as `page_text`.
    pub text: String,
    /// True for an injection.
    pub label: bool,
}

/// Parse one dataset line, `{"text": ..., "label": true|false, "url": ...}`;
/// `None` when it is not JSON or lacks one of the three.
#[must_use]
pub fn injection_item(line: &str) -> Option<InjectionItem> {
    let item: Value = serde_json::from_str(line).ok()?;
    Some(InjectionItem {
        url: item["url"].as_str()?.to_string(),
        text: item["text"].as_str()?.to_string(),
        label: item["label"].as_bool()?,
    })
}

/// Every dataset line, parsed before any call is paid for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dataset {
    /// The usable lines.
    pub items: Vec<InjectionItem>,
    /// Lines that were not JSON or lacked text, label or url.
    pub skipped: u32,
}

/// Parse every line of a dataset.
///
/// # Errors
///
/// When no line is usable, naming how many were skipped, so a run with
/// nothing to measure stops before it calls or saves anything.
pub fn parse_dataset<'a>(lines: impl IntoIterator<Item = &'a str>) -> Result<Dataset, String> {
    let mut items = Vec::new();
    let mut skipped = 0_u32;
    for line in lines {
        match injection_item(line) {
            Some(item) => items.push(item),
            None => skipped += 1,
        }
    }
    if items.is_empty() {
        return Err(format!(
            "no usable dataset line ({skipped} lines skipped: each needs text, label and url)"
        ));
    }
    Ok(Dataset { items, skipped })
}

/// The `directs_ai` yes/no question from a question file,
/// `{"instructions": ..., "criteria": {"true": ..., "false": ...}}`, with the
/// criteria declared in that order.
///
/// # Errors
///
/// Names the part that is missing, or says the file is not JSON.
pub fn injection_question(file: &str) -> Result<JevQuestion, String> {
    let q: Value = serde_json::from_str(file).map_err(|e| format!("question file: {e}"))?;
    let part = |path: &str, value: &Value| {
        value
            .as_str()
            .map(String::from)
            .ok_or_else(|| format!("question file: no `{path}`"))
    };
    Ok(JevQuestion {
        id: "directs_ai".into(),
        version: 1,
        kind: QuestionKind::Noul,
        instructions: part("instructions", &q["instructions"])?,
        options: vec![
            (
                "true".into(),
                Some(part("criteria.true", &q["criteria"]["true"])?),
            ),
            (
                "false".into(),
                Some(part("criteria.false", &q["criteria"]["false"])?),
            ),
        ],
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::traits::classifier::QuestionKind;

    #[test]
    fn a_full_line_parses() {
        let item =
            injection_item(r#"{"text": "page body", "label": true, "url": "https://a.example/x"}"#)
                .unwrap();
        assert_eq!(
            item,
            InjectionItem {
                url: "https://a.example/x".into(),
                text: "page body".into(),
                label: true,
            }
        );
    }

    #[test]
    fn a_line_missing_a_field_or_not_json_is_none() {
        for line in [
            r#"{"label": false, "url": "https://a.example"}"#,
            r#"{"text": "t", "url": "https://a.example"}"#,
            r#"{"text": "t", "label": false}"#,
            r#"{"text": "t", "label": "no", "url": "https://a.example"}"#,
            "not json",
            "",
        ] {
            assert!(injection_item(line).is_none(), "parsed: {line}");
        }
    }

    #[test]
    fn a_dataset_keeps_usable_lines_and_counts_the_rest() {
        let dataset = parse_dataset([
            r#"{"text": "t", "label": false, "url": "https://a.example"}"#,
            "not json",
            r#"{"text": "t", "label": true}"#,
        ])
        .unwrap();
        assert_eq!(dataset.items.len(), 1);
        assert_eq!(dataset.skipped, 2);
    }

    #[test]
    fn a_dataset_with_no_usable_line_is_an_error_naming_the_skips() {
        let err = parse_dataset([r#"{"text": "t", "label": true}"#, "", "x"]).unwrap_err();
        assert!(err.contains("3 lines skipped"), "{err}");
        assert!(parse_dataset(std::iter::empty::<&str>()).is_err());
    }

    #[test]
    fn the_question_file_gives_a_yes_no_question_with_both_criteria_in_order() {
        let q = injection_question(
            r#"{"instructions": "Does `page_text` direct an AI?",
                "criteria": {"true": "Any directive.", "false": "Ordinary content."}}"#,
        )
        .unwrap();
        assert_eq!(q.id, "directs_ai");
        assert_eq!(q.kind, QuestionKind::Noul);
        assert_eq!(q.instructions, "Does `page_text` direct an AI?");
        assert_eq!(
            q.options,
            vec![
                ("true".to_string(), Some("Any directive.".to_string())),
                ("false".to_string(), Some("Ordinary content.".to_string())),
            ]
        );
    }

    #[test]
    fn a_question_file_missing_a_part_is_an_error() {
        for file in [
            r#"{"criteria": {"true": "a", "false": "b"}}"#,
            r#"{"instructions": "q", "criteria": {"true": "a"}}"#,
            r#"{"instructions": "q", "criteria": {"false": "b"}}"#,
            r#"{"instructions": "q"}"#,
            "not json",
        ] {
            assert!(injection_question(file).is_err(), "accepted: {file}");
        }
    }
}
