#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use chrono::TimeZone;

#[test]
fn logit_spread_is_the_standard_deviation_on_the_logit_scale() {
    // logit(0.5) = 0 and logit(0.731_058_6) = 1, so the population SD is 0.5.
    let spread = logit_spread(&[0.5, 0.731_058_578_6]).unwrap();
    assert!((spread - 0.5).abs() < 1e-6, "{spread}");
    assert_eq!(logit_spread(&[0.9, 0.9, 0.9]), Some(0.0));
    assert_eq!(logit_spread(&[]), None);
}

#[test]
fn logit_spread_stays_finite_at_zero_and_one() {
    let spread = logit_spread(&[0.0, 1.0]).unwrap();
    assert!(spread.is_finite() && spread > 0.0);
}

#[test]
fn a_flip_needs_answers_on_both_sides_of_the_cut() {
    assert!(flipped(&[0.4, 0.6], 0.5));
    assert!(!flipped(&[0.6, 0.7, 0.9], 0.5));
    assert!(!flipped(&[0.1, 0.2], 0.5));
    assert!(flipped(&[0.5, 0.49], 0.5));
}

fn line(kind: &str, ts: &str, text: &str) -> String {
    serde_json::json!({
        "type": kind,
        "timestamp": ts,
        "message": { "content": [ { "type": "text", "text": text } ] }
    })
    .to_string()
}

#[test]
fn the_transcript_is_cut_at_the_first_later_line() {
    let text = [
        line("user", "2026-11-25T10:00:00Z", "q1"),
        line("assistant", "2026-11-25T10:00:05Z", "a1"),
        line("user", "2026-11-25T10:01:00Z", "q2"),
        line("assistant", "2026-11-25T10:01:09Z", "a2"),
    ]
    .join("\n");
    let until = Utc.with_ymd_and_hms(2026, 11, 25, 10, 0, 30).unwrap();
    let cut = transcript_until(&text, until);
    assert_eq!(cut.lines().count(), 2);
    assert!(cut.contains("\"a1\"") && !cut.contains("\"q2\""));
}

#[test]
fn lines_without_a_timestamp_before_the_cut_are_kept() {
    let text = [
        r#"{"type":"summary","summary":"s"}"#.to_string(),
        line("assistant", "2026-11-25T10:00:05Z", "a1"),
        line("user", "2026-11-25T11:00:00Z", "q2"),
    ]
    .join("\n");
    let until = Utc.with_ymd_and_hms(2026, 11, 25, 10, 30, 0).unwrap();
    assert_eq!(transcript_until(&text, until).lines().count(), 2);
}

#[test]
fn the_final_message_is_the_last_assistant_text() {
    let text = [
        line("assistant", "2026-11-25T10:00:05Z", "first answer"),
        serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-11-25T10:00:06Z",
            "message": { "content": [ { "type": "tool_use", "name": "Bash" } ] }
        })
        .to_string(),
        line("assistant", "2026-11-25T10:00:07Z", "final answer"),
        line("user", "2026-11-25T10:00:08Z", "thanks"),
    ]
    .join("\n");
    assert_eq!(last_assistant_text(&text).as_deref(), Some("final answer"));
    assert_eq!(last_assistant_text(&line("user", "t", "only user")), None);
}

#[test]
fn results_round_trip_and_reproduce_the_same_tables() {
    let results = Results {
        noise: vec![
            NoiseItem {
                site: "research_claims".into(),
                row_id: "r1".into(),
                question: "relation_1".into(),
                samples: vec![0.8, 0.8, 0.8, 0.8, 0.8],
            },
            NoiseItem {
                site: "turn_review".into(),
                row_id: "t1".into(),
                question: "contradiction".into(),
                samples: vec![0.4, 0.6, 0.5, 0.5, 0.5],
            },
        ],
        order: vec![OrderItem {
            row_id: "r1".into(),
            question: "relation_1".into(),
            order: vec![
                "supports".into(),
                "says_nothing".into(),
                "contradicts".into(),
            ],
            base_supports: true,
            supports: false,
        }],
        self_agreement: vec![AgreementItem {
            site: "research_claims".into(),
            row_id: "r1".into(),
            first: "supported".into(),
            second: "supported".into(),
        }],
        skipped: vec![("turn_review".into(), "transcript missing".into(), 3)],
    };
    let saved = serde_json::to_string(&results).unwrap();
    let loaded: Results = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded, results);
    let printed = tables(&loaded);
    assert_eq!(printed, tables(&results));
    assert!(printed.contains("turn_review: 1 items"), "{printed}");
    assert!(printed.contains("flipped 1"), "{printed}");
    assert!(printed.contains("1 decisions, 1 flips"), "{printed}");
    assert!(printed.contains("same verdict twice: 1 of 1"), "{printed}");
    assert!(printed.contains("transcript missing: 3"), "{printed}");
}
