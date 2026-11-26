//! Answer validation and bands for Jev (064).
//!
//! An answer is used only if it passes every check; a rejected or missing
//! answer is treated as missing and never lands in an acting band.

use crate::traits::classifier::{JevAnswer, JevQuestion, JevResponse, QuestionKind};
use std::collections::BTreeSet;

/// How far a choice's probabilities may stray from summing to 1.
const SUM_TOLERANCE: f64 = 0.01;

/// Where an answer falls against a question's two thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    /// At or above the yes threshold.
    Yes,
    /// Between the thresholds: recorded, never acted on.
    Middle,
    /// At or below the no threshold.
    No,
    /// No usable answer. Never treated as yes or no.
    Missing,
}

/// The injection threshold pinned per Jev model (065, research D3): the
/// 064 back-test measured 0.35 on `jev-1.14.0`. A model not listed has no
/// threshold, so the injection site cannot be `on` for it.
pub const INJECTION_THRESHOLDS: &[(&str, f64)] = &[("jev-1.14.0", 0.35)];

/// The pinned injection threshold for `model`, if there is one.
#[must_use]
pub fn injection_threshold(model: &str) -> Option<f64> {
    INJECTION_THRESHOLDS
        .iter()
        .find(|(m, _)| *m == model)
        .map(|(_, t)| *t)
}

/// An injection answer against its threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionDecision {
    /// At or above the threshold.
    Warned,
    /// Below the threshold.
    Clear,
    /// No usable answer; never a warning.
    Missing,
}

/// The decision for a validated probability `p` at `threshold`.
#[must_use]
pub fn injection_decision(p: Option<f64>, threshold: f64) -> InjectionDecision {
    match p {
        Some(p) if p >= threshold => InjectionDecision::Warned,
        Some(_) => InjectionDecision::Clear,
        None => InjectionDecision::Missing,
    }
}

/// The band for `p` against `no_at` and `yes_at`. A missing answer is
/// [`Band::Missing`], whichever direction a caller acts on.
#[must_use]
pub fn band(p: Option<f64>, no_at: f64, yes_at: f64) -> Band {
    match p {
        Some(p) if p >= yes_at => Band::Yes,
        Some(p) if p <= no_at => Band::No,
        Some(p) if p.is_finite() => Band::Middle,
        _ => Band::Missing,
    }
}

/// The P(yes) answered for question `id`, if any.
#[must_use]
pub fn noul_p(response: &JevResponse, id: &str) -> Option<f64> {
    response
        .answers
        .iter()
        .find_map(|(key, answer)| match answer {
            JevAnswer::Noul { p } if key == id => Some(*p),
            _ => None,
        })
}

/// Check a response against the questions asked and the pinned model.
///
/// # Errors
///
/// A short reason when any check fails: the answering model isn't
/// `pinned`; answer ids are missing, extra or duplicated; an answer's type
/// doesn't match its question; a probability is outside 0..=1 or not a
/// number; a choice isn't a declared option; a declared option has no
/// probability; or the probabilities don't sum to 1 within 0.01.
pub fn validate(
    questions: &[JevQuestion],
    response: &JevResponse,
    pinned: &str,
) -> Result<(), String> {
    if response.model_answered != pinned {
        return Err(format!(
            "answered by {} instead of {pinned}",
            response.model_answered
        ));
    }
    let asked: BTreeSet<&str> = questions.iter().map(|q| q.id.as_str()).collect();
    let mut seen = BTreeSet::new();
    for (id, _) in &response.answers {
        if !seen.insert(id.as_str()) {
            return Err(format!("duplicate answer `{id}`"));
        }
        if !asked.contains(id.as_str()) {
            return Err(format!("answer `{id}` was not asked"));
        }
    }
    for question in questions {
        let answer = response
            .answers
            .iter()
            .find(|(id, _)| *id == question.id)
            .map(|(_, a)| a)
            .ok_or_else(|| format!("no answer for `{}`", question.id))?;
        check_answer(question, answer)?;
    }
    Ok(())
}

fn is_probability(p: f64) -> bool {
    p.is_finite() && (0.0..=1.0).contains(&p)
}

fn check_answer(question: &JevQuestion, answer: &JevAnswer) -> Result<(), String> {
    let id = &question.id;
    match (question.kind, answer) {
        (QuestionKind::Noul, JevAnswer::Noul { p }) => {
            if is_probability(*p) {
                Ok(())
            } else {
                Err(format!("`{id}`: probability {p} outside 0..=1"))
            }
        }
        (
            QuestionKind::Choice,
            JevAnswer::Choice {
                choice,
                probabilities,
            },
        ) => {
            if !question.options.iter().any(|(o, _)| o == choice) {
                return Err(format!("`{id}`: `{choice}` is not a declared option"));
            }
            for (option, _) in &question.options {
                let p = probabilities
                    .iter()
                    .find(|(o, _)| o == option)
                    .map(|(_, p)| *p)
                    .ok_or_else(|| format!("`{id}`: no probability for `{option}`"))?;
                if !is_probability(p) {
                    return Err(format!("`{id}`: probability {p} outside 0..=1"));
                }
            }
            let sum: f64 = probabilities.iter().map(|(_, p)| p).sum();
            if (sum - 1.0).abs() > SUM_TOLERANCE {
                return Err(format!("`{id}`: probabilities sum to {sum}"));
            }
            Ok(())
        }
        _ => Err(format!("`{id}`: answer type does not match the question")),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::{
        band, injection_decision, injection_threshold, noul_p, validate, Band, InjectionDecision,
    };
    use crate::traits::classifier::{JevAnswer, JevQuestion, JevResponse, QuestionKind};

    const PINNED: &str = "jev-1.14.0";

    fn noul_q(id: &str) -> JevQuestion {
        JevQuestion {
            id: id.into(),
            version: 1,
            kind: QuestionKind::Noul,
            instructions: "Is it?".into(),
            options: vec![],
        }
    }

    fn choice_q(id: &str) -> JevQuestion {
        JevQuestion {
            id: id.into(),
            version: 1,
            kind: QuestionKind::Choice,
            instructions: "Which?".into(),
            options: vec![("a".into(), None), ("b".into(), None)],
        }
    }

    fn response(answers: Vec<(&str, JevAnswer)>) -> JevResponse {
        JevResponse {
            model_answered: PINNED.into(),
            answers: answers
                .into_iter()
                .map(|(id, a)| (id.to_string(), a))
                .collect(),
            input_tokens: Some(100),
            request_id: Some("req-1".into()),
            latency_ms: 120,
        }
    }

    fn choice(choice: &str, probabilities: &[(&str, f64)]) -> JevAnswer {
        JevAnswer::Choice {
            choice: choice.into(),
            probabilities: probabilities
                .iter()
                .map(|(o, p)| ((*o).to_string(), *p))
                .collect(),
        }
    }

    fn questions() -> Vec<JevQuestion> {
        vec![noul_q("q1"), choice_q("q2")]
    }

    fn valid() -> JevResponse {
        response(vec![
            ("q1", JevAnswer::Noul { p: 0.9 }),
            ("q2", choice("a", &[("a", 0.7), ("b", 0.3)])),
        ])
    }

    #[test]
    fn a_fully_valid_response_passes() {
        assert!(validate(&questions(), &valid(), PINNED).is_ok());
    }

    #[test]
    fn a_noul_outside_zero_to_one_or_not_a_number_is_rejected() {
        for p in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
            let r = response(vec![
                ("q1", JevAnswer::Noul { p }),
                ("q2", choice("a", &[("a", 0.7), ("b", 0.3)])),
            ]);
            assert!(validate(&questions(), &r, PINNED).is_err(), "{p}");
        }
    }

    #[test]
    fn a_choice_that_is_not_a_declared_option_is_rejected() {
        let r = response(vec![
            ("q1", JevAnswer::Noul { p: 0.9 }),
            ("q2", choice("c", &[("a", 0.7), ("b", 0.3)])),
        ]);
        assert!(validate(&questions(), &r, PINNED).is_err());
    }

    #[test]
    fn a_declared_option_without_a_probability_is_rejected() {
        let r = response(vec![
            ("q1", JevAnswer::Noul { p: 0.9 }),
            ("q2", choice("a", &[("a", 1.0)])),
        ]);
        assert!(validate(&questions(), &r, PINNED).is_err());
    }

    #[test]
    fn probabilities_that_do_not_sum_to_one_within_a_hundredth_are_rejected() {
        let r = response(vec![
            ("q1", JevAnswer::Noul { p: 0.9 }),
            ("q2", choice("a", &[("a", 0.7), ("b", 0.32)])),
        ]);
        assert!(validate(&questions(), &r, PINNED).is_err());
        let ok = response(vec![
            ("q1", JevAnswer::Noul { p: 0.9 }),
            ("q2", choice("a", &[("a", 0.7), ("b", 0.305)])),
        ]);
        assert!(validate(&questions(), &ok, PINNED).is_ok());
    }

    #[test]
    fn missing_extra_or_duplicate_answer_ids_are_rejected() {
        let missing = response(vec![("q1", JevAnswer::Noul { p: 0.9 })]);
        let extra = response(vec![
            ("q1", JevAnswer::Noul { p: 0.9 }),
            ("q2", choice("a", &[("a", 0.7), ("b", 0.3)])),
            ("q3", JevAnswer::Noul { p: 0.1 }),
        ]);
        let duplicate = response(vec![
            ("q1", JevAnswer::Noul { p: 0.9 }),
            ("q1", JevAnswer::Noul { p: 0.8 }),
            ("q2", choice("a", &[("a", 0.7), ("b", 0.3)])),
        ]);
        for r in [missing, extra, duplicate] {
            assert!(validate(&questions(), &r, PINNED).is_err());
        }
    }

    #[test]
    fn an_answer_type_that_does_not_match_the_question_is_rejected() {
        let r = response(vec![
            ("q1", choice("a", &[("a", 0.7), ("b", 0.3)])),
            ("q2", choice("a", &[("a", 0.7), ("b", 0.3)])),
        ]);
        assert!(validate(&questions(), &r, PINNED).is_err());
    }

    #[test]
    fn an_answer_from_another_model_is_rejected() {
        let mut r = valid();
        r.model_answered = "jev-1.15.0".into();
        assert!(validate(&questions(), &r, PINNED).is_err());
    }

    #[test]
    fn bands_split_at_the_edges_and_missing_never_acts() {
        let (no_at, yes_at): (f64, f64) = (0.3, 0.7);
        assert_eq!(band(Some(yes_at), no_at, yes_at), Band::Yes);
        assert_eq!(band(Some(yes_at.next_down()), no_at, yes_at), Band::Middle);
        assert_eq!(band(Some(no_at), no_at, yes_at), Band::No);
        assert_eq!(band(Some(no_at.next_up()), no_at, yes_at), Band::Middle);
        assert_eq!(band(Some(1.0), no_at, yes_at), Band::Yes);
        assert_eq!(band(Some(0.0), no_at, yes_at), Band::No);
        assert_eq!(band(None, no_at, yes_at), Band::Missing);
        assert_ne!(band(None, no_at, yes_at), Band::Yes);
        assert_ne!(band(None, no_at, yes_at), Band::No);
    }

    #[test]
    fn a_partial_response_leaves_the_missing_question_missing() {
        let r = response(vec![("q1", JevAnswer::Noul { p: 0.9 })]);
        assert_eq!(noul_p(&r, "q1"), Some(0.9));
        assert_eq!(noul_p(&r, "q2"), None);
        assert_eq!(band(noul_p(&r, "q2"), 0.3, 0.7), Band::Missing);
    }

    #[test]
    fn the_injection_threshold_is_pinned_to_the_model() {
        assert_eq!(injection_threshold("jev-1.14.0"), Some(0.35));
        assert_eq!(injection_threshold("jev-1.13.0"), None);
        assert_eq!(injection_threshold("jev-latest"), None);
    }

    #[test]
    fn the_pinned_threshold_is_the_one_the_back_test_measured() {
        let doc = include_str!("../../claudedocs/jev-064-measurements-2026-11-24.md");
        assert!(
            doc.contains("injection at threshold 0.35:"),
            "the T038 (e) record no longer shows the 0.35 run"
        );
    }

    #[test]
    fn injection_warns_at_the_threshold_and_never_without_an_answer() {
        let t = 0.35_f64;
        assert_eq!(injection_decision(Some(t), t), InjectionDecision::Warned);
        assert_eq!(
            injection_decision(Some(t.next_down()), t),
            InjectionDecision::Clear
        );
        assert_eq!(injection_decision(Some(1.0), t), InjectionDecision::Warned);
        assert_eq!(injection_decision(None, t), InjectionDecision::Missing);
    }
}
