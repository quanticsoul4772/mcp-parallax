//! The classify+translate hop (research.md 005 D3).
//!
//! One constrained call that either declines (decline-biased — FR-004) or
//! produces a small typed formal target for exactly one engine. Cross-field
//! consistency is enforced by a pure validator; its failures are translation
//! violations, fed to the single retry exactly like an engine parse error
//! (D5).

use crate::deterministic::{
    Polarity, Violation, EXPRESSION_MAX_CHARS, PARTS_MAX, SMTLIB_MAX_CHARS,
};
use crate::error::AppError;
use crate::modes::{CorrectiveMode, ModeRegistry};
use crate::schema::validate;
use crate::traits::client::ModelClient;
use serde::Deserialize;

/// Registry id of the translation mode.
pub const TRANSLATE_MODE_ID: &str = "deterministic_translate";

/// The decline-bias sentence — prompt-borne and pinned by test (FR-004).
pub const DECLINE_BIAS: &str = "When uncertain whether a claim is checkable, decline: a crisp \
answer to the wrong question is worse than no answer.";

/// The evalexpr dialect whitelist — prompt-borne and pinned by test
/// (analysis A1: syntax mistakes must not burn the semantic retry).
pub const EVALEXPR_WHITELIST: &str = "operators + - * / % ^ (exponentiation), comparisons \
< <= > >= == !=, boolean && || !, parentheses, and ONLY these functions: math::abs, min, max, \
floor, ceil, round";

const TRANSLATE_PROMPT_TEMPLATE: &str = "\
You classify and translate one claim for deterministic checking. The claim is \
checkable only if its truth is computable by one of two engines, with no \
judgment, world knowledge, or prediction required. <<decline_bias>>\n\
\n\
<<parts_rule>>\n\
\n\
Engine 'arithmetic': produce a single BOOLEAN-VALUED expression that is true \
exactly when the claim is true. Dialect (use nothing outside it): \
<<whitelist>>. Vague quantities ('roughly', 'about') must become an explicit \
tolerance in the expression (e.g. math::abs(x - y) <= bound); if no \
reasonable bound exists, the claim is not checkable. Arithmetic is exact: \
integers and decimal literals are exact fractions, and / is exact division \
(7/2 is 3.5; write floor(a/b) for integer division, never //), so == and != \
are reliable over + - * / %, integer powers, and floor/ceil/round of those — \
exact percentage and power claims ARE checkable this way; do not decline \
them. Write a claim's own rounding explicitly ('rounds to 62%' is \
round(x*100) == 62; round goes half away from zero). % is the truncated \
remainder: its sign follows the left operand (-7 % 3 is -1). Parenthesise \
chained powers: (a^b)^c or a^(b^c). HARD RULE: a fractional exponent (a \
root, e.g. x^(1/3) or x^0.5) has no exact value — never use == or != on \
anything computed from one; write math::abs(a - b) <= 0.0001 (a literal \
numeric bound, never the word 'tolerance'), or better, restate the claim \
root-free (x^(1/3) > 2 becomes x > 2^3). Call functions with parentheses, \
and write numbers plainly in decimal, with every digit the claim gives and \
no digit grouping (1000, not 1,000). Limits: values up to about 2,400 \
digits, integer exponents up to 4096 (64 on a root), fractional exponents \
up to 64.\n\
\n\
Engine 'constraints': produce an SMT-LIB 2 script — (declare-const ...) for \
each variable over Int, Real, or Bool, then (assert ...) forms using linear \
arithmetic; do NOT emit (check-sat). Set 'stated' to what the claim says \
about the system: 'satisfiable' if the claim says an assignment exists, \
'unsatisfiable' if the claim says none exists. The polarity reflects what \
the claim STATES, never what you compute to be true. Cues: 'there is/are', \
'there exist(s)', 'you can', 'it is possible' assert satisfiable; 'no ... \
exists', 'you cannot', 'it is impossible', 'never' assert unsatisfiable.\n\
\n\
A part mixing a checkable core with judgment may be checked ONLY for the \
core — the formal form is the statement of what was checked; if the core \
cannot stand alone, decline that part. A remark of opinion attached to a \
statement ('..., which is a great improvement') is NOT a part of its own: it \
belongs to that statement's part and is left out of the form. Only a \
judgment the claim asserts as a separate statement becomes its own part.<<violation_clause>>\n\
\n\
Claim: <<claim>>\n\
Context: <<context>>";

/// How a claim is divided into separately checked parts — prompt-borne and
/// pinned by test. The `{max}` slot is filled from [`PARTS_MAX`].
pub const PARTS_RULE_TEMPLATE: &str = "PARTS. A claim may bundle several statements. Split it \
into parts ONLY where the claim asserts every part at once (statements joined by 'and', by \
commas, or as listed items) and each part is a complete statement that is true or false on \
its own. The parts are combined as 'all must hold', so a claim joined any other way — 'or', \
'either', 'if ... then', 'unless', 'only if', 'because', 'so', 'therefore', or a negation \
over several statements such as \
'not both' — stays in ONE part, written as one expression or script. Statements that share \
the variables of one constraint system, or that only make sense together, also stay in ONE \
part. When in doubt, keep one part. At most {max} parts. Every statement the claim makes \
must appear in exactly one part: never leave one out. A statement no engine can check is \
still listed, and declined. Restate each \
part so it stands alone, carrying in any value it needs from an earlier part or the context. \
Each part is classified and translated independently for exactly one engine: a part that \
needs judgment, or a function no engine has, is declined by itself and does not stop the \
other parts from being checked. A list of numbers to add or count is written out in full \
inside the expression (a + b + c ...); there is no summation or counting function. Answer in \
six lists of equal length, one entry per part, in the order the claim states the parts: \
'parts' (the restated part), 'checkable', 'reasons' (why not checkable; empty string when \
checkable), 'engines' ('arithmetic' or 'constraints'; empty string when not checkable), \
'forms' (the expression or the SMT-LIB script; empty string when not checkable), 'stated' \
('satisfiable' or 'unsatisfiable' for a constraints part; empty string otherwise).";

/// The hop's constrained output (flat + closed — Principle II;
/// data-model.md §3).
///
/// Mode schemas admit only scalars and arrays of scalars,
/// so the parts travel as six index-aligned lists, the encoding `decide`
/// uses for its per-option scores.
#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
pub struct TranslateOut {
    /// Each part of the claim, restated so it stands alone. One entry per
    /// part; a claim that is a single statement has exactly one.
    pub parts: Vec<String>,
    /// Per part: whether its truth is computable by an engine (decline-biased).
    pub checkable: Vec<bool>,
    /// Per part: why it is not checkable; empty string when it is.
    pub reasons: Vec<String>,
    /// Per part: "arithmetic" or "constraints"; empty string when not checkable.
    pub engines: Vec<String>,
    /// Per part: the boolean-valued expression (arithmetic) or the SMT-LIB 2
    /// declares + asserts with no check-sat (constraints); empty string when
    /// not checkable.
    pub forms: Vec<String>,
    /// Per part: "satisfiable" or "unsatisfiable" for a constraints part;
    /// empty string otherwise.
    ///
    /// Named `stated` on the wire so it is the last field the model writes:
    /// reply keys are emitted in alphabetical order, and a label written
    /// before the script came back as the model's own conclusion in live
    /// runs instead of what the claim states.
    #[serde(rename = "stated")]
    pub asserted: Vec<String>,
}

/// One part of the claim with its validated translation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// The part as the translator restated it.
    pub text: String,
    /// What to execute for it, or its decline.
    pub translation: Translation,
}

/// A validated translation of one part, ready for exactly one engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Translation {
    /// A decline (a successful outcome — FR-004).
    NotCheckable {
        /// The classifier's stated reason.
        reason: String,
    },
    /// Boolean-valued expression for the arithmetic engine.
    Arithmetic {
        /// The expression.
        expression: String,
    },
    /// SMT-LIB 2 script + polarity for the solver.
    Constraints {
        /// The script (declares + asserts).
        smtlib: String,
        /// What the claim asserts about the system.
        asserted: Polarity,
    },
}

/// One translation attempt's outcome: the claim's parts, or a REAL violation.
///
/// On success every part has a usable translation. The violation, fed to the
/// single retry, carries every part's validation violation together, each
/// labelled with its part.
pub type TranslateAttempt = Result<Vec<Part>, Violation>;

/// Register the translation mode (boot-time; enforces flat+closed).
///
/// # Errors
///
/// Propagates the registry's schema-invariant failure.
pub fn register(registry: &mut ModeRegistry) -> Result<(), AppError> {
    let schema = serde_json::to_value(schemars::schema_for!(TranslateOut))
        .map_err(|e| AppError::ValidationFailure(format!("schema serialization: {e}")))?;
    registry.register(
        TRANSLATE_MODE_ID,
        "internal: deterministic-check translation",
        TRANSLATE_PROMPT_TEMPLATE,
        schema,
        1,
    )
}

/// Build the attempt prompt.
///
/// `violation` is the prior REAL violation fed back verbatim on the retry
/// (D5).
fn build_prompt(
    mode: &CorrectiveMode,
    claim: &str,
    context: Option<&str>,
    violation: Option<&Violation>,
) -> String {
    let violation_clause = violation.map_or(String::new(), |v| {
        format!(
            "\n\nYOUR PREVIOUS TRANSLATION WAS REJECTED by the engine with this exact \
             violation: {v}. Produce a corrected translation of every part (or decline a \
             part that cannot be faithfully formalized)."
        )
    });
    let parts_rule = PARTS_RULE_TEMPLATE.replace("{max}", &PARTS_MAX.to_string());
    fill(
        mode.prompt_template,
        &[
            ("<<decline_bias>>", DECLINE_BIAS),
            ("<<whitelist>>", EVALEXPR_WHITELIST),
            ("<<parts_rule>>", &parts_rule),
            ("<<violation_clause>>", &violation_clause),
            ("<<claim>>", claim),
            ("<<context>>", context.unwrap_or("(none)")),
        ],
    )
}

/// One-pass placeholder substitution: scans only the pristine template, so
/// placeholder-like text inside substituted values (a hostile claim
/// containing `<<context>>`, an engine violation echoing model output) is
/// NEVER re-expanded (review finding: template-injection seam).
fn fill(template: &str, substitutions: &[(&str, &str)]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut rest = template;
    while !rest.is_empty() {
        // The earliest placeholder occurrence wins.
        let next = substitutions
            .iter()
            .filter_map(|(key, value)| rest.find(key).map(|at| (at, *key, *value)))
            .min_by_key(|(at, _, _)| *at);
        let Some((at, key, value)) = next else {
            output.push_str(rest);
            break;
        };
        output.push_str(&rest[..at]);
        output.push_str(value);
        rest = &rest[at + key.len()..];
    }
    output
}

/// One classify+translate call.
///
/// Outer errors are hard failure classes (refusal, timeout, …); the inner
/// [`TranslateAttempt`] distinguishes a usable translation from a retryable
/// violation. Returns token usage too.
///
/// # Errors
///
/// Provider classes from the model call; schema violations on the hop are
/// `ValidationFailure` (the constrained-output contract itself failed).
pub async fn translate_once(
    client: &dyn ModelClient,
    mode: &CorrectiveMode,
    claim: &str,
    context: Option<&str>,
    violation: Option<&Violation>,
) -> Result<(TranslateAttempt, u64, u64), AppError> {
    let prompt = build_prompt(mode, claim, context, violation);
    let completion = client.complete(&prompt, &mode.sanitized_schema).await?;
    validate(&mode.output_schema, &completion.value)?;
    let mut out: TranslateOut = serde_json::from_value(completion.value)
        .map_err(|e| AppError::ValidationFailure(format!("translation shape: {e}")))?;
    // A single claim needs no restating, so an empty text there is the
    // caller's own claim — not a violation that spends the retry on a field a
    // one-claim call never needed. In a bundle the text is the only thing
    // saying which statement a part is, so the validator still rejects it.
    if let [only] = out.parts.as_mut_slice() {
        if only.trim().is_empty() {
            *only = claim.trim().to_string();
        }
    }
    Ok((
        cross_field_validate(&out),
        completion.input_tokens,
        completion.output_tokens,
    ))
}

/// The pure cross-field validator (D3): enforce the consistency the flat
/// schema cannot express. Failures are translation violations (retryable).
fn cross_field_validate(out: &TranslateOut) -> TranslateAttempt {
    let count = out.parts.len();
    for (name, len) in [
        ("checkable", out.checkable.len()),
        ("reasons", out.reasons.len()),
        ("engines", out.engines.len()),
        ("forms", out.forms.len()),
        ("stated", out.asserted.len()),
    ] {
        if len != count {
            return Err(Violation(format!(
                "the six lists must have one entry per part: 'parts' has {count} but \
                 '{name}' has {len}"
            )));
        }
    }
    if count == 0 {
        return Err(Violation(
            "'parts' is empty; a claim has at least one part".to_string(),
        ));
    }
    if count > PARTS_MAX {
        return Err(Violation(format!(
            "the claim was split into {count} parts; the maximum is {PARTS_MAX}"
        )));
    }

    // Every part is validated, and every validation violation is reported, so
    // the single retry is told about all of them at once. (Engine violations
    // are a later stage: no engine runs on a reply rejected here.) A bundle
    // names the part; a one-part claim reads as it always has.
    let mut parts = Vec::with_capacity(count);
    let mut violations = Vec::new();
    for index in 0..count {
        let text = out.parts[index].trim();
        let validated = if text.is_empty() {
            Err(Violation("the part's text is empty".to_string()))
        } else {
            validate_part(
                out.checkable[index],
                &out.reasons[index],
                &out.engines[index],
                &out.forms[index],
                &out.asserted[index],
                count > 1,
            )
        };
        match validated {
            Ok(translation) => parts.push(Part {
                text: text.to_string(),
                translation,
            }),
            Err(violation) => violations.push(part_violation(index, count, text, &violation)),
        }
    }
    if violations.is_empty() {
        Ok(parts)
    } else {
        Err(Violation(violations.join("; ")))
    }
}

/// How much of a part's text a violation label quotes.
const PART_LABEL_MAX_CHARS: usize = 80;

/// Label a violation with its 1-based part number and the part's own text
/// when the claim has several parts; a one-part claim's violation is passed
/// through unchanged.
///
/// The text is quoted because a number alone refers to a split the retry is
/// not shown: a solver violation does not echo its script, so "part 2" would
/// leave the model to reproduce its earlier split to learn what to fix.
#[must_use]
pub fn part_violation(index: usize, count: usize, text: &str, violation: &Violation) -> String {
    if count <= 1 {
        return violation.0.clone();
    }
    let text = text.trim();
    let quoted: String = text.chars().take(PART_LABEL_MAX_CHARS).collect();
    let cut = if text.chars().count() > PART_LABEL_MAX_CHARS {
        "…"
    } else {
        ""
    };
    format!("part {} (\"{quoted}{cut}\"): {violation}", index + 1)
}

/// Validate one part's fields into a translation for exactly one engine.
fn validate_part(
    checkable: bool,
    reason: &str,
    engine: &str,
    form: &str,
    asserted: &str,
    in_bundle: bool,
) -> Result<Translation, Violation> {
    if !checkable {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(Violation(
                "checkable=false requires a non-empty reason".to_string(),
            ));
        }
        // In a bundle a declined part carries nothing for an engine, and
        // leftovers are not dropped: they are the cheapest sign that the
        // index-aligned lists have slipped by a position. A one-part reply
        // has nothing to slip against, so its leftovers are ignored and the
        // decline stands — as it did before parts existed.
        for (name, value) in [("engines", engine), ("forms", form), ("stated", asserted)] {
            if in_bundle && !value.trim().is_empty() {
                return Err(Violation(format!(
                    "checkable=false requires an empty '{name}' entry; got {value:?} — check \
                     that the six lists line up part by part"
                )));
            }
        }
        return Ok(Translation::NotCheckable {
            reason: reason.to_string(),
        });
    }
    let form = form.trim();
    match engine {
        "arithmetic" => {
            if form.is_empty() {
                return Err(Violation(
                    "engine=arithmetic requires a non-empty form (the expression)".to_string(),
                ));
            }
            if form.chars().count() > EXPRESSION_MAX_CHARS {
                return Err(Violation(format!(
                    "the arithmetic expression exceeds {EXPRESSION_MAX_CHARS} characters"
                )));
            }
            Ok(Translation::Arithmetic {
                expression: form.to_string(),
            })
        }
        "constraints" => {
            let smtlib = form;
            if smtlib.is_empty() {
                return Err(Violation(
                    "engine=constraints requires a non-empty form (the SMT-LIB script)".to_string(),
                ));
            }
            if smtlib.chars().count() > SMTLIB_MAX_CHARS {
                return Err(Violation(format!(
                    "the SMT-LIB script exceeds {SMTLIB_MAX_CHARS} characters"
                )));
            }
            if smtlib.contains('\0') {
                return Err(Violation(
                    "the SMT-LIB script contains a NUL byte".to_string(),
                ));
            }
            if crate::deterministic::solver::count_asserts(smtlib) == 0 {
                return Err(Violation(
                    "the SMT-LIB script contains no (assert ...) form".to_string(),
                ));
            }
            // The prompt restricts scripts to declares + asserts; comments
            // and stateful commands would desynchronize the parse-detection
            // count (review finding) and are rejected up front.
            for forbidden in [
                ";",
                "(check-sat",
                "(push",
                "(pop",
                "(reset",
                "(set-option",
                "(assert-soft",
            ] {
                if smtlib.contains(forbidden) {
                    return Err(Violation(format!(
                        "the SMT-LIB script must contain only (declare-const ...) and (assert ...) forms; {forbidden:?} is not allowed"
                    )));
                }
            }
            let asserted = match asserted {
                "satisfiable" => Polarity::Satisfiable,
                "unsatisfiable" => Polarity::Unsatisfiable,
                other => {
                    return Err(Violation(format!(
                        "engine=constraints requires 'stated' to be exactly \"satisfiable\" or \"unsatisfiable\"; got {other:?}"
                    )))
                }
            };
            Ok(Translation::Constraints {
                smtlib: smtlib.to_string(),
                asserted,
            })
        }
        "" => Err(Violation(
            "checkable=true requires an engine choice".to_string(),
        )),
        other => Err(Violation(format!(
            "engine must be exactly \"arithmetic\" or \"constraints\"; got {other:?}"
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::traits::client::{Completion, MockModelClient};
    use serde_json::json;

    fn mode() -> CorrectiveMode {
        let mut registry = ModeRegistry::new();
        register(&mut registry).unwrap();
        registry.get(TRANSLATE_MODE_ID).unwrap().clone()
    }

    fn client_returning(value: serde_json::Value) -> MockModelClient {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(move |_, _| {
            Ok(Completion {
                value: value.clone(),
                input_tokens: 10,
                output_tokens: 5,
            })
        });
        client
    }

    /// One part of a stand-in reply: (text, checkable, reason, engine, form,
    /// polarity — sent as `stated`).
    type PartSpec<'a> = (&'a str, bool, &'a str, &'a str, &'a str, &'a str);

    /// A reply in the index-aligned wire shape.
    fn reply(parts: &[PartSpec<'_>]) -> serde_json::Value {
        json!({
            "parts": parts.iter().map(|p| p.0).collect::<Vec<_>>(),
            "checkable": parts.iter().map(|p| p.1).collect::<Vec<_>>(),
            "reasons": parts.iter().map(|p| p.2).collect::<Vec<_>>(),
            "engines": parts.iter().map(|p| p.3).collect::<Vec<_>>(),
            "forms": parts.iter().map(|p| p.4).collect::<Vec<_>>(),
            "stated": parts.iter().map(|p| p.5).collect::<Vec<_>>(),
        })
    }

    /// The parts rule sits before the engine paragraphs. Placed after the
    /// polarity instruction it separated that instruction from the claim, and
    /// the live model labelled polarity with its own conclusion in every run.
    #[test]
    fn the_parts_rule_precedes_the_polarity_instruction() {
        let prompt = build_prompt(&mode(), "c", None, None);
        let rule = prompt.find("PARTS. A claim may bundle").unwrap();
        let first_engine = prompt.find("Engine 'arithmetic':").unwrap();
        let polarity = prompt
            .find("The polarity reflects what the claim STATES")
            .unwrap();
        assert!(rule < first_engine, "rule after an engine paragraph");
        assert!(first_engine < polarity, "{prompt}");

        // Nothing but the mixed-judgment paragraph may sit between the
        // polarity paragraph and the claim: the live evidence was gathered
        // with exactly this text there.
        let polarity_end = "'never' assert unsatisfiable.";
        let from = prompt.find(polarity_end).unwrap() + polarity_end.len();
        let to = prompt.find("\n\nClaim: c\n").unwrap();
        assert_eq!(
            &prompt[from..to],
            "\n\nA part mixing a checkable core with judgment may be checked ONLY for the \
             core — the formal form is the statement of what was checked; if the core \
             cannot stand alone, decline that part. A remark of opinion attached to a \
             statement ('..., which is a great improvement') is NOT a part of its own: it \
             belongs to that statement's part and is left out of the form. Only a \
             judgment the claim asserts as a separate statement becomes its own part."
        );
    }

    /// The model-hop table in the data model lists exactly the reply shape's
    /// fields, by their wire names. Nothing else ties that table to the code.
    #[test]
    fn the_data_model_table_names_the_reply_fields() {
        let doc = include_str!("../../specs/005-deterministic-layer/data-model.md");
        let start = doc.find("## 3. Model-hop schema").expect("section 3");
        let end = doc.find("## 4. ").expect("section 4");
        assert!(start < end);
        let mut documented: Vec<String> = doc[start..end]
            .lines()
            .filter(|line| line.starts_with('|'))
            .map(|line| line.split('|').nth(1).unwrap().trim().to_string())
            .filter(|cell| cell != "field" && !cell.starts_with("---"))
            .collect();
        documented.sort();
        let schema = serde_json::to_value(schemars::schema_for!(TranslateOut)).unwrap();
        let mut fields: Vec<String> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        fields.sort();
        assert_eq!(fields.len(), 6, "{fields:?}");
        assert_eq!(documented, fields);
    }

    /// The polarity field's description is model-facing text: the doc comment
    /// on the field is what the model reads. The live acceptance evidence was
    /// gathered with exactly this text, so a change to it must be deliberate
    /// and re-measured live.
    #[test]
    fn the_polarity_description_sent_to_the_model_is_pinned() {
        let sent = &mode().sanitized_schema["properties"]["stated"]["description"];
        assert_eq!(
            sent.as_str().unwrap(),
            "Per part: \"satisfiable\" or \"unsatisfiable\" for a constraints part;\n\
             empty string otherwise.\n\n\
             Named `stated` on the wire so it is the last field the model writes:\n\
             reply keys are emitted in alphabetical order, and a label written\n\
             before the script came back as the model's own conclusion in live\n\
             runs instead of what the claim states."
        );
    }

    /// A rejection is sent back to the model on the retry, so it must name
    /// the polarity field as the reply shape names it.
    #[test]
    fn rejections_name_the_polarity_field_as_the_reply_does() {
        let missing =
            cross_field_validate(&out(&[("c", true, "", "constraints", "(assert true)", "")]))
                .unwrap_err();
        let leftover = cross_field_validate(&out(&[
            SUM,
            ("c", false, "judgment", "", "", "satisfiable"),
        ]))
        .unwrap_err();
        let mut short = out(&[SUM, SUM]);
        short.asserted.pop();
        let uneven = cross_field_validate(&short).unwrap_err();
        for violation in [missing, leftover, uneven] {
            assert!(
                violation.0.contains("'stated'") && !violation.0.contains("asserted"),
                "{violation}"
            );
        }
    }

    /// Reply keys are emitted alphabetically, so the polarity key must sort
    /// after every other key: the model then writes the script first and the
    /// label after it.
    #[test]
    fn the_polarity_field_is_the_last_one_written() {
        // The shape as sent to the provider, not the raw derived one.
        let schema = mode().sanitized_schema;
        let keys: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        assert_eq!(keys.last().map(|k| k.as_str()), Some("stated"), "{keys:?}");
        let prompt = build_prompt(&mode(), "c", None, None);
        assert!(prompt.contains("'stated'") && !prompt.contains("'asserted'"));
    }

    fn out(parts: &[PartSpec<'_>]) -> TranslateOut {
        serde_json::from_value(reply(parts)).unwrap()
    }

    const DECLINED: PartSpec<'static> = ("the claim", false, "judgment", "", "", "");
    const SUM: PartSpec<'static> = ("1 + 1 is 2", true, "", "arithmetic", "1 + 1 == 2", "");

    // Analysis A1 + FR-004: the prompt-borne guarantees are pinned.
    #[tokio::test]
    async fn the_prompt_carries_the_decline_bias_and_the_whitelist_verbatim() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|prompt, _| {
            assert!(prompt.contains(DECLINE_BIAS), "decline bias missing");
            assert!(prompt.contains(EVALEXPR_WHITELIST), "whitelist missing");
            assert!(prompt.contains("Claim: the claim text"));
            Ok(Completion {
                value: reply(&[DECLINED]),
                input_tokens: 1,
                output_tokens: 1,
            })
        });
        let (attempt, _, _) = translate_once(&client, &mode(), "the claim text", None, None)
            .await
            .unwrap();
        assert_eq!(
            attempt.unwrap(),
            [Part {
                text: "the claim".into(),
                translation: Translation::NotCheckable {
                    reason: "judgment".into()
                }
            }]
        );
    }

    /// The parts rule reaches the model with the real limit filled in, and no
    /// placeholder survives into the prompt.
    #[tokio::test]
    async fn the_prompt_carries_the_parts_rule_with_the_limit_filled_in() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|prompt, _| {
            let rule = PARTS_RULE_TEMPLATE.replace("{max}", &PARTS_MAX.to_string());
            assert!(prompt.contains(&rule), "parts rule missing: {prompt}");
            assert!(prompt.contains(&format!("At most {PARTS_MAX} parts.")));
            assert!(
                !prompt.contains("{max}") && !prompt.contains("<<"),
                "{prompt}"
            );
            Ok(Completion {
                value: reply(&[DECLINED]),
                input_tokens: 1,
                output_tokens: 1,
            })
        });
        translate_once(&client, &mode(), "c", None, None)
            .await
            .unwrap()
            .0
            .unwrap();
    }

    /// Parts are combined as a conjunction, so the model must be told not to
    /// split a claim joined any other way: a split "either A or B" with A
    /// false would come back refuted although the claim is true.
    #[test]
    fn the_parts_rule_confines_splitting_to_conjunctions() {
        let prompt = build_prompt(&mode(), "c", None, None);
        assert!(
            prompt.contains("The parts are combined as 'all must hold'"),
            "{prompt}"
        );
        for connective in [
            "'or'",
            "'if ... then'",
            "'unless'",
            "'not both'",
            "'because'",
            "'so'",
            "'therefore'",
        ] {
            assert!(prompt.contains(connective), "{connective} missing");
        }
    }

    /// The model is told to list every statement, because the server can only
    /// check the parts it is given. (The server does not rely on this: a
    /// bundle is never reported supported as a whole — see `bundle.rs`.)
    #[test]
    fn the_parts_rule_requires_every_statement_to_be_listed() {
        let prompt = build_prompt(&mode(), "c", None, None);
        assert!(
            prompt.contains(
                "Every statement the claim makes must appear in exactly one part: never \
                 leave one out. A statement no engine can check is still listed, and declined."
            ),
            "{prompt}"
        );
    }

    /// An opinion attached to a computable statement must not become its own
    /// part: that would turn a claim `main` settles as supported into
    /// `not_checkable`, and leave the outcome to how the model happened to
    /// split it.
    #[test]
    fn an_attached_opinion_is_not_a_part_of_its_own() {
        let prompt = build_prompt(&mode(), "c", None, None);
        assert!(prompt.contains("is NOT a part of its own"), "{prompt}");
        assert!(
            prompt.contains("asserts as a separate statement becomes its own part"),
            "{prompt}"
        );
    }

    #[tokio::test]
    async fn the_retry_prompt_carries_the_violation_verbatim() {
        let mut client = MockModelClient::new();
        client.expect_complete().times(1).returning(|prompt, _| {
            assert!(
                prompt.contains(
                    "REJECTED by the engine with this exact violation: parse \
                                 error at token 3"
                ),
                "{prompt}"
            );
            // A bundle is corrected part by part, not declined whole.
            assert!(
                prompt.contains(
                    "Produce a corrected translation of every part (or decline a part \
                     that cannot be faithfully formalized)."
                ),
                "{prompt}"
            );
            Ok(Completion {
                value: reply(&[SUM]),
                input_tokens: 1,
                output_tokens: 1,
            })
        });
        let violation = Violation("parse error at token 3".into());
        let (attempt, _, _) = translate_once(&client, &mode(), "c", None, Some(&violation))
            .await
            .unwrap();
        let parts = attempt.unwrap();
        assert!(matches!(
            parts.as_slice(),
            [Part {
                translation: Translation::Arithmetic { .. },
                ..
            }]
        ));
    }

    #[tokio::test]
    async fn clean_constraint_translation_passes_the_cross_field_validator() {
        let client = client_returning(reply(&[(
            "some x is positive",
            true,
            "",
            "constraints",
            "(declare-const x Int)\n(assert (> x 0))",
            "satisfiable",
        )]));
        let (attempt, input, output) = translate_once(&client, &mode(), "c", Some("ctx"), None)
            .await
            .unwrap();
        let parts = attempt.unwrap();
        assert!(matches!(
            parts.as_slice(),
            [Part {
                translation: Translation::Constraints {
                    asserted: Polarity::Satisfiable,
                    ..
                },
                ..
            }]
        ));
        assert_eq!((input, output), (10, 5));
    }

    #[tokio::test]
    async fn cross_field_violations_are_retryable_not_hard_errors() {
        for (part, marker) in [
            (("c", true, "", "", "", ""), "requires an engine"),
            (("c", true, "", "sympy", "x", ""), "engine must be exactly"),
            (
                ("c", true, "", "arithmetic", "  ", ""),
                "requires a non-empty form",
            ),
            (
                (
                    "c",
                    true,
                    "",
                    "constraints",
                    "(declare-const x Int)",
                    "satisfiable",
                ),
                "no (assert",
            ),
            (
                (
                    "c",
                    true,
                    "",
                    "constraints",
                    "(assert true)(check-sat)",
                    "satisfiable",
                ),
                "is not allowed",
            ),
            (
                ("c", true, "", "constraints", "(assert true)", ""),
                "requires 'stated'",
            ),
            (("c", false, "  ", "", "", ""), "non-empty reason"),
        ] {
            let client = client_returning(reply(&[part]));
            let (attempt, _, _) = translate_once(&client, &mode(), "c", None, None)
                .await
                .unwrap();
            let violation = attempt.unwrap_err();
            assert!(violation.0.contains(marker), "{marker}: {violation}");
            // A one-part claim's violation carries no part label.
            assert!(!violation.0.contains("part 1"), "{violation}");
        }
    }

    #[test]
    fn oversized_targets_are_violations() {
        let long_expression = "1".repeat(EXPRESSION_MAX_CHARS + 1);
        let big_expr =
            cross_field_validate(&out(&[("c", true, "", "arithmetic", &long_expression, "")]));
        assert!(big_expr.unwrap_err().0.contains("exceeds"));

        let long_script = "(assert true)".repeat(SMTLIB_MAX_CHARS / 10);
        let big_smt = cross_field_validate(&out(&[(
            "c",
            true,
            "",
            "constraints",
            &long_script,
            "satisfiable",
        )]));
        assert!(big_smt.unwrap_err().0.contains("exceeds"));
    }

    // ---- Parts -------------------------------------------------------------

    #[test]
    fn a_bundle_yields_one_translation_per_part_in_order() {
        let parts = cross_field_validate(&out(&[
            SUM,
            ("it is elegant", false, "a judgment", "", "", ""),
            (
                "no x is below itself",
                true,
                "",
                "constraints",
                "(declare-const x Int)\n(assert (< x x))",
                "unsatisfiable",
            ),
        ]))
        .unwrap();
        assert_eq!(
            parts.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(),
            ["1 + 1 is 2", "it is elegant", "no x is below itself"]
        );
        assert!(matches!(
            parts[0].translation,
            Translation::Arithmetic { .. }
        ));
        assert!(matches!(
            parts[1].translation,
            Translation::NotCheckable { .. }
        ));
        assert!(matches!(
            parts[2].translation,
            Translation::Constraints {
                asserted: Polarity::Unsatisfiable,
                ..
            }
        ));
    }

    /// Every bad part is named, so the single retry hears about all of them.
    #[test]
    fn a_bundle_reports_every_violating_part_by_number() {
        let violation = cross_field_validate(&out(&[
            ("a", true, "", "arithmetic", "", ""),
            SUM,
            ("c", false, "", "", "", ""),
        ]))
        .unwrap_err();
        assert!(
            violation.0.contains("part 1 (\"a\"): engine=arithmetic")
                && violation.0.contains("part 3 (\"c\"): checkable=false"),
            "{violation}"
        );
        assert!(!violation.0.contains("part 2"), "{violation}");
    }

    /// A long part is quoted up to the limit and marked as cut, counting
    /// characters rather than bytes.
    #[test]
    fn a_part_label_quotes_at_most_the_limit() {
        let violation = Violation("bad".into());
        let long = "é".repeat(PART_LABEL_MAX_CHARS + 5);
        assert_eq!(
            part_violation(1, 2, &long, &violation),
            format!("part 2 (\"{}…\"): bad", "é".repeat(PART_LABEL_MAX_CHARS))
        );
        let exact = "x".repeat(PART_LABEL_MAX_CHARS);
        assert_eq!(
            part_violation(0, 2, &exact, &violation),
            format!("part 1 (\"{exact}\"): bad")
        );
        // One part: no label at all.
        assert_eq!(part_violation(0, 1, "anything", &violation), "bad");
    }

    /// A declined part that still carries an engine, a form or a polarity is
    /// the cheapest sign that the six lists have slipped out of line, so it is
    /// reported rather than dropped.
    #[test]
    fn a_declined_part_carrying_engine_fields_is_a_violation_naming_the_field() {
        for (part, field) in [
            (("c", false, "judgment", "arithmetic", "", ""), "engines"),
            (("c", false, "judgment", "", "1 == 2", ""), "forms"),
            (("c", false, "judgment", "", "", "satisfiable"), "stated"),
        ] {
            let violation = cross_field_validate(&out(&[SUM, part])).unwrap_err();
            assert!(
                violation.0.contains("part 2 (\"c\"): checkable=false")
                    && violation.0.contains(&format!("'{field}'")),
                "{field}: {violation}"
            );

            // With ONE part there is nothing for a list to slip against, and
            // `main` ignored such leftovers: the decline stands, in one
            // attempt, exactly as it always did.
            let alone = cross_field_validate(&out(&[part])).unwrap();
            assert_eq!(
                alone[0].translation,
                Translation::NotCheckable {
                    reason: "judgment".into()
                },
                "{field}"
            );
        }
        // Whitespace is not content: a clean decline still passes.
        let clean = cross_field_validate(&out(&[("c", false, "judgment", " ", "\n", "")]));
        assert!(matches!(
            clean.unwrap().as_slice(),
            [Part {
                translation: Translation::NotCheckable { .. },
                ..
            }]
        ));

        // The reviewer's case: lists shifted so part 1's form sits on a
        // declined part. Named by part, not absorbed.
        let shifted = cross_field_validate(&out(&[
            ("a", false, "judgment", "arithmetic", "1 == 2", ""),
            ("b", true, "", "arithmetic", "1 == 1", ""),
        ]))
        .unwrap_err();
        assert!(shifted.0.contains("part 1 (\"a\"): "), "{shifted}");
    }

    /// A single claim needs no restating: an empty text is filled with the
    /// caller's own claim rather than spending the retry on a field `main`
    /// never had. In a bundle the text is the only thing saying which
    /// statement a part is, so there it stays a violation.
    #[tokio::test]
    async fn an_empty_text_is_the_callers_claim_for_one_part_and_a_violation_in_a_bundle() {
        let client = client_returning(reply(&[("  ", true, "", "arithmetic", "1 == 1", "")]));
        let (attempt, _, _) = translate_once(&client, &mode(), " the caller's claim ", None, None)
            .await
            .unwrap();
        let parts = attempt.unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].text, "the caller's claim");
        assert!(matches!(
            parts[0].translation,
            Translation::Arithmetic { .. }
        ));

        let client = client_returning(reply(&[SUM, ("", true, "", "arithmetic", "1 == 1", "")]));
        let (attempt, _, _) = translate_once(&client, &mode(), "the caller's claim", None, None)
            .await
            .unwrap();
        let violation = attempt.unwrap_err();
        assert!(
            violation
                .0
                .contains("part 2 (\"\"): the part's text is empty"),
            "{violation}"
        );
    }

    #[test]
    fn lists_of_unequal_length_are_a_violation_naming_the_short_list() {
        let mut uneven = out(&[SUM, SUM]);
        uneven.forms.pop();
        let violation = cross_field_validate(&uneven).unwrap_err();
        assert!(
            violation.0.contains("'parts' has 2") && violation.0.contains("'forms' has 1"),
            "{violation}"
        );
    }

    #[test]
    fn the_part_count_is_bounded_at_both_ends() {
        let empty = cross_field_validate(&out(&[])).unwrap_err();
        assert!(empty.0.contains("at least one part"), "{empty}");

        let at_limit = vec![SUM; PARTS_MAX];
        assert_eq!(
            cross_field_validate(&out(&at_limit)).unwrap().len(),
            PARTS_MAX
        );

        let over = vec![SUM; PARTS_MAX + 1];
        let violation = cross_field_validate(&out(&over)).unwrap_err();
        assert!(
            violation.0.contains(&format!(
                "{} parts; the maximum is {PARTS_MAX}",
                PARTS_MAX + 1
            )),
            "{violation}"
        );
    }
}
