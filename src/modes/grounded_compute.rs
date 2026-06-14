//! The compute-settle unit (011) — pure, deterministic, no IO.
//!
//! When `grounded_verify`'s passes flag a claim computable
//! ([`super::grounded_verify::GroundedPass::needs_computation`]) and agree on an
//! in-class spec, this module counts the property over the verbatim source the
//! server already read and settles the comparison with the deterministic
//! arithmetic engine ([`crate::deterministic::arithmetic`]) — the model names
//! *what* to count, never the value or the verdict. Everything here is a pure
//! function over [`GroundedPass`] / [`RawUnit`]; the orchestration and the
//! abstain fallbacks live in `grounded_verify::aggregate`.

use crate::deterministic::arithmetic;
use crate::grounded::assemble::RawUnit;
use crate::modes::grounded_verify::{GroundedPass, GroundedVerdictKind};

/// An in-class computable property of a single source (011).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Property {
    /// Number of lines.
    Lines,
    /// Byte/size of the source.
    Bytes,
    /// Count of a literal string.
    Matches(String),
}

/// A numeric comparison operator (011).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Gt,
    Ge,
    Lt,
    Le,
    Eq,
    Ne,
}

impl Op {
    /// The evalexpr-legal operator text.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }

    /// Server-side validation of the model-supplied operator string.
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            ">" => Self::Gt,
            ">=" => Self::Ge,
            "<" => Self::Lt,
            "<=" => Self::Le,
            "==" => Self::Eq,
            "!=" => Self::Ne,
            _ => return None,
        })
    }
}

/// The agreed computation to run over the single source (011).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComputeSpec {
    property: Property,
    operator: Op,
    threshold: i64,
}

impl ComputeSpec {
    /// Build a spec from one pass's compute fields — server-validating the
    /// property/operator strings (analyze H1: nullable strings, not enums).
    /// `None` if any field is missing or out of the supported class.
    fn from_pass(pass: &GroundedPass) -> Option<Self> {
        let property = match pass.compute_property.as_deref()? {
            "lines" => Property::Lines,
            "bytes" => Property::Bytes,
            "matches" => Property::Matches(
                pass.compute_match_literal
                    .clone()
                    .filter(|l| !l.is_empty())?,
            ),
            _ => return None,
        };
        Some(Self {
            property,
            operator: Op::parse(pass.compute_operator.as_deref()?)?,
            threshold: pass.compute_threshold?,
        })
    }
}

/// The spec held by a strict majority of the `needs_computation` passes, if any
/// (T003). Disagreement, an out-of-class string, or a missing field → `None`.
pub(crate) fn agreed_spec(needs_computation_passes: &[&GroundedPass]) -> Option<ComputeSpec> {
    let specs: Vec<ComputeSpec> = needs_computation_passes
        .iter()
        .filter_map(|p| ComputeSpec::from_pass(p))
        .collect();
    let n = needs_computation_passes.len();
    specs
        .iter()
        .find(|cand| specs.iter().filter(|s| s == cand).count() * 2 > n)
        .cloned()
}

/// Count the property over one raw source unit (T004) — deterministic, server-side.
/// `Lines`: newline count plus one for a non-empty unterminated final line
/// (empty source → 0). `Bytes`: the reader's byte length. `Matches`:
/// non-overlapping literal occurrences.
fn count_property(property: &Property, unit: &RawUnit) -> i64 {
    match property {
        Property::Bytes => i64::try_from(unit.bytes).unwrap_or(i64::MAX),
        Property::Lines => {
            if unit.text.is_empty() {
                return 0;
            }
            let newlines = unit.text.matches('\n').count();
            let unterminated = usize::from(!unit.text.ends_with('\n'));
            i64::try_from(newlines + unterminated).unwrap_or(i64::MAX)
        }
        Property::Matches(literal) => {
            i64::try_from(unit.text.matches(literal.as_str()).count()).unwrap_or(i64::MAX)
        }
    }
}

/// The outcome of a settled compute claim: verdict, executed form, raw result,
/// and a human-readable count note. Fields are read by `grounded_verify::aggregate`.
pub(crate) struct Settled {
    pub(crate) verdict: GroundedVerdictKind,
    pub(crate) executed_form: String,
    pub(crate) engine_result: String,
    pub(crate) note: String,
}

/// Count the property and settle the comparison via the deterministic arithmetic
/// engine (T006, FR-007) — `check`'s engine reused, the model translator bypassed
/// because the value is server-counted. `None` (→ abstain) on an engine error.
pub(crate) fn settle(spec: &ComputeSpec, unit: &RawUnit) -> Option<Settled> {
    let value = count_property(&spec.property, unit);
    let executed_form = format!("{value} {} {}", spec.operator.as_str(), spec.threshold);
    let outcome = arithmetic::evaluate(&executed_form).ok()?;
    let verdict = if outcome.holds {
        GroundedVerdictKind::Supported
    } else {
        GroundedVerdictKind::Refuted
    };
    let note = match &spec.property {
        Property::Lines => format!("counted {value} lines"),
        Property::Bytes => format!("measured {value} bytes"),
        Property::Matches(literal) => format!("counted {value} occurrences of \"{literal}\""),
    };
    Some(Settled {
        verdict,
        executed_form,
        engine_result: outcome.result_text,
        note,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::modes::verify::VerdictKind;

    /// A `GroundedPass` carrying the given compute fields (needs_computation set).
    fn compute_pass(
        property: Option<&str>,
        op: Option<&str>,
        threshold: Option<i64>,
        literal: Option<&str>,
    ) -> GroundedPass {
        GroundedPass {
            verdict: VerdictKind::Supported,
            findings: vec![],
            missing_evidence: vec![],
            needs_computation: true,
            compute_property: property.map(str::to_string),
            compute_match_literal: literal.map(str::to_string),
            compute_operator: op.map(str::to_string),
            compute_threshold: threshold,
        }
    }

    #[test]
    fn line_count_convention_lf_terminated_and_unterminated() {
        let lf = RawUnit {
            text: "a\nb\nc\n".to_string(),
            bytes: 6,
        };
        assert_eq!(count_property(&Property::Lines, &lf), 3);
        let no_nl = RawUnit {
            text: "a\nb\nc".to_string(),
            bytes: 5,
        };
        assert_eq!(count_property(&Property::Lines, &no_nl), 3);
        let empty = RawUnit {
            text: String::new(),
            bytes: 0,
        };
        assert_eq!(count_property(&Property::Lines, &empty), 0);
    }

    #[test]
    fn byte_and_match_counts() {
        let unit = RawUnit {
            text: "foo bar foo baz foo".to_string(),
            bytes: 19,
        };
        assert_eq!(count_property(&Property::Bytes, &unit), 19);
        assert_eq!(
            count_property(&Property::Matches("foo".to_string()), &unit),
            3
        );
    }

    #[test]
    fn compute_spec_validates_property_and_operator_strings() {
        assert_eq!(
            ComputeSpec::from_pass(&compute_pass(Some("lines"), Some(">"), Some(1000), None)),
            Some(ComputeSpec {
                property: Property::Lines,
                operator: Op::Gt,
                threshold: 1000
            })
        );
        // An unrecognized property string is out-of-class.
        assert_eq!(
            ComputeSpec::from_pass(&compute_pass(Some("functions"), Some(">"), Some(5), None)),
            None
        );
        // An unrecognized operator is out-of-class.
        assert_eq!(
            ComputeSpec::from_pass(&compute_pass(Some("lines"), Some("=>"), Some(5), None)),
            None
        );
        // `matches` requires a non-empty literal.
        assert_eq!(
            ComputeSpec::from_pass(&compute_pass(Some("matches"), Some(">"), Some(1), Some(""))),
            None
        );
        // A missing field is out-of-class.
        assert_eq!(
            ComputeSpec::from_pass(&compute_pass(Some("lines"), None, Some(1), None)),
            None
        );
    }

    #[test]
    fn agreed_spec_needs_a_strict_majority() {
        let p1 = compute_pass(Some("lines"), Some(">"), Some(1000), None);
        let p2 = compute_pass(Some("lines"), Some(">"), Some(1000), None);
        let p3 = compute_pass(Some("bytes"), Some("<"), Some(50), None);
        // 2 of 3 agree → that spec.
        assert!(agreed_spec(&[&p1, &p2, &p3]).is_some());
        // 1 of 2 each → no majority.
        assert_eq!(agreed_spec(&[&p1, &p3]), None);
    }

    #[test]
    fn settle_decides_via_the_engine_over_the_counted_value() {
        let unit = RawUnit {
            text: "x\n".repeat(1224),
            bytes: 2448,
        };
        let spec = ComputeSpec {
            property: Property::Lines,
            operator: Op::Gt,
            threshold: 1000,
        };
        let settled = settle(&spec, &unit).expect("settles");
        assert_eq!(settled.verdict, GroundedVerdictKind::Supported);
        assert_eq!(settled.executed_form, "1224 > 1000");
        assert_eq!(settled.engine_result, "true");
        assert_eq!(settled.note, "counted 1224 lines");
    }
}
