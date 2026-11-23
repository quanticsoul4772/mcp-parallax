//! The CLI's reply → a [`Completion`] or a classified error.
//!
//! 060 research.md D8; `contracts/claude-cli-call.md` §Reply classification.
//! Pure: reads documented fields of the one JSON object the CLI prints,
//! nothing else.

use crate::error::AppError;
use crate::traits::client::Completion;
use crate::traits::runner::CliOutput;
use serde::Deserialize;
use serde_json::Value;

/// How much of the CLI's stderr an error carries.
const STDERR_CAP: usize = 2_000;

/// The reply fields Parallax reads; every other field is ignored.
#[derive(Debug, Deserialize)]
struct Reply {
    subtype: Option<String>,
    structured_output: Option<Value>,
    #[serde(default)]
    usage: Usage,
    total_cost_usd: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    #[serde(default, rename = "input_tokens")]
    input: u64,
    #[serde(default, rename = "cache_creation_input_tokens")]
    cache_creation: u64,
    #[serde(default, rename = "cache_read_input_tokens")]
    cache_read: u64,
    #[serde(default, rename = "output_tokens")]
    output: u64,
}

/// Classify one finished CLI run.
///
/// # Errors
///
/// [`AppError::Client`] when stdout is not one JSON reply (with stderr,
/// capped); [`AppError::ValidationFailure`] when the reply carries no
/// structured output, including when the CLI's own schema retries or turn
/// cap ran out.
pub fn classify(output: &CliOutput) -> Result<Completion, AppError> {
    let reply: Reply = serde_json::from_str(output.stdout.trim()).map_err(|e| {
        let stderr: String = output.stderr.chars().take(STDERR_CAP).collect();
        AppError::Client(format!(
            "claude CLI printed no JSON reply (exit {:?}: {e}); stderr: {stderr}",
            output.exit_code
        ))
    })?;
    match reply.subtype.as_deref() {
        Some("error_max_turns") => {
            return Err(AppError::ValidationFailure(
                "claude CLI reached its turn cap before producing the structured output".into(),
            ));
        }
        Some("error_max_structured_output_retries") => {
            return Err(AppError::ValidationFailure(
                "claude CLI could not produce output matching the schema".into(),
            ));
        }
        _ => {}
    }
    let value = reply.structured_output.ok_or_else(|| {
        AppError::ValidationFailure("claude CLI reply has no structured output".into())
    })?;
    let usage = reply.usage;
    Ok(Completion {
        value,
        input_tokens: usage.input + usage.cache_creation + usage.cache_read,
        output_tokens: usage.output,
        reported_cost_micros: reply.total_cost_usd.and_then(to_micros),
    })
}

/// Dollars → whole millionths, rounded; `None` for a figure that is not a
/// finite, non-negative amount (unknown, never a made-up zero).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // range checked just above the cast
fn to_micros(usd: f64) -> Option<u64> {
    let micros = (usd * 1_000_000.0).round();
    // The range excludes NaN and both infinities as well as negatives.
    (0.0..1e18).contains(&micros).then_some(micros as u64)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::traits::runner::CliOutput;
    use serde_json::json;

    fn reply(body: &serde_json::Value) -> CliOutput {
        CliOutput {
            exit_code: Some(0),
            stdout: body.to_string(),
            stderr: String::new(),
        }
    }

    /// 060 T008: a success with structured output is the completion; input
    /// tokens count the cached ones too, and the estimate is kept in
    /// millionths of a dollar.
    #[test]
    fn a_success_with_structured_output_is_the_completion() {
        let out = classify(&reply(&json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "structured_output": {"methodology": "weigh"},
            "usage": {
                "input_tokens": 2,
                "cache_creation_input_tokens": 2407,
                "cache_read_input_tokens": 100,
                "output_tokens": 375
            },
            "total_cost_usd": 0.013_382_4
        })))
        .unwrap();
        assert_eq!(out.value, json!({"methodology": "weigh"}));
        assert_eq!(out.input_tokens, 2 + 2407 + 100);
        assert_eq!(out.output_tokens, 375);
        assert_eq!(out.reported_cost_micros, Some(13_382));
    }

    /// 060 T008: a reply without a cost figure records none, not zero.
    #[test]
    fn a_reply_without_a_cost_records_none() {
        let out = classify(&reply(&json!({
            "subtype": "success",
            "is_error": false,
            "structured_output": {},
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })))
        .unwrap();
        assert_eq!(out.reported_cost_micros, None);
        assert_eq!(out.input_tokens, 1);
    }

    /// 060 T008: no structured output, the CLI's own schema retries running
    /// out, and the turn cap are schema failures.
    #[test]
    fn missing_structured_output_and_its_error_subtypes_are_validation_failures() {
        let cases = [
            json!({"subtype": "success", "is_error": false, "result": "plain text"}),
            json!({"subtype": "error_max_structured_output_retries", "is_error": true}),
            json!({"subtype": "error_max_turns", "is_error": true}),
        ];
        for case in cases {
            let err = classify(&reply(&case)).unwrap_err();
            assert!(
                matches!(err, AppError::ValidationFailure(_)),
                "{case}: {err:?}"
            );
        }
        let turns = classify(&reply(
            &json!({"subtype": "error_max_turns", "is_error": true}),
        ))
        .unwrap_err()
        .to_string();
        assert!(turns.contains("turn"), "{turns}");
    }

    /// 060 T008: stdout that is not one JSON object is a client error carrying
    /// stderr, capped at 2,000 characters.
    #[test]
    fn unparseable_stdout_is_a_client_error_with_capped_stderr() {
        let out = CliOutput {
            exit_code: Some(1),
            stdout: "not json".into(),
            stderr: "e".repeat(5_000),
        };
        let err = classify(&out).unwrap_err();
        assert!(
            matches!(&err, AppError::Client(m)
                if m.contains(&"e".repeat(2_000)) && !m.contains(&"e".repeat(2_001))),
            "stderr missing or not capped: {err:?}"
        );
    }
}
