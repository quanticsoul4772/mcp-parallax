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
        version: 1,
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
            version: 1,
        }],
        self_agreement: vec![AgreementItem {
            site: "research_claims".into(),
            row_id: "r1".into(),
            first: "supported".into(),
            second: "supported".into(),
            version: 1,
        }],
        yes_no_order: vec![],
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
            skipped: 2,
            // The rows add up to the pooled result above.
            per_set: vec![
                crate::jev::injection::InjectionSetRow {
                    set: "bipia".into(),
                    failed: 1,
                    tp: 3,
                    fp: 0,
                    tn: 0,
                    fn_: 1,
                    undecided_positive: 0,
                    undecided_negative: 0,
                },
                crate::jev::injection::InjectionSetRow {
                    set: "clean".into(),
                    failed: 0,
                    tp: 0,
                    fp: 1,
                    tn: 5,
                    fn_: 0,
                    undecided_positive: 0,
                    undecided_negative: 0,
                },
            ],
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
        "2 dataset lines skipped",
        "set bipia: 4 items, 1 failed; recall 0.7500; false-positive rate undefined",
        "set clean: 6 items, 0 failed; recall undefined; false-positive rate 0.1667",
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
    results.replace_noise("turn_review", 1, fresh.clone());
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
        version: 1,
    }];
    results.replace_agreement("turn_review", 1, fresh.clone());
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
    results.record_noise_run(
        "turn_review",
        1,
        None,
        &counts(&[("transcript missing", 9)]),
    );
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
    results.record_noise_run(
        "turn_review",
        1,
        None,
        &counts(&[("transcript missing", 9)]),
    );
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
    results.record_noise_run("turn_review", 1, Some(fresh), &counts(&[]));
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
    results.record_noise_run("turn_review", 1, None, &counts(&[]));
    results.record_agreement_run("turn_review", 1, None, &counts(&[]));
    assert_eq!(results, before);
}

#[test]
fn agreement_runs_follow_the_same_rule() {
    let mut results = sample_results();
    results.record_agreement_run(
        "research_claims",
        1,
        None,
        &counts(&[("no stored state", 4)]),
    );
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

#[test]
fn an_injection_result_saved_before_the_skip_count_loads_with_zero() {
    let saved = r#"{"threshold": 0.35, "items": 3, "failed": 0, "tp": 1, "fp": 0,
        "tn": 2, "fn_": 0, "undecided_positive": 0, "undecided_negative": 0}"#;
    let loaded: InjectionResult = serde_json::from_str(saved).unwrap();
    assert_eq!(loaded.skipped, 0);
    assert!(loaded.per_set.is_empty());
}

// 066 T019a: results carry the question version, so a version-2 run keeps
// 064's version-1 results; files saved before 066 read back as version 1.

#[test]
fn saved_items_without_a_version_read_back_as_version_1() {
    let noise: NoiseItem = serde_json::from_value(serde_json::json!({
        "site": "turn_review", "row_id": "r", "question": "contradiction",
        "samples": [0.1, 0.2]
    }))
    .unwrap();
    assert_eq!(noise.version, 1);
    let order: OrderItem = serde_json::from_value(serde_json::json!({
        "row_id": "r", "question": "relation_1", "order": ["a", "b", "c"],
        "base_supports": true, "supports": true
    }))
    .unwrap();
    assert_eq!(order.version, 1);
    let agreement: AgreementItem = serde_json::from_value(serde_json::json!({
        "site": "turn_review", "row_id": "r", "first": "x", "second": "x"
    }))
    .unwrap();
    assert_eq!(agreement.version, 1);
}

#[test]
fn a_version_2_noise_run_keeps_the_version_1_items_and_counts() {
    let mut results = sample_results();
    results.record_noise_run(
        "turn_review",
        1,
        Some(vec![noise("turn_review", "contradiction", &[0.3; 5], &[])]),
        &counts(&[("no stored state", 2)]),
    );
    let mut v2 = noise("turn_review", "contradiction", &[0.4; 5], &[]);
    v2.version = 2;
    results.record_noise_run(
        "turn_review",
        2,
        Some(vec![v2]),
        &counts(&[("no stored state", 7)]),
    );
    let turn: Vec<u32> = results
        .noise
        .iter()
        .filter(|i| i.site == "turn_review")
        .map(|i| i.version)
        .collect();
    assert_eq!(turn, [1, 2]);
    let reasons: Vec<&str> = skips_for(&results, "turn_review")
        .iter()
        .map(|(_, reason, _)| reason.as_str())
        .collect();
    assert!(reasons.contains(&"noise: no stored state"), "{reasons:?}");
    assert!(
        reasons.contains(&"noise v2: no stored state"),
        "{reasons:?}"
    );
}

#[test]
fn a_version_2_agreement_run_keeps_the_version_1_items() {
    let mut results = sample_results();
    let item = |version: u32| AgreementItem {
        site: "turn_review".into(),
        row_id: format!("t{version}"),
        first: "same".into(),
        second: "same".into(),
        version,
    };
    results.record_agreement_run("turn_review", 1, Some(vec![item(1)]), &counts(&[]));
    results.record_agreement_run("turn_review", 2, Some(vec![item(2)]), &counts(&[]));
    assert!(results.self_agreement.contains(&item(1)));
    assert!(results.self_agreement.contains(&item(2)));
}

#[test]
fn the_tables_name_a_question_version_other_than_1() {
    let mut v2 = noise("turn_review", "contradiction", &[0.4; 5], &[]);
    v2.version = 2;
    let results = Results {
        noise: vec![noise("turn_review", "contradiction", &[0.3; 5], &[]), v2],
        ..Results::default()
    };
    let printed = tables(&results);
    assert!(
        printed.contains("turn_review contradiction v2:"),
        "{printed}"
    );
    assert!(printed.contains("turn_review contradiction:"), "{printed}");
}

// 066 T019c: the yes/no order result keeps P(yes) under both criteria orders,
// so flips can be recounted at any cut; the table reports flips at 0.5 and
// the mean absolute shift.

fn yes_no(question: &str, true_first: f64, false_first: f64) -> YesNoOrderItem {
    YesNoOrderItem {
        site: "turn_review".into(),
        row_id: "r".into(),
        question: question.into(),
        version: 2,
        true_first,
        false_first,
    }
}

#[test]
fn a_results_file_saved_before_the_yes_no_order_result_reads_back_empty() {
    let results: Results = serde_json::from_value(serde_json::json!({
        "noise": [], "order": [], "self_agreement": [], "skipped": [],
        "base_rate": null, "injection": null
    }))
    .unwrap();
    assert!(results.yes_no_order.is_empty());
}

#[test]
fn the_yes_no_order_table_counts_flips_at_one_half_and_the_mean_shift() {
    let results = Results {
        yes_no_order: vec![
            yes_no("contradiction", 0.6, 0.4), // yes to no
            yes_no("contradiction", 0.3, 0.7), // no to yes
            yes_no("contradiction", 0.2, 0.3), // no flip
            yes_no("contradiction", 0.9, 0.9), // no flip
        ],
        ..Results::default()
    };
    let printed = tables(&results);
    assert!(
        printed.contains(
            "yes/no order turn_review contradiction v2: 4 items, 2 flips at 0.5 \
             (1 yes to no, 1 no to yes), McNemar 0.000, mean absolute shift 0.1750"
        ),
        "{printed}"
    );
}

#[test]
fn replacing_the_yes_no_order_items_keeps_other_versions() {
    let mut results = Results::default();
    let mut v3 = yes_no("contradiction", 0.1, 0.2);
    v3.version = 3;
    results.replace_yes_no_order("turn_review", 3, vec![v3.clone()]);
    results.replace_yes_no_order("turn_review", 2, vec![yes_no("contradiction", 0.5, 0.5)]);
    results.replace_yes_no_order("turn_review", 2, vec![yes_no("capture_worthy", 0.4, 0.4)]);
    assert_eq!(results.yes_no_order.len(), 2);
    assert!(results.yes_no_order.contains(&v3));
    assert!(results
        .yes_no_order
        .iter()
        .any(|i| i.question == "capture_worthy" && i.version == 2));
}
