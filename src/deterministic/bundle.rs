//! Assembling one `check` result from its separately checked parts.
//!
//! Pure: part results in, the wire result out. A one-part call passes its
//! part's fields through unchanged, so a single claim reads exactly as it did
//! before bundles existed. Two or more parts get a summary on top.
//!
//! The top-level verdict of a bundle is `refuted` if any part is refuted and
//! `not_checkable` otherwise — never `supported`. The model chose the parts
//! and nothing can confirm they cover the whole claim, so a caller reading
//! only the top-level verdict must not take an unchecked or omitted statement
//! as verified. What each part showed is in `parts`.

use crate::deterministic::contract::{CheckResult, PartResult};
use crate::deterministic::{Engine, Verdict};

/// The placeholder line for a part no engine executed.
const NOT_CHECKED: &str = "(not checked)";
/// Why a bundle is never supported as a whole — the first line of its reason.
const COVERAGE: &str = "the parts were checked separately; nothing can confirm they cover every \
                        statement in the claim, so the claim as a whole is not reported as \
                        supported — read each part's verdict in parts";
/// The reason given when there are no parts at all.
const NO_PARTS: &str = "the translation produced no parts, so nothing was checked";

/// One executed (or declined) part plus the explanation its engine produced.
/// The explanation is used only when the part is the whole claim.
pub struct Checked {
    /// The part as it appears in `parts`.
    pub part: PartResult,
    /// The deterministic explanation for this part alone.
    pub explanation: String,
}

/// Assemble the wire result.
///
/// The translation validator rejects a reply with no parts before execution,
/// so `checked` is not empty in the server's own path; an empty list still
/// assembles to `not_checkable` rather than relying on that.
#[must_use]
pub fn assemble(checked: Vec<Checked>, translation_attempts: u32) -> CheckResult {
    if let [only] = checked.as_slice() {
        let part = &only.part;
        return CheckResult {
            verdict: part.verdict,
            engine: part.engine,
            formal_form: part.formal_form.clone(),
            engine_result: part.engine_result.clone(),
            witness: part.witness.clone(),
            explanation: only.explanation.clone(),
            reason: part.reason.clone(),
            translation_attempts,
            parts: vec![part.clone()],
        };
    }

    let parts: Vec<PartResult> = checked.into_iter().map(|c| c.part).collect();
    let verdict = bundle_verdict(&parts);
    let any_executed = parts.iter().any(|p| p.engine.is_some());
    CheckResult {
        verdict,
        engine: shared_engine(&parts),
        formal_form: any_executed.then(|| lines(&parts, |p| p.formal_form.as_deref(), true)),
        engine_result: any_executed.then(|| lines(&parts, |p| p.engine_result.as_deref(), true)),
        witness: parts
            .iter()
            .any(|p| p.witness.is_some())
            .then(|| lines(&parts, |p| p.witness.as_deref(), false)),
        explanation: explain(&parts, verdict),
        reason: (verdict == Verdict::NotCheckable).then(|| {
            if parts.is_empty() {
                return NO_PARTS.to_string();
            }
            let declined = lines(&parts, |p| p.reason.as_deref(), false);
            if declined.is_empty() {
                COVERAGE.to_string()
            } else {
                format!("{COVERAGE}\n{declined}")
            }
        }),
        translation_attempts,
        parts,
    }
}

/// Any refuted part refutes the bundle; nothing else is more than
/// `NotCheckable`. A bundle is never `Supported`.
///
/// The translating model chose the parts, and no computation can confirm
/// they cover every statement of the claim: a statement it left out would be
/// reported supported without ever reaching an engine. A refutation needs no
/// such coverage — one false part makes the claim false whatever was omitted.
/// (A one-part claim never reaches this function; `assemble` passes it
/// through.)
fn bundle_verdict(parts: &[PartResult]) -> Verdict {
    if parts.iter().any(|p| p.verdict == Verdict::Refuted) {
        Verdict::Refuted
    } else {
        Verdict::NotCheckable
    }
}

/// The engine, when every part ran on the same one.
fn shared_engine(parts: &[PartResult]) -> Option<Engine> {
    let first = parts.first()?.engine?;
    parts
        .iter()
        .all(|p| p.engine == Some(first))
        .then_some(first)
}

/// One `[n] value` line per part, numbered from 1 in part order.
///
/// Whitespace inside a value is collapsed to single spaces so each part
/// occupies exactly one line (a constraint script spans several; SMT-LIB is
/// whitespace-insensitive and comments are rejected upstream, so the collapsed
/// form is the same script). With `every_part`, a part with no value gets a
/// placeholder line, keeping line n aligned to part n; without it, such parts
/// are skipped and the number alone identifies the part.
fn lines(
    parts: &[PartResult],
    field: impl Fn(&PartResult) -> Option<&str>,
    every_part: bool,
) -> String {
    parts
        .iter()
        .enumerate()
        .filter_map(|(index, part)| {
            let value = match field(part) {
                Some(value) => value.split_whitespace().collect::<Vec<_>>().join(" "),
                None if every_part => NOT_CHECKED.to_string(),
                None => return None,
            };
            Some(format!("[{}] {value}", index + 1))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Deterministic bundle explanation — counts and the rule, never model text.
fn explain(parts: &[PartResult], verdict: Verdict) -> String {
    let count = |wanted: Verdict| parts.iter().filter(|p| p.verdict == wanted).count();
    let rule = if verdict == Verdict::Refuted {
        "At least one part does not hold as formalized, so the claim is refuted."
    } else {
        "No part was refuted. A claim checked in parts is never reported as supported as a \
         whole, because nothing can confirm the parts cover every statement in it."
    };
    format!(
        "The claim was checked as {} parts: {} supported, {} refuted, {} not checked. {rule} \
         See `parts` for each part's verdict, executed form and result.",
        parts.len(),
        count(Verdict::Supported),
        count(Verdict::Refuted),
        count(Verdict::NotCheckable),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn part(verdict: Verdict) -> PartResult {
        let executed = verdict != Verdict::NotCheckable;
        PartResult {
            claim: "c".to_string(),
            verdict,
            engine: executed.then_some(Engine::Arithmetic),
            formal_form: executed.then(|| "1 == 1".to_string()),
            engine_result: executed.then(|| "true".to_string()),
            witness: None,
            reason: (!executed).then(|| "why".to_string()),
        }
    }

    fn verdict_of(verdicts: &[Verdict]) -> Verdict {
        let checked = verdicts
            .iter()
            .map(|v| Checked {
                part: part(*v),
                explanation: "single".to_string(),
            })
            .collect();
        assemble(checked, 1).verdict
    }

    /// A bundle whose parts all hold is NOT supported as a whole: the model
    /// chose the parts, and nothing can confirm they cover every statement of
    /// the claim. A statement it left out would otherwise pass unchecked.
    #[test]
    fn a_bundle_whose_parts_all_hold_is_not_checkable_with_the_coverage_reason() {
        let all_held = |count: usize| {
            let checked = (0..count)
                .map(|_| Checked {
                    part: part(Verdict::Supported),
                    explanation: "single".to_string(),
                })
                .collect();
            assemble(checked, 1)
        };
        for count in [2, 3, 12] {
            let result = all_held(count);
            assert_eq!(result.verdict, Verdict::NotCheckable, "{count} parts");
            assert_eq!(result.reason.as_deref(), Some(COVERAGE), "{count} parts");
            assert!(
                result.explanation.contains("never reported as supported"),
                "{}",
                result.explanation
            );
            // What each part showed is untouched.
            assert!(result.parts.iter().all(|p| p.verdict == Verdict::Supported));
            assert_eq!(result.engine, Some(Engine::Arithmetic));
            assert!(result.formal_form.is_some() && result.engine_result.is_some());
        }
        // One part is a single claim, settled as it always was.
        assert_eq!(all_held(1).verdict, Verdict::Supported);

        // Beside an unchecked part the coverage sentence comes first, then
        // that part's own reason.
        let mixed = assemble(
            vec![
                Checked {
                    part: part(Verdict::Supported),
                    explanation: String::new(),
                },
                Checked {
                    part: part(Verdict::NotCheckable),
                    explanation: String::new(),
                },
            ],
            1,
        );
        assert_eq!(
            mixed.reason.as_deref(),
            Some(format!("{COVERAGE}\n[2] why").as_str())
        );
    }

    /// The whole truth table over two parts: any refuted part refutes, and
    /// nothing else is ever more than `not_checkable`.
    #[test]
    fn the_bundle_verdict_is_refuted_or_not_checkable_never_supported() {
        use Verdict::{NotCheckable as N, Refuted as R, Supported as S};
        for (verdicts, expected) in [
            ([S, S], N),
            ([S, R], R),
            ([R, S], R),
            ([R, R], R),
            ([S, N], N),
            ([N, S], N),
            ([N, N], N),
            ([R, N], R),
            ([N, R], R),
        ] {
            assert_eq!(verdict_of(&verdicts), expected, "{verdicts:?}");
        }
    }

    /// "Every part is supported" is true of no parts at all. Nothing checked
    /// must never read as supported, whoever calls this.
    #[test]
    fn no_parts_at_all_is_not_checkable_never_supported() {
        let result = assemble(Vec::new(), 1);
        assert_eq!(result.verdict, Verdict::NotCheckable);
        assert_eq!(verdict_of(&[]), Verdict::NotCheckable);
        assert!(result.engine.is_none() && result.formal_form.is_none());
        assert!(result.engine_result.is_none() && result.witness.is_none());
        assert_eq!(result.reason.as_deref(), Some(NO_PARTS));
        assert!(
            result.explanation.contains("0 parts"),
            "{}",
            result.explanation
        );
        assert!(result.parts.is_empty());
    }

    #[test]
    fn a_single_part_passes_through_with_its_own_explanation() {
        let result = assemble(
            vec![Checked {
                part: part(Verdict::Refuted),
                explanation: "its own explanation".to_string(),
            }],
            2,
        );
        assert_eq!(result.verdict, Verdict::Refuted);
        assert_eq!(result.explanation, "its own explanation");
        assert_eq!(result.formal_form.as_deref(), Some("1 == 1"));
        assert_eq!(result.translation_attempts, 2);
        assert_eq!(result.parts.len(), 1);
    }

    /// Line n always describes part n, so a multi-line value must not spill.
    #[test]
    fn every_part_gets_exactly_one_numbered_line() {
        let mut script = part(Verdict::Supported);
        script.formal_form = Some("(declare-const x Int)\n  (assert (> x 2))".to_string());
        let parts = [part(Verdict::NotCheckable), script];
        assert_eq!(
            lines(&parts, |p| p.formal_form.as_deref(), true),
            "[1] (not checked)\n[2] (declare-const x Int) (assert (> x 2))"
        );
        // Without placeholders the number still names the part.
        assert_eq!(lines(&parts, |p| p.reason.as_deref(), false), "[1] why");
    }

    #[test]
    fn the_engine_is_named_only_when_every_part_shares_it() {
        let mut solver = part(Verdict::Supported);
        solver.engine = Some(Engine::Constraints);
        assert_eq!(
            shared_engine(&[part(Verdict::Supported), part(Verdict::Refuted)]),
            Some(Engine::Arithmetic)
        );
        assert_eq!(shared_engine(&[part(Verdict::Supported), solver]), None);
        assert_eq!(
            shared_engine(&[part(Verdict::Supported), part(Verdict::NotCheckable)]),
            None
        );
        assert_eq!(
            shared_engine(&[part(Verdict::NotCheckable), part(Verdict::Supported)]),
            None
        );
    }
}
