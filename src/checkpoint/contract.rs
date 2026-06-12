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
    /// evaluation to screening so continuation can never loop.
    pub continuation: bool,
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
        }
    }

    /// A delivered flag (feedback boundaries only).
    #[must_use]
    pub fn flag(message: String, signals: &[Signal], latency_ms: u64) -> Self {
        Self {
            verdict: Verdict::Flag,
            message: Some(message),
            signals: signals.iter().map(WireSignal::from).collect(),
            suppressed: false,
            fail_open: false,
            latency_ms,
        }
    }

    /// A held action (gate boundary only).
    #[must_use]
    pub fn hold(message: String, signals: &[Signal], latency_ms: u64) -> Self {
        Self {
            verdict: Verdict::Hold,
            message: Some(message),
            signals: signals.iter().map(WireSignal::from).collect(),
            suppressed: false,
            fail_open: false,
            latency_ms,
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
