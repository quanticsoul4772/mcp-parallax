//! Tests for the injection site's report (065 T023).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{injection_table, Agreement, InjectionTable};
use crate::error::Outcome;
use crate::storage::jev::{JevOutcome, JevRecord};
use crate::traits::classifier::JevSite;
use serde_json::{json, Value};

fn row(band: &Value, patterns: &[&str], outcome: Outcome) -> JevRecord {
    let mut r = crate::jev::shadow::tests_support::claim_row();
    r.site = JevSite::ResearchInjection;
    r.questions_json = json!([{ "id": "directs_ai", "band": band.clone() }]);
    r.label_json = Some(json!({ "patterns063": patterns }));
    r.outcome = JevOutcome::Call(outcome);
    r
}

#[test]
fn the_table_counts_warned_failures_and_agreement_with_063() {
    let rows = [
        row(&json!("warned"), &["role_marker"], Outcome::Success),
        row(&json!("warned"), &[], Outcome::Success),
        row(&json!("clear"), &["hidden_text"], Outcome::Success),
        row(&json!("clear"), &[], Outcome::Success),
        row(&json!("clear"), &[], Outcome::Success),
        row(&Value::Null, &[], Outcome::Timeout),
        row(&Value::Null, &["role_marker"], Outcome::ClassifierProvider),
    ];
    let table = injection_table(&rows.iter().collect::<Vec<_>>());
    assert_eq!(
        table,
        InjectionTable {
            rows: 7,
            screened: 5,
            warned: 2,
            failures: vec![("classifier_provider".into(), 1), ("timeout".into(), 1)],
            agreement: Agreement {
                both: 1,
                jev_only: 1,
                only_063: 1,
                neither: 2,
            },
        }
    );
}

#[test]
fn rows_from_other_sites_are_ignored() {
    let claim = crate::jev::shadow::tests_support::claim_row();
    let table = injection_table(&[&claim]);
    assert_eq!(table.rows, 0);
    assert_eq!(table.screened, 0);
}
