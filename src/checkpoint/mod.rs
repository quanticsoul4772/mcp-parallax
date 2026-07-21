//! The checkpoint layer: harness-triggered correctives (the watchdog
//! re-grounded for MCP — `WATCHDOG_LAYER.md`, 2026-06-12 amendment).
//!
//! The harness's hooks are the sensor/actuator plane; this module is the
//! brain. Three boundaries — pre-action (gate), post-batch (feedback),
//! end-of-turn (feedback) — decide from cheap deterministic screening, with
//! at most one blind model pass at end of turn. Verdicts and all wording are
//! server-assembled; the layer never rewrites anything and fails open.

pub mod contract;
pub mod gate;
pub mod preference;
pub mod review;
pub mod run;
pub mod screen;
pub mod trajectory;

use serde::{Deserialize, Serialize};

/// Max transcript entries read per evaluation (bounded window — FR-009/D3).
pub const WINDOW_ENTRIES: usize = 200;

/// Max transcript bytes read per evaluation (tail window — D3).
pub const WINDOW_BYTES: u64 = 2 * 1024 * 1024;

/// Repetition lookback in tool batches (D5).
pub const WINDOW_BATCHES: u32 = 10;

/// Identical normalized actions within the window that constitute a loop
/// (US1-AS1).
pub const REPEAT_THRESHOLD: usize = 4;

/// Consecutive failures of the same normalized action that constitute a
/// repeated failure (US1-AS2).
pub const FAILURE_THRESHOLD: usize = 3;

/// Hard pre-action budget in milliseconds; timeout → fail-open (FR-009).
pub const GATE_BUDGET_MS: u64 = 500;

/// Minimum cosine relevance for a constraint memory to hold an action (D4).
///
/// Validated by acceptance run 1 (2026-06-12: 3/3 seeded holds, 0/60 false
/// holds on benign risk-matched actions) — moves only with new measurement.
pub const GATE_RELEVANCE_TAU: f32 = 0.55;

/// Minimum cosine relevance for a memory to become a review candidate (D6).
pub const REVIEW_RECALL_FLOOR: f32 = 0.45;

/// Cap on candidate pairs sent to the review hop (D6).
pub const REVIEW_CANDIDATES_MAX: usize = 4;

/// Flag suppression window in milliseconds (FR-010).
pub const COOLDOWN_WINDOW_MS: i64 = 1_800_000;

/// Built-in risk patterns for the pre-action gate (FR-013 defaults:
/// consequential shell commands and writes).
///
/// Matched case-insensitively as substrings of `tool_name + " " +
/// tool_input`. Extended (never replaced) by `CHECKPOINT_GATE_PATTERNS`.
pub const GATE_RISK_PATTERNS: &[&str] = &[
    "deploy",
    "push",
    "publish",
    "release",
    "rm -",
    "rmdir",
    "del /",
    "delete",
    "drop table",
    "drop database",
    "truncate",
    "migrate",
    "terraform",
    "kubectl",
    "force",
    "reset --hard",
];

/// One harness boundary (the `boundary` column on checkpoint records).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    /// Pre-action gate (`checkpoint_action`).
    Action,
    /// Post-tool-batch feedback (`checkpoint_batch`).
    Batch,
    /// End-of-turn review (`checkpoint_turn`).
    Turn,
}

impl Boundary {
    /// Stable string form (the `boundary` column).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Action => "action",
            Self::Batch => "batch",
            Self::Turn => "turn",
        }
    }

    /// Parse the stable string form (storage read path).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "action" => Some(Self::Action),
            "batch" => Some(Self::Batch),
            "turn" => Some(Self::Turn),
            _ => None,
        }
    }
}

/// A checkpoint verdict (FR-002: the closed set — the layer never rewrites).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Nothing reaches the model or user (the default).
    Silence,
    /// A specific, actionable observation delivered to the model.
    Flag,
    /// A pending action paused for user confirmation (gate boundary only).
    Hold,
}

impl Verdict {
    /// Stable string form (the `verdict` column).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Silence => "silence",
            Self::Flag => "flag",
            Self::Hold => "hold",
        }
    }
}

/// A v1 detector (FR-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    /// Near-identical actions repeated within the window (US1).
    Repetition,
    /// The same action failing consecutively (US1).
    RepeatedFailure,
    /// A pending action conflicting with a verified stored constraint (US2).
    MemoryConflict,
    /// The turn's conclusion conflicting with an earlier committed statement
    /// (US3).
    SelfContradiction,
    /// The turn violating a trusted stored preference (015 — the enforce
    /// half of capture→store→recall→enforce; flag-only authority).
    PreferenceViolation,
}

impl SignalKind {
    /// Stable string form (record JSON + `signal_key` prefix).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repetition => "repetition",
            Self::RepeatedFailure => "repeated_failure",
            Self::MemoryConflict => "memory_conflict",
            Self::SelfContradiction => "self_contradiction",
            Self::PreferenceViolation => "preference_violation",
        }
    }
}

/// One fired signal: the detector, its specific evidence, and the stable
/// cooldown identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signal {
    /// The detector that fired.
    pub kind: SignalKind,
    /// What it detected — specific, names the action/statements (SC-007).
    pub evidence: String,
    /// `kind:fnv1a64(normalized evidence)` — the FR-010 cooldown identity.
    pub signal_key: String,
}

impl Signal {
    /// Build a signal, deriving its stable cooldown key from `identity` (the
    /// normalized form of what fired — NOT the display evidence, which may
    /// carry counts that change between checkpoints).
    #[must_use]
    pub fn new(kind: SignalKind, evidence: String, identity: &str) -> Self {
        let signal_key = format!("{}:{:016x}", kind.as_str(), fnv1a64(identity.as_bytes()));
        Self {
            kind,
            evidence,
            signal_key,
        }
    }
}

/// One checkpoint evaluation's audit row (FR-006; data-model.md §6).
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointRecord {
    /// UUID v4.
    pub id: String,
    /// The harness session.
    pub session_id: String,
    /// Which boundary evaluated.
    pub boundary: Boundary,
    /// Every detector this evaluation ran.
    pub signals_evaluated: Vec<SignalKind>,
    /// The signals that fired (pre-cooldown — the audit view).
    pub signals_fired: Vec<Signal>,
    /// The signal keys actually DELIVERED by this evaluation — the FR-010
    /// cooldown feed. A partially suppressed flag delivers only its
    /// unsuppressed subset; recording all fired keys here would extend the
    /// cooldown of signals that were never redelivered (review finding 2).
    pub delivered_keys: Vec<String>,
    /// Whether the review hop ran (turn boundary only).
    pub review_ran: bool,
    /// The delivered verdict (post-cooldown).
    pub verdict: Verdict,
    /// A flag was due but cooldown-suppressed (FR-010).
    pub suppressed: bool,
    /// The evaluation degraded (FR-008).
    pub fail_open: bool,
    /// Wall-clock evaluation time.
    pub latency_ms: u64,
    /// Metered cost of this evaluation (the review hop; 0 for pure paths).
    pub cost_usd: f64,
    /// Via `TimeProvider`.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// FNV-1a 64-bit — implemented locally so `signal_key` is stable across
/// builds and platforms (std's `DefaultHasher` makes no such promise).
#[must_use]
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn wire_enums_serialize_snake_case() {
        assert_eq!(
            serde_json::to_value(Verdict::Silence).unwrap(),
            serde_json::json!("silence")
        );
        assert_eq!(
            serde_json::to_value(SignalKind::RepeatedFailure).unwrap(),
            serde_json::json!("repeated_failure")
        );
        assert_eq!(
            serde_json::to_value(SignalKind::SelfContradiction).unwrap(),
            serde_json::json!("self_contradiction")
        );
        assert_eq!(
            serde_json::to_value(SignalKind::PreferenceViolation).unwrap(),
            serde_json::json!("preference_violation")
        );
        assert_eq!(
            SignalKind::PreferenceViolation.as_str(),
            "preference_violation"
        );
    }

    #[test]
    fn boundary_round_trips_its_column_form() {
        for boundary in [Boundary::Action, Boundary::Batch, Boundary::Turn] {
            assert_eq!(Boundary::parse(boundary.as_str()), Some(boundary));
        }
        assert_eq!(Boundary::parse("nope"), None);
    }

    #[test]
    fn signal_key_is_stable_and_identity_driven() {
        let a = Signal::new(
            SignalKind::Repetition,
            "`cargo test` invoked 4 times".into(),
            "bash cargo test",
        );
        let b = Signal::new(
            SignalKind::Repetition,
            "`cargo test` invoked 5 times".into(), // display count changed
            "bash cargo test",                     // identity did not
        );
        assert_eq!(a.signal_key, b.signal_key);
        // Pinned value: the key must never change across releases (stored
        // rows from prior sessions feed the cooldown).
        assert_eq!(a.signal_key, "repetition:305cf12b85b4a35d");

        let other = Signal::new(SignalKind::Repetition, "x".into(), "bash cargo build");
        assert_ne!(a.signal_key, other.signal_key);
        let other_kind = Signal::new(SignalKind::RepeatedFailure, "x".into(), "bash cargo test");
        assert_ne!(a.signal_key, other_kind.signal_key);
    }
}
