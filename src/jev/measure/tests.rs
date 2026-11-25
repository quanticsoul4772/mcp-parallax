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

fn noise(site: &str, question: &str, samples: &[f64], choices: &[&str]) -> NoiseItem {
    NoiseItem {
        site: site.into(),
        row_id: "r".into(),
        question: question.into(),
        samples: samples.to_vec(),
        choices: choices.iter().map(|c| (*c).to_string()).collect(),
    }
}

#[test]
fn a_choice_question_flips_when_its_chosen_option_changes() {
    // P(supports) stays low, but the top option changes.
    let moved = noise(
        "s",
        "relation_1",
        &[0.1, 0.1],
        &["says_nothing", "contradicts"],
    );
    assert!(item_flipped(&moved));
    // P(supports) crosses 0.5, but supports stays the top option.
    let steady = noise("s", "relation_1", &[0.48, 0.52], &["supports", "supports"]);
    assert!(!item_flipped(&steady));
    let yes_no = noise("s", "contradiction", &[0.4, 0.6], &[]);
    assert!(item_flipped(&yes_no));
}

#[test]
fn per_source_questions_fold_into_one_family() {
    assert_eq!(question_family("relation_1"), "relation");
    assert_eq!(question_family("same_subject_12"), "same_subject");
    assert_eq!(question_family("contradiction"), "contradiction");
    assert_eq!(question_family("capture_worthy"), "capture_worthy");
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
fn untimestamped_lines_are_kept_before_the_turn_and_dropped_after_it() {
    let text = [
        r#"{"type":"summary","summary":"before"}"#.to_string(),
        line("assistant", "2026-11-25T10:00:05Z", "a1"),
        r#"{"type":"file-history-snapshot","note":"after"}"#.to_string(),
        line("user", "2026-11-25T11:00:00Z", "q2"),
    ]
    .join("\n");
    let until = Utc.with_ymd_and_hms(2026, 11, 25, 10, 30, 0).unwrap();
    let cut = transcript_until(&text, until);
    assert!(cut.contains("before") && cut.contains("\"a1\""), "{cut}");
    assert!(!cut.contains("after"), "{cut}");
}

#[test]
fn the_cut_keeps_the_files_own_line_endings() {
    let text = format!(
        "{}\r\n{}\r\n{}\r\n",
        line("user", "2026-11-25T10:00:00Z", "q1"),
        line("assistant", "2026-11-25T10:00:05Z", "a1"),
        line("user", "2026-11-25T11:00:00Z", "q2"),
    );
    let until = Utc.with_ymd_and_hms(2026, 11, 25, 10, 30, 0).unwrap();
    let cut = transcript_until(&text, until);
    assert_eq!(cut.matches("\r\n").count(), 2, "{cut:?}");
    assert!(cut.ends_with("\r\n"));
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

fn sample_results() -> Results {
    Results {
        noise: vec![
            noise("research_claims", "relation_1", &[0.8; 5], &["supports"; 5]),
            noise("research_claims", "relation_2", &[0.8; 3], &["supports"; 3]),
            noise("research_claims", "same_subject_1", &[0.9; 5], &[]),
            noise(
                "turn_review",
                "contradiction",
                &[0.4, 0.6, 0.5, 0.5, 0.5],
                &[],
            ),
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
        skipped: vec![
            ("turn_review".into(), "noise: transcript missing".into(), 3),
            ("research_claims".into(), "noise: failed calls".into(), 2),
        ],
        base_rate: Some(BaseRate {
            evaluations: 761,
            reviews: 648,
            fired: vec![("self_contradiction".into(), 5)],
        }),
        injection: Some(InjectionResult {
            threshold: 0.35,
            items: 10,
            failed: 1,
            tp: 3,
            fp: 1,
            tn: 5,
            fn_: 1,
            undecided_positive: 0,
            undecided_negative: 0,
        }),
        notes: vec!["turn review run without stored preferences".into()],
    }
}

#[test]
fn results_round_trip_and_reproduce_the_same_tables() {
    let results = sample_results();
    let saved = serde_json::to_string(&results).unwrap();
    let loaded: Results = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded, results);
    let printed = tables(&loaded);
    assert_eq!(printed, tables(&results));
    for expected in [
        "research_claims relation: 2 items, 3..5 samples each",
        "research_claims same_subject: 1 items, 5..5 samples each",
        "turn_review contradiction: 1 items",
        "flipped 1",
        "transcript missing: 3",
        "failed calls: 2",
        "1 decisions, 1 flips",
        "same verdict twice: 1 of 1",
        "base rate: 761 turn evaluations, 648 reviews run",
        "fired self_contradiction: 5",
        "injection at threshold 0.35: 10 items, 1 failed",
        "recall 95% interval",
        "false-positive rate 95% interval",
        "undecided: 0 injections, 0 clean",
        "note: turn review run without stored preferences",
    ] {
        assert!(
            printed.contains(expected),
            "missing {expected:?} in\n{printed}"
        );
    }
}

#[test]
fn replacing_one_sites_noise_keeps_the_other_sites() {
    let mut results = sample_results();
    let fresh = vec![noise("turn_review", "capture_worthy", &[0.2; 5], &[])];
    results.replace_noise("turn_review", fresh.clone());
    let turn: Vec<&NoiseItem> = results
        .noise
        .iter()
        .filter(|i| i.site == "turn_review")
        .collect();
    assert_eq!(turn, fresh.iter().collect::<Vec<_>>());
    assert_eq!(
        results
            .noise
            .iter()
            .filter(|i| i.site == "research_claims")
            .count(),
        3
    );
}

#[test]
fn replacing_one_sites_agreement_keeps_the_other_sites() {
    let mut results = sample_results();
    let fresh = vec![AgreementItem {
        site: "turn_review".into(),
        row_id: "t1".into(),
        first: "contradiction=false capture=false".into(),
        second: "contradiction=false capture=false".into(),
    }];
    results.replace_agreement("turn_review", fresh.clone());
    assert!(results.self_agreement.contains(&fresh[0]));
    assert!(results
        .self_agreement
        .iter()
        .any(|i| i.site == "research_claims"));
}

fn counts(pairs: &[(&str, u32)]) -> BTreeMap<String, u32> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
}

fn skips_for<'a>(results: &'a Results, site: &str) -> Vec<&'a (String, String, u32)> {
    results
        .skipped
        .iter()
        .filter(|(s, _, _)| s == site)
        .collect()
}

#[test]
fn a_site_whose_rows_were_all_skipped_keeps_its_items_but_updates_its_counts() {
    let mut results = sample_results();
    let before = results.noise.clone();
    results.record_noise_run("turn_review", None, &counts(&[("transcript missing", 9)]));
    assert_eq!(results.noise, before);
    let turn = skips_for(&results, "turn_review");
    assert_eq!(turn.len(), 1);
    assert_eq!(turn[0].1, "noise: transcript missing");
    assert_eq!(turn[0].2, 9);
}

#[test]
fn an_all_skipped_run_keeps_the_loss_counts_behind_the_kept_items() {
    let mut results = sample_results();
    results
        .skipped
        .push(("turn_review".into(), "noise: failed calls".into(), 3));
    results.record_noise_run("turn_review", None, &counts(&[("transcript missing", 9)]));
    let turn = skips_for(&results, "turn_review");
    assert!(turn
        .iter()
        .any(|(_, r, n)| r == "noise: failed calls" && *n == 3));
    assert!(turn
        .iter()
        .any(|(_, r, n)| r == "noise: transcript missing" && *n == 9));
    assert_eq!(turn.len(), 2);
}

#[test]
fn a_site_with_usable_rows_replaces_its_items_and_counts() {
    let mut results = sample_results();
    let fresh = vec![noise("turn_review", "contradiction", &[0.1; 5], &[])];
    results.record_noise_run("turn_review", Some(fresh), &counts(&[]));
    assert_eq!(
        results
            .noise
            .iter()
            .filter(|i| i.site == "turn_review")
            .count(),
        1
    );
    assert!(skips_for(&results, "turn_review").is_empty());
    assert_eq!(skips_for(&results, "research_claims").len(), 1);
}

#[test]
fn a_site_the_run_found_nothing_for_is_left_alone() {
    let mut results = sample_results();
    let before = results.clone();
    results.record_noise_run("turn_review", None, &counts(&[]));
    results.record_agreement_run("turn_review", None, &counts(&[]));
    assert_eq!(results, before);
}

#[test]
fn agreement_runs_follow_the_same_rule() {
    let mut results = sample_results();
    results.record_agreement_run("research_claims", None, &counts(&[("no stored state", 4)]));
    assert_eq!(results.self_agreement.len(), 1);
    assert!(results
        .skipped
        .iter()
        .any(|(s, r, n)| s == "research_claims"
            && r == "self-agreement: no stored state"
            && *n == 4));
}

#[test]
fn replacing_a_note_keeps_the_other_notes() {
    let mut results = sample_results();
    results.replace_note(
        "self-agreement turn_review",
        "self-agreement turn_review: model a".into(),
    );
    results.replace_note(
        "self-agreement research_claims",
        "self-agreement research_claims: model b".into(),
    );
    results.replace_note(
        "self-agreement turn_review",
        "self-agreement turn_review: model c".into(),
    );
    assert!(results
        .notes
        .contains(&"self-agreement research_claims: model b".to_string()));
    assert!(results
        .notes
        .contains(&"self-agreement turn_review: model c".to_string()));
    assert!(!results
        .notes
        .contains(&"self-agreement turn_review: model a".to_string()));
    assert!(results
        .notes
        .contains(&"turn review run without stored preferences".to_string()));
}

#[test]
fn old_format_self_agreement_notes_are_dropped() {
    let mut results = sample_results();
    results
        .notes
        .push("self-agreement models: research verify a (effort None)".into());
    results
        .notes
        .push("self-agreement: turn review run without stored preferences".into());
    results
        .notes
        .push("self-agreement turn_review: turn review model b (effort None)".into());
    results.drop_legacy_notes();
    assert!(!results
        .notes
        .iter()
        .any(|n| n.starts_with("self-agreement models:") || n.starts_with("self-agreement: ")));
    assert!(results
        .notes
        .contains(&"self-agreement turn_review: turn review model b (effort None)".to_string()));
    assert!(results
        .notes
        .contains(&"turn review run without stored preferences".to_string()));
}

#[test]
fn old_all_site_entries_are_dropped() {
    let mut results = sample_results();
    results
        .skipped
        .push(("all".into(), "noise: failed calls".into(), 7));
    results.drop_legacy_all();
    assert!(results.skipped.iter().all(|(s, _, _)| s != "all"));
    assert_eq!(results.skipped.len(), 2);
}

#[test]
fn a_broken_results_file_is_an_error_not_empty_results() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("none.json");
    assert!(read_results(&missing, true).is_err());
    assert_eq!(read_results(&missing, false).unwrap(), Results::default());
    let broken = dir.path().join("broken.json");
    std::fs::write(&broken, "{ not json").unwrap();
    assert!(read_results(&broken, false).is_err());
    let unrelated = dir.path().join("unrelated.json");
    std::fs::write(&unrelated, r#"{"a":1}"#).unwrap();
    assert!(read_results(&unrelated, true).is_err());
    let good = dir.path().join("good.json");
    std::fs::write(&good, serde_json::to_string(&sample_results()).unwrap()).unwrap();
    assert_eq!(read_results(&good, true).unwrap(), sample_results());
}
