//! Wire types for the three checkpoint tools (MCP-side; data-model.md §3).
//!
//! These are not model-hop schemas — the flat+closed invariant does not
//! apply (the one model hop lives in [`crate::checkpoint::review`]). Verdict
//! subsets per boundary are enforced at construction: the gate can never
//! flag, the feedback boundaries can never hold.

use crate::checkpoint::{Signal, SignalKind, Verdict};
use serde::{Deserialize, Serialize};

/// `checkpoint_action` input.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[schemars(description = "`checkpoint_action` input.")]
pub struct CheckpointActionParams {
    /// The harness session identifier; must match the transcript.
    pub session_id: String,
    /// Path to the session's transcript (.jsonl). Validated and read as a
    /// bounded tail window.
    pub transcript_path: String,
    /// Name of the pending tool.
    pub tool_name: String,
    /// The pending tool's input, serialized; matched against the
    /// risk-pattern set and embedded for constraint recall.
    pub tool_input: String,
}

/// `checkpoint_batch` input.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[schemars(description = "`checkpoint_batch` input.")]
pub struct CheckpointBatchParams {
    /// The harness session identifier; must match the transcript.
    pub session_id: String,
    /// Path to the session's transcript (.jsonl). Validated and read as a
    /// bounded tail window.
    pub transcript_path: String,
}

/// `checkpoint_turn` input.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[schemars(description = "`checkpoint_turn` input.")]
pub struct CheckpointTurnParams {
    /// The harness session identifier; must match the transcript.
    pub session_id: String,
    /// Path to the session's transcript (.jsonl). Validated and read as a
    /// bounded tail window.
    pub transcript_path: String,
    /// The model's final message for the turn (as provided by the harness).
    pub final_message: String,
    /// True when this turn end follows a forced continuation; limits this
    /// evaluation to screening so continuation can never loop. Accepts a
    /// boolean or its string form ("true"/"false") — the harness's hook
    /// `${stop_hook_active}` substitution stringifies booleans (S1 round 2).
    #[serde(deserialize_with = "lenient_bool")]
    #[schemars(schema_with = "lenient_bool_schema")]
    pub continuation: bool,
}

/// Deserialize a bool from a JSON boolean or its string form — the harness's
/// `${path}` hook substitution produces strings for non-string payload
/// fields (S1 round 2 finding).
fn lenient_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct LenientBool;
    impl serde::de::Visitor<'_> for LenientBool {
        type Value = bool;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a boolean or \"true\"/\"false\"")
        }

        fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<bool, E> {
            Ok(value)
        }

        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<bool, E> {
            match value {
                "true" => Ok(true),
                "false" => Ok(false),
                other => Err(E::custom(format!(
                    "expected \"true\" or \"false\", got {other:?}"
                ))),
            }
        }
    }
    deserializer.deserialize_any(LenientBool)
}

fn lenient_bool_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["boolean", "string"],
        "description": "True when this turn end follows a forced continuation. Accepts a boolean or its string form (\"true\"/\"false\")."
    })
}

/// One fired signal as reported on the wire (the cooldown key stays
/// internal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct WireSignal {
    /// The detector that fired.
    pub kind: SignalKind,
    /// What it detected — specific, names the action/statements (SC-007).
    pub evidence: String,
}

impl From<&Signal> for WireSignal {
    fn from(signal: &Signal) -> Self {
        Self {
            kind: signal.kind,
            evidence: signal.evidence.clone(),
        }
    }
}

/// Claude Code hook-output mapping for a hold (S1: the harness interprets
/// the mcp_tool hook's result as hook output JSON — a hold must carry
/// `hookSpecificOutput.permissionDecision`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HookPermissionOutput {
    /// Always "PreToolUse".
    pub hook_event_name: String,
    /// Always "ask" — holds escalate to the user, never deny (FR-011).
    pub permission_decision: String,
    /// The assembled hold reason (same text as `message`).
    pub permission_decision_reason: String,
}

/// Shared checkpoint result (contracts/*.tool.json).
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct CheckpointResult {
    /// silence | flag | hold (per-boundary subsets — see constructors).
    pub verdict: Verdict,
    /// Assembled, model/user-facing message; null iff silence.
    pub message: Option<String>,
    /// The fired signals with their evidence.
    pub signals: Vec<WireSignal>,
    /// A flag was due but cooldown-suppressed (FR-010).
    pub suppressed: bool,
    /// Evaluation degraded (FR-008) — verdict is silence.
    pub fail_open: bool,
    /// Wall-clock evaluation time.
    pub latency_ms: u64,
    /// Harness hook mapping (S1): "block" on flag verdicts — the harness
    /// feeds `reason` back to the model. Absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// Harness hook mapping: the flag message, in the field the hook
    /// contract reads. Absent unless `decision` is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Harness hook mapping for holds (`permissionDecision: "ask"`).
    /// Absent otherwise.
    #[serde(rename = "hookSpecificOutput", skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookPermissionOutput>,
}

impl CheckpointResult {
    /// The default outcome: nothing fired, nothing delivered.
    #[must_use]
    pub const fn silence(latency_ms: u64) -> Self {
        Self {
            verdict: Verdict::Silence,
            message: None,
            signals: Vec::new(),
            suppressed: false,
            fail_open: false,
            latency_ms,
            decision: None,
            reason: None,
            hook_specific_output: None,
        }
    }

    /// A cooldown-suppressed outcome: signals fired but were already
    /// delivered; verdict downgrades to silence (FR-010).
    #[must_use]
    pub fn suppressed(signals: &[Signal], latency_ms: u64) -> Self {
        Self {
            verdict: Verdict::Silence,
            message: None,
            signals: signals.iter().map(WireSignal::from).collect(),
            suppressed: true,
            fail_open: false,
            latency_ms,
            decision: None,
            reason: None,
            hook_specific_output: None,
        }
    }

    /// A degraded outcome (FR-008): the evaluation failed; the session
    /// proceeds as if no checkpoint existed.
    #[must_use]
    pub const fn fail_open(latency_ms: u64) -> Self {
        Self {
            verdict: Verdict::Silence,
            message: None,
            signals: Vec::new(),
            suppressed: false,
            fail_open: true,
            latency_ms,
            decision: None,
            reason: None,
            hook_specific_output: None,
        }
    }

    /// A delivered flag (feedback boundaries only). Carries the harness
    /// hook mapping (`decision: "block"`) so the message reaches the model
    /// when invoked via a hook (S1).
    #[must_use]
    pub fn flag(message: String, signals: &[Signal], latency_ms: u64) -> Self {
        Self {
            verdict: Verdict::Flag,
            message: Some(message.clone()),
            signals: signals.iter().map(WireSignal::from).collect(),
            suppressed: false,
            fail_open: false,
            latency_ms,
            decision: Some("block".to_string()),
            reason: Some(message),
            hook_specific_output: None,
        }
    }

    /// A held action (gate boundary only). Carries the harness hook mapping
    /// (`permissionDecision: "ask"` — FR-011 escalate-only) so the hold
    /// pauses the action when invoked via a hook (S1).
    #[must_use]
    pub fn hold(message: String, signals: &[Signal], latency_ms: u64) -> Self {
        Self {
            verdict: Verdict::Hold,
            message: Some(message.clone()),
            signals: signals.iter().map(WireSignal::from).collect(),
            suppressed: false,
            fail_open: false,
            latency_ms,
            decision: None,
            reason: None,
            hook_specific_output: Some(HookPermissionOutput {
                hook_event_name: "PreToolUse".to_string(),
                permission_decision: "ask".to_string(),
                permission_decision_reason: message,
            }),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::checkpoint::Signal;

    #[test]
    fn silence_carries_no_message_and_no_signals() {
        let result = CheckpointResult::silence(3);
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(result.message.is_none());
        assert!(result.signals.is_empty());
        assert!(!result.suppressed && !result.fail_open);
    }

    #[test]
    fn fail_open_is_silence_with_the_marker() {
        let result = CheckpointResult::fail_open(7);
        assert_eq!(result.verdict, Verdict::Silence);
        assert!(result.fail_open);
        assert!(result.message.is_none());
    }

    // S1: the harness interprets the mcp_tool hook result as hook-output
    // JSON — flags must carry decision/reason, holds permissionDecision,
    // and silence must carry neither (a decision-less JSON is a no-op).
    #[test]
    fn hook_mapping_fields_follow_the_verdict() {
        let signals = vec![Signal::new(
            SignalKind::Repetition,
            "evidence".into(),
            "identity",
        )];
        let flag = serde_json::to_value(CheckpointResult::flag("msg".into(), &signals, 1)).unwrap();
        assert_eq!(flag["decision"], "block");
        assert_eq!(flag["reason"], "msg");
        assert!(flag.get("hookSpecificOutput").is_none());

        let hold = serde_json::to_value(CheckpointResult::hold("why".into(), &signals, 1)).unwrap();
        assert!(hold.get("decision").is_none());
        assert_eq!(hold["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(hold["hookSpecificOutput"]["permissionDecision"], "ask");
        assert_eq!(
            hold["hookSpecificOutput"]["permissionDecisionReason"],
            "why"
        );

        for quiet in [
            CheckpointResult::silence(1),
            CheckpointResult::fail_open(1),
            CheckpointResult::suppressed(&signals, 1),
        ] {
            let value = serde_json::to_value(quiet).unwrap();
            assert!(value.get("decision").is_none(), "{value}");
            assert!(value.get("reason").is_none(), "{value}");
            assert!(value.get("hookSpecificOutput").is_none(), "{value}");
        }
    }

    // S1 round 2: ${stop_hook_active} substitution stringifies booleans.
    #[test]
    fn continuation_accepts_boolean_and_string_forms() {
        let from =
            |v: serde_json::Value| -> Result<CheckpointTurnParams, _> { serde_json::from_value(v) };
        let base = |cont: serde_json::Value| {
            serde_json::json!({
                "session_id": "s", "transcript_path": "t.jsonl",
                "final_message": "m", "continuation": cont
            })
        };
        assert!(from(base(serde_json::json!(true))).unwrap().continuation);
        assert!(!from(base(serde_json::json!(false))).unwrap().continuation);
        assert!(from(base(serde_json::json!("true"))).unwrap().continuation);
        assert!(!from(base(serde_json::json!("false"))).unwrap().continuation);
        assert!(from(base(serde_json::json!("maybe"))).is_err());
    }

    #[test]
    fn flag_and_hold_carry_message_and_signals() {
        let signals = vec![Signal::new(
            SignalKind::Repetition,
            "`cargo test` invoked 4 times".into(),
            "bash cargo test",
        )];
        let flag = CheckpointResult::flag("msg".into(), &signals, 1);
        assert_eq!(flag.verdict, Verdict::Flag);
        assert_eq!(flag.signals.len(), 1);
        assert_eq!(flag.signals[0].kind, SignalKind::Repetition);

        let hold = CheckpointResult::hold("why".into(), &signals, 1);
        assert_eq!(hold.verdict, Verdict::Hold);
        assert_eq!(hold.message.as_deref(), Some("why"));
    }

    /// Contract sync (the 005 pattern): wire shapes match the three contract
    /// JSONs — params property names + required lists, and the result's
    /// field set.
    #[test]
    fn wire_types_match_the_contract_jsons() {
        let contracts_dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/specs/006-checkpoint-layer/contracts"
        );
        let load = |name: &str| -> serde_json::Value {
            let text = std::fs::read_to_string(format!("{contracts_dir}/{name}")).unwrap();
            serde_json::from_str(&text).unwrap()
        };
        let property_names = |schema: &serde_json::Value| -> Vec<String> {
            let mut names: Vec<String> = schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            names.sort();
            names
        };
        let schema_of = |schema: schemars::Schema| -> serde_json::Value {
            serde_json::to_value(schema).unwrap()
        };

        for (file, params_schema) in [
            (
                "checkpoint_action.tool.json",
                schema_of(schemars::schema_for!(CheckpointActionParams)),
            ),
            (
                "checkpoint_batch.tool.json",
                schema_of(schemars::schema_for!(CheckpointBatchParams)),
            ),
            (
                "checkpoint_turn.tool.json",
                schema_of(schemars::schema_for!(CheckpointTurnParams)),
            ),
        ] {
            let contract = load(file);
            assert_eq!(
                property_names(&contract["params"]),
                property_names(&params_schema),
                "{file}: params properties diverge from the contract"
            );
            let mut required: Vec<String> = contract["params"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            required.sort();
            let mut declared: Vec<String> = params_schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            declared.sort();
            assert_eq!(required, declared, "{file}: required diverges");

            // Result field set is shared across all three contracts.
            let result_schema = schema_of(schemars::schema_for!(CheckpointResult));
            assert_eq!(
                property_names(&contract["result"]),
                property_names(&result_schema),
                "{file}: result properties diverge from the contract"
            );
        }
    }
}
