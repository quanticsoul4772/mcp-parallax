//! Deterministic screening detectors (D5) — pure functions over the window.
//!
//! Exact match after normalization; no fuzzy similarity, no model calls.
//! Tested against ground-truth tables, never mocked.

use crate::checkpoint::trajectory::{TrajectoryEntry, TrajectoryWindow};
use crate::checkpoint::{Signal, SignalKind, FAILURE_THRESHOLD, REPEAT_THRESHOLD, WINDOW_BATCHES};
use std::collections::HashMap;

/// Run both screening detectors over the window.
#[must_use]
pub fn screen(window: &TrajectoryWindow) -> Vec<Signal> {
    let mut signals = repetition(window);
    signals.extend(repeated_failure(window));
    signals
}

/// The normalized identity of one action — the unit of repetition.
fn action_identity(tool_name: &str, normalized_input: &str) -> String {
    format!("{tool_name} {normalized_input}")
}

/// A display snippet of the action for evidence strings (SC-007: every flag
/// names the specific action).
fn action_display(tool_name: &str, normalized_input: &str) -> String {
    const SNIPPET_MAX: usize = 80;
    let input: String = normalized_input.chars().take(SNIPPET_MAX).collect();
    let ellipsis = if normalized_input.chars().count() > SNIPPET_MAX {
        "..."
    } else {
        ""
    };
    format!("{tool_name} {input}{ellipsis}")
}

/// US1-AS1: the same normalized action ≥ [`REPEAT_THRESHOLD`] times within
/// the last [`WINDOW_BATCHES`] batches.
#[must_use]
pub fn repetition(window: &TrajectoryWindow) -> Vec<Signal> {
    let max_batch = window
        .entries
        .iter()
        .filter_map(|e| match e {
            TrajectoryEntry::ToolCall { batch_index, .. } => Some(*batch_index),
            TrajectoryEntry::Assistant { .. } => None,
        })
        .max()
        .unwrap_or(0);
    let floor = max_batch.saturating_sub(WINDOW_BATCHES - 1);

    let mut counts: HashMap<String, (usize, String)> = HashMap::new();
    for entry in &window.entries {
        if let TrajectoryEntry::ToolCall {
            batch_index,
            tool_name,
            normalized_input,
            ..
        } = entry
        {
            if *batch_index >= floor {
                let identity = action_identity(tool_name, normalized_input);
                let slot = counts
                    .entry(identity)
                    .or_insert_with(|| (0, action_display(tool_name, normalized_input)));
                slot.0 += 1;
            }
        }
    }

    let mut fired: Vec<(String, usize, String)> = counts
        .into_iter()
        .filter(|(_, (count, _))| *count >= REPEAT_THRESHOLD)
        .map(|(identity, (count, display))| (identity, count, display))
        .collect();
    fired.sort_by(|a, b| a.0.cmp(&b.0)); // deterministic order
    fired
        .into_iter()
        .map(|(identity, count, display)| {
            Signal::new(
                SignalKind::Repetition,
                format!(
                    "the action `{display}` was invoked {count} times in the last \
                     {WINDOW_BATCHES} tool batches with near-identical input"
                ),
                &identity,
            )
        })
        .collect()
}

/// US1-AS2: the same normalized action failing ≥ [`FAILURE_THRESHOLD`]
/// times consecutively.
///
/// Consecutive among that action's own invocations — interleaved other
/// actions don't reset the streak; its own success does.
#[must_use]
pub fn repeated_failure(window: &TrajectoryWindow) -> Vec<Signal> {
    let mut streaks: HashMap<String, (usize, String)> = HashMap::new();
    let mut fired: HashMap<String, (usize, String)> = HashMap::new();

    for entry in &window.entries {
        if let TrajectoryEntry::ToolCall {
            tool_name,
            normalized_input,
            failed,
            ..
        } = entry
        {
            let identity = action_identity(tool_name, normalized_input);
            if *failed {
                let slot = streaks
                    .entry(identity.clone())
                    .or_insert_with(|| (0, action_display(tool_name, normalized_input)));
                slot.0 += 1;
                if slot.0 >= FAILURE_THRESHOLD {
                    fired.insert(identity, slot.clone());
                }
            } else {
                streaks.remove(&identity);
                fired.remove(&identity);
            }
        }
    }

    let mut sorted: Vec<(String, (usize, String))> = fired.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
        .into_iter()
        .map(|(identity, (count, display))| {
            Signal::new(
                SignalKind::RepeatedFailure,
                format!(
                    "the action `{display}` has failed {count} consecutive times \
                     with the same input"
                ),
                &identity,
            )
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn call(batch: u32, tool: &str, input: &str, failed: bool) -> TrajectoryEntry {
        TrajectoryEntry::ToolCall {
            batch_index: batch,
            tool_name: tool.to_string(),
            normalized_input: input.to_string(),
            failed,
        }
    }

    fn window(entries: Vec<TrajectoryEntry>) -> TrajectoryWindow {
        TrajectoryWindow {
            session_id: "s1".into(),
            entries,
        }
    }

    // US1-AS1: 4 near-identical invocations fire; the evidence names them.
    #[test]
    fn four_identical_actions_fire_repetition_with_named_evidence() {
        let w = window(vec![
            call(1, "bash", "{command=cargo test;}", false),
            call(2, "bash", "{command=cargo test;}", true),
            call(3, "bash", "{command=cargo test;}", false),
            call(4, "bash", "{command=cargo test;}", true),
        ]);
        let signals = repetition(&w);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].kind, SignalKind::Repetition);
        assert!(
            signals[0].evidence.contains("cargo test"),
            "{}",
            signals[0].evidence
        );
        assert!(
            signals[0].evidence.contains("4 times"),
            "{}",
            signals[0].evidence
        );
    }

    // Benign: 3 repetitions stay under threshold.
    #[test]
    fn three_repetitions_are_benign() {
        let w = window(vec![
            call(1, "bash", "{command=cargo test;}", false),
            call(2, "bash", "{command=cargo test;}", false),
            call(3, "bash", "{command=cargo test;}", false),
        ]);
        assert!(repetition(&w).is_empty());
    }

    // Old repetitions outside the batch window don't count.
    #[test]
    fn repetitions_outside_the_batch_window_are_ignored() {
        let mut entries = vec![
            call(1, "bash", "{command=cargo test;}", false),
            call(2, "bash", "{command=cargo test;}", false),
            call(3, "bash", "{command=cargo test;}", false),
        ];
        // 12 batches later, one more — only it is inside the window.
        entries.push(call(15, "bash", "{command=cargo test;}", false));
        assert!(repetition(&window(entries)).is_empty());
    }

    // US1-AS2: 3 consecutive failures of the same command fire.
    #[test]
    fn three_consecutive_failures_fire_with_named_command() {
        let w = window(vec![
            call(1, "bash", "{command=npm run build;}", true),
            call(2, "read", "{file_path=a.rs;}", false), // interleaved, no reset
            call(2, "bash", "{command=npm run build;}", true),
            call(3, "bash", "{command=npm run build;}", true),
        ]);
        let signals = repeated_failure(&w);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].kind, SignalKind::RepeatedFailure);
        assert!(signals[0].evidence.contains("npm run build"));
        assert!(signals[0].evidence.contains("3 consecutive"));
    }

    // A success between failures resets the streak.
    #[test]
    fn success_resets_the_failure_streak() {
        let w = window(vec![
            call(1, "bash", "{command=x;}", true),
            call(2, "bash", "{command=x;}", true),
            call(3, "bash", "{command=x;}", false), // reset
            call(4, "bash", "{command=x;}", true),
        ]);
        assert!(repeated_failure(&w).is_empty());
    }

    // Non-consecutive failures (different inputs) stay benign.
    #[test]
    fn varied_failing_actions_are_benign() {
        let w = window(vec![
            call(1, "bash", "{command=a;}", true),
            call(2, "bash", "{command=b;}", true),
            call(3, "bash", "{command=c;}", true),
        ]);
        assert!(repeated_failure(&w).is_empty());
    }

    // A benign mixed window: varied tools, progress, nothing fires.
    #[test]
    fn benign_mixed_window_is_silent() {
        let w = window(vec![
            call(1, "read", "{file_path=a.rs;}", false),
            call(1, "grep", "{pattern=foo;}", false),
            call(2, "edit", "{file_path=a.rs;}", false),
            call(3, "bash", "{command=cargo test;}", true),
            call(4, "edit", "{file_path=a.rs;}", false),
            call(5, "bash", "{command=cargo test;}", false),
        ]);
        assert!(screen(&w).is_empty());
    }

    // SC-007 determinism: same window twice → identical signals.
    #[test]
    fn screening_is_deterministic() {
        let w = window(vec![
            call(1, "bash", "{command=cargo test;}", true),
            call(2, "bash", "{command=cargo test;}", true),
            call(3, "bash", "{command=cargo test;}", true),
            call(4, "bash", "{command=cargo test;}", true),
        ]);
        assert_eq!(screen(&w), screen(&w));
        // Both detectors fire on this window.
        let kinds: Vec<SignalKind> = screen(&w).iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&SignalKind::Repetition));
        assert!(kinds.contains(&SignalKind::RepeatedFailure));
    }
}
