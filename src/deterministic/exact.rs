//! Exact evaluation of the arithmetic dialect.
//!
//! evalexpr parses; this module evaluates its tree. Integers and decimal
//! literals are arbitrary-precision fractions, so `+ - * / %`, integer
//! powers, and every comparison over them are exact: `90/144` is `5/8`,
//! `0.15 * 240 == 36` holds, and integers above 2^53 keep every digit.
//!
//! Only a fractional exponent (a root) has no exact result. It is computed in
//! `f64` and the value is marked approximate; `==`/`!=` over an approximate
//! value is rejected, and an ordering comparison too close to decide is
//! rejected rather than answered. Anything outside the dialect is rejected.

use evalexpr::{DefaultNumericTypes, Node, Operator, Value};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Pow, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;

/// Largest integer exponent evaluated exactly.
const EXACT_EXPONENT_MAX: i32 = 4_096;
/// Largest exact power, in bits of numerator or denominator.
const EXACT_POWER_MAX_BITS: u64 = 1 << 20;
/// Most significant digits a non-integer literal may carry: beyond this the
/// decimal cannot be recovered exactly from the `f64` evalexpr parsed it to.
const DECIMAL_LITERAL_MAX_DIGITS: usize = 15;
/// Two approximate values closer than this (relative to their magnitude)
/// cannot be ordered reliably in `f64`.
const NEAR_TIE: f64 = 1e-9;

/// A number: exact, or an `f64` approximation of an irrational result.
#[derive(Debug, Clone)]
enum Num {
    Exact(BigRational),
    Approx(f64),
}

#[derive(Debug, Clone)]
enum Val {
    Num(Num),
    Bool(bool),
}

/// Evaluate `expression`; the error is the reason the engine rejected it.
pub(super) fn evaluate(expression: &str) -> Result<Outcome, String> {
    check_literals(expression)?;
    let tree = evalexpr::build_operator_tree::<DefaultNumericTypes>(expression)
        .map_err(|e| e.to_string())?;
    match eval(&tree)? {
        Val::Bool(holds) => Ok(Outcome::Boolean(holds)),
        Val::Num(Num::Exact(n)) => Ok(Outcome::NonBoolean(n.to_string())),
        Val::Num(Num::Approx(f)) => Ok(Outcome::NonBoolean(f.to_string())),
    }
}

/// What a well-formed expression evaluated to.
pub(super) enum Outcome {
    Boolean(bool),
    /// A number, printed — the caller reports it as a non-boolean result.
    NonBoolean(String),
}

/// Reject numeric literals evalexpr would round before this module sees
/// them: an integer beyond `i64` (parsed as a float) or a decimal with more
/// significant digits than an `f64` round-trips.
fn check_literals(expression: &str) -> Result<(), String> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.';
    let mut rest = expression;
    while let Some(start) = rest.find(|c: char| c.is_ascii_digit() || c == '.') {
        let preceded_by_word = rest[..start].chars().next_back().is_some_and(is_word);
        let token_len = rest[start..]
            .find(|c: char| !is_word(c))
            .unwrap_or(rest.len() - start);
        let token = &rest[start..start + token_len];
        rest = &rest[start + token_len..];
        if preceded_by_word {
            continue; // part of an identifier such as `log2`
        }
        let digits = token.chars().filter(char::is_ascii_digit).count();
        if token.chars().all(|c| c.is_ascii_digit()) {
            if token.parse::<i64>().is_err() {
                return Err(format!(
                    "the integer literal {token} is too large for the engine"
                ));
            }
        } else if digits > DECIMAL_LITERAL_MAX_DIGITS {
            return Err(format!(
                "the literal {token} has more than {DECIMAL_LITERAL_MAX_DIGITS} significant \
                 digits, which the engine cannot represent exactly"
            ));
        }
    }
    Ok(())
}

fn eval(node: &Node) -> Result<Val, String> {
    let children = node.children();
    match node.operator() {
        Operator::RootNode => match children {
            [only] => eval(only),
            _ => Err("empty expression".to_owned()),
        },
        Operator::Const { value } => constant(value),
        Operator::Neg => Ok(Val::Num(match number(unary(children)?)? {
            Num::Exact(n) => Num::Exact(-n),
            Num::Approx(f) => Num::Approx(-f),
        })),
        Operator::Add | Operator::Sub | Operator::Mul => {
            let (a, b) = numbers(children)?;
            arithmetic(node.operator(), &a, &b).map(Val::Num)
        }
        Operator::Div => {
            let (a, b) = numbers(children)?;
            divide(&a, &b).map(Val::Num)
        }
        Operator::Mod => {
            let (a, b) = numbers(children)?;
            remainder(&a, &b).map(Val::Num)
        }
        Operator::Exp => {
            let (a, b) = numbers(children)?;
            power(&a, &b).map(Val::Num)
        }
        Operator::Eq | Operator::Neq => {
            let (a, b) = numbers(children)?;
            let (Num::Exact(a), Num::Exact(b)) = (a, b) else {
                return Err(
                    "== or != over a fractional power, which is computed in floating \
                     point - rewrite the comparison as math::abs(a - b) <= 0.0001 (an \
                     explicit numeric tolerance)"
                        .to_owned(),
                );
            };
            Ok(Val::Bool(
                (a == b) == matches!(node.operator(), Operator::Eq),
            ))
        }
        Operator::Gt | Operator::Lt | Operator::Geq | Operator::Leq => {
            let (a, b) = numbers(children)?;
            let ordering = order(&a, &b)?;
            Ok(Val::Bool(match node.operator() {
                Operator::Gt => ordering == Ordering::Greater,
                Operator::Lt => ordering == Ordering::Less,
                Operator::Geq => ordering != Ordering::Less,
                _ => ordering != Ordering::Greater,
            }))
        }
        Operator::And | Operator::Or => {
            // Both sides are always evaluated: an error anywhere is a
            // rejection, never hidden behind a short circuit.
            let [a, b] = children else {
                return Err("malformed boolean operator".to_owned());
            };
            let (a, b) = (boolean(a)?, boolean(b)?);
            Ok(Val::Bool(if matches!(node.operator(), Operator::And) {
                a && b
            } else {
                a || b
            }))
        }
        Operator::Not => Ok(Val::Bool(!boolean(unary(children)?)?)),
        Operator::FunctionIdentifier { identifier } => {
            let mut arguments = Vec::new();
            collect_arguments(unary(children)?, &mut arguments)?;
            function(identifier, arguments).map(Val::Num)
        }
        other => Err(format!("`{other}` is not in the dialect")),
    }
}

fn unary(children: &[Node]) -> Result<&Node, String> {
    match children {
        [only] => Ok(only),
        _ => Err("malformed operator".to_owned()),
    }
}

fn number(node: &Node) -> Result<Num, String> {
    match eval(node)? {
        Val::Num(n) => Ok(n),
        Val::Bool(_) => Err("a boolean was used where a number is required".to_owned()),
    }
}

fn numbers(children: &[Node]) -> Result<(Num, Num), String> {
    match children {
        [a, b] => Ok((number(a)?, number(b)?)),
        _ => Err("malformed operator".to_owned()),
    }
}

fn boolean(node: &Node) -> Result<bool, String> {
    match eval(node)? {
        Val::Bool(b) => Ok(b),
        Val::Num(_) => Err("a number was used where a boolean is required".to_owned()),
    }
}

/// Function arguments: one expression, or a comma tuple of them. A
/// parenthesised tuple inside the list is not flattened.
fn collect_arguments(node: &Node, out: &mut Vec<Num>) -> Result<(), String> {
    let inner = match (node.operator(), node.children()) {
        (Operator::RootNode, [only]) => only,
        _ => node,
    };
    if matches!(inner.operator(), Operator::Tuple) {
        for child in inner.children() {
            out.push(number(child)?);
        }
    } else {
        out.push(number(inner)?);
    }
    Ok(())
}

fn constant(value: &Value) -> Result<Val, String> {
    match value {
        Value::Int(i) => Ok(Val::Num(Num::Exact(BigRational::from_integer(
            BigInt::from(*i),
        )))),
        Value::Float(f) => decimal(*f).map(|n| Val::Num(Num::Exact(n))),
        Value::Boolean(b) => Ok(Val::Bool(*b)),
        other => Err(format!("the value {other} is not in the dialect")),
    }
}

/// The decimal a float literal was written as. `f64`'s `Display` prints the
/// shortest decimal that round-trips, with no exponent, which is the literal
/// itself for anything `check_literals` admits.
fn decimal(f: f64) -> Result<BigRational, String> {
    if !f.is_finite() {
        return Err("a literal is out of range".to_owned());
    }
    let text = f.to_string();
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let digits = format!("{whole}{fraction}");
    let numerator = digits
        .parse::<BigInt>()
        .map_err(|_| format!("the literal {text} could not be read"))?;
    let scale = u32::try_from(fraction.len()).map_err(|_| "a literal is out of range")?;
    Ok(BigRational::new(numerator, BigInt::from(10).pow(scale)))
}

fn approx(n: &Num) -> Result<f64, String> {
    let f = match n {
        Num::Exact(r) => r.to_f64().unwrap_or(f64::NAN),
        Num::Approx(f) => *f,
    };
    finite(f)
}

fn finite(f: f64) -> Result<f64, String> {
    if f.is_finite() {
        Ok(f)
    } else {
        Err("a floating-point result is out of range or undefined".to_owned())
    }
}

fn arithmetic(operator: &Operator, a: &Num, b: &Num) -> Result<Num, String> {
    if let (Num::Exact(x), Num::Exact(y)) = (a, b) {
        return Ok(Num::Exact(match operator {
            Operator::Add => x + y,
            Operator::Sub => x - y,
            _ => x * y,
        }));
    }
    let (x, y) = (approx(a)?, approx(b)?);
    finite(match operator {
        Operator::Add => x + y,
        Operator::Sub => x - y,
        _ => x * y,
    })
    .map(Num::Approx)
}

fn divide(a: &Num, b: &Num) -> Result<Num, String> {
    if let (Num::Exact(x), Num::Exact(y)) = (a, b) {
        if y.is_zero() {
            return Err("division by zero".to_owned());
        }
        return Ok(Num::Exact(x / y));
    }
    let (x, y) = (approx(a)?, approx(b)?);
    if y.abs() < f64::MIN_POSITIVE {
        return Err("division by zero".to_owned());
    }
    finite(x / y).map(Num::Approx)
}

/// Truncated remainder: the sign follows the dividend, as in Rust.
fn remainder(a: &Num, b: &Num) -> Result<Num, String> {
    if let (Num::Exact(x), Num::Exact(y)) = (a, b) {
        if y.is_zero() {
            return Err("remainder by zero".to_owned());
        }
        return Ok(Num::Exact(x - y * (x / y).trunc()));
    }
    let (x, y) = (approx(a)?, approx(b)?);
    if y.abs() < f64::MIN_POSITIVE {
        return Err("remainder by zero".to_owned());
    }
    finite(x % y).map(Num::Approx)
}

fn power(base: &Num, exponent: &Num) -> Result<Num, String> {
    if let (Num::Exact(b), Num::Exact(e)) = (base, exponent) {
        if e.is_integer() {
            let Some(e) = e
                .to_integer()
                .to_i32()
                .filter(|e| e.abs() <= EXACT_EXPONENT_MAX)
            else {
                return Err(format!(
                    "the exponent {e} is beyond the engine's exact range (±{EXACT_EXPONENT_MAX})"
                ));
            };
            if b.is_zero() && e < 0 {
                return Err("division by zero (zero raised to a negative power)".to_owned());
            }
            let bits = b.numer().bits().max(b.denom().bits());
            if bits.saturating_mul(u64::from(e.unsigned_abs())) > EXACT_POWER_MAX_BITS {
                return Err("a power is too large for exact arithmetic".to_owned());
            }
            return Ok(Num::Exact(Pow::pow(b, e)));
        }
    }
    let (b, e) = (approx(base)?, approx(exponent)?);
    finite(b.powf(e)).map(Num::Approx)
}

fn order(a: &Num, b: &Num) -> Result<Ordering, String> {
    if let (Num::Exact(x), Num::Exact(y)) = (a, b) {
        return Ok(x.cmp(y));
    }
    let (x, y) = (approx(a)?, approx(b)?);
    if near(x, y) {
        return Err(format!(
            "the comparison of {x} with {y} involves a fractional power, computed in \
             floating point, and the two sides are too close to decide - state an \
             explicit numeric tolerance, e.g. math::abs(a - b) <= 0.0001"
        ));
    }
    x.partial_cmp(&y)
        .ok_or_else(|| "a floating-point result is undefined".to_owned())
}

fn near(x: f64, y: f64) -> bool {
    (x - y).abs() <= NEAR_TIE * x.abs().max(y.abs()).max(1.0)
}

fn function(identifier: &str, arguments: Vec<Num>) -> Result<Num, String> {
    match (identifier, arguments.as_slice()) {
        ("math::abs", [Num::Exact(x)]) => Ok(Num::Exact(x.abs())),
        ("math::abs", [Num::Approx(x)]) => Ok(Num::Approx(x.abs())),
        ("floor" | "ceil" | "round", [Num::Exact(x)]) => Ok(Num::Exact(match identifier {
            "floor" => x.floor(),
            "ceil" => x.ceil(),
            _ => x.round(),
        })),
        ("floor" | "ceil" | "round", [Num::Approx(x)]) => {
            // The step is at an integer for floor/ceil, at a half for round.
            let step = if identifier == "round" { x - 0.5 } else { *x };
            if near(step, step.round()) {
                return Err(format!(
                    "{identifier} of a fractional power ({x}) falls too close to a rounding \
                     boundary to decide in floating point"
                ));
            }
            // Away from the boundary the rounded value is a known integer.
            let rounded = match identifier {
                "floor" => x.floor(),
                "ceil" => x.ceil(),
                _ => x.round(),
            };
            BigRational::from_float(rounded)
                .map(Num::Exact)
                .ok_or_else(|| "a floating-point result is out of range".to_owned())
        }
        ("min" | "max", [_, ..]) => extreme(identifier == "max", arguments),
        ("math::abs" | "floor" | "ceil" | "round" | "min" | "max", _) => Err(format!(
            "`{identifier}` was given {} arguments",
            arguments.len()
        )),
        _ => Err(format!("the function `{identifier}` is not in the dialect")),
    }
}

fn extreme(want_max: bool, arguments: Vec<Num>) -> Result<Num, String> {
    let mut best: Option<Num> = None;
    for candidate in arguments {
        best = Some(match best {
            None => candidate,
            Some(current) => {
                let ordering = match (&candidate, &current) {
                    (Num::Exact(x), Num::Exact(y)) => x.cmp(y),
                    // Near-equal approximations are interchangeable here:
                    // either one is the extreme to within the tolerance.
                    _ => approx(&candidate)?
                        .partial_cmp(&approx(&current)?)
                        .unwrap_or(Ordering::Equal),
                };
                if (ordering == Ordering::Greater) == want_max && ordering != Ordering::Equal {
                    candidate
                } else {
                    current
                }
            }
        });
    }
    best.ok_or_else(|| "min/max need at least one argument".to_owned())
}
