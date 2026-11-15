//! Exact evaluation of the arithmetic dialect.
//!
//! evalexpr parses; this module evaluates its tree. Integers and decimal
//! literals are arbitrary-precision fractions, so `+ - * / %`, integer
//! powers, and every comparison over them are exact: `90/144` is `5/8`,
//! `0.15 * 240 == 36` holds, and integers above 2^53 keep every digit.
//!
//! Only a fractional exponent (a root) has no exact result. It becomes a
//! [`Num::Range`]: two exact fractions that bracket the true value. Ranges
//! flow through later arithmetic as exact interval arithmetic, so the
//! bracket stays true however the value is scaled or cancelled. A comparison
//! is answered only when the ranges do not overlap; `==`/`!=` over a range is
//! rejected. Anything outside the dialect is rejected.

use evalexpr::{DefaultNumericTypes, Node, Operator, Value};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{One, Pow, Signed, ToPrimitive, Zero};
use std::cmp::Ordering;

/// Largest integer exponent applied to an exact base.
const EXACT_EXPONENT_MAX: u32 = 4_096;
/// Largest integer exponent applied to a range, and largest magnitude of a
/// fractional exponent.
const RANGE_EXPONENT_MAX: u32 = 64;
/// Largest numerator or denominator, in bits, any value may reach. Bounds
/// both memory and the cost of each operation.
const VALUE_MAX_BITS: u64 = 1 << 16;
/// Most significant digits a decimal literal may carry: up to this many, an
/// `f64` round-trips the decimal exactly, so it can be recovered from the
/// `f64` evalexpr parsed it to.
const DECIMAL_LITERAL_MAX_DIGITS: usize = 15;
/// A fractional power is bracketed by its `f64` value ± one part in this.
/// The `f64` result is within about 1e-11 of the truth for the admitted
/// operands (correctly rounded inputs, |exponent| <= 64, a libm `pow` good to
/// a few ulp), so the bracket has two orders of magnitude to spare.
const ROOT_BRACKET_PARTS: u32 = 1_000_000_000;
/// Deepest expression tree walked.
const DEPTH_MAX: usize = 500;
/// Stack for the walk: `DEPTH_MAX` frames fit with room to spare in a debug
/// build, whatever stack the caller happens to be running on.
const STACK_BYTES: usize = 32 << 20;
/// Longest printed number in a rejection message.
const PRINTED_MAX_CHARS: usize = 60;

/// The dialect's functions.
const FUNCTIONS: [&str; 6] = ["math::abs", "min", "max", "floor", "ceil", "round"];

/// A number: exact, or bracketed by two exact bounds (`low < high`).
#[derive(Debug, Clone)]
enum Num {
    Exact(BigRational),
    Range(BigRational, BigRational),
}

#[derive(Debug, Clone)]
enum Val {
    Num(Num),
    Bool(bool),
}

/// What a well-formed expression evaluated to.
pub(super) enum Outcome {
    Boolean(bool),
    /// A number, printed — the caller reports it as a non-boolean result.
    NonBoolean(String),
}

/// Evaluate `expression`; the error is the reason the engine rejected it.
pub(super) fn evaluate(expression: &str) -> Result<Outcome, String> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(STACK_BYTES)
            .spawn_scoped(scope, || evaluate_here(expression))
            .map_err(|e| format!("the engine could not start: {e}"))?
            .join()
            .map_err(|_| "the engine failed on this expression".to_owned())?
    })
}

fn evaluate_here(expression: &str) -> Result<Outcome, String> {
    if expression.contains("//") || expression.contains("/*") {
        return Err(
            "`//` and `/*` start a comment, which is not in the dialect - write \
             floor(a/b) for integer division"
                .to_owned(),
        );
    }
    check_literals(expression)?;
    let tree = evalexpr::build_operator_tree::<DefaultNumericTypes>(expression)
        .map_err(|e| e.to_string())?;
    Ok(match eval(&tree, 0)? {
        Val::Bool(holds) => Outcome::Boolean(holds),
        Val::Num(Num::Exact(n)) => Outcome::NonBoolean(printed(&n)),
        Val::Num(Num::Range(low, _)) => Outcome::NonBoolean(format!("about {}", printed(&low))),
    })
}

fn printed(n: &BigRational) -> String {
    let text = n.to_string();
    if text.len() <= PRINTED_MAX_CHARS {
        return text;
    }
    let head: String = text.chars().take(PRINTED_MAX_CHARS).collect();
    format!("{head}... ({} characters)", text.len())
}

/// Reject numeric literals evalexpr would not hand over exactly: an integer
/// beyond `i64` (it parses as a rounded float), a decimal with more
/// significant digits than an `f64` round-trips, one outside `f64`'s normal
/// range, or a non-decimal form such as hex.
fn check_literals(expression: &str) -> Result<(), String> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.';
    let mut previous_is_word = false;
    let mut chars = expression.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if previous_is_word || !(c.is_ascii_digit() || c == '.') {
            previous_is_word = is_word(c);
            continue;
        }
        // A token starting with a digit or a dot: take it whole, including
        // the sign of a decimal exponent (`1e-7`).
        let mut end = start + c.len_utf8();
        while let Some(&(at, next)) = chars.peek() {
            let token = &expression[start..end];
            let exponent_sign = matches!(next, '+' | '-')
                && token.ends_with(['e', 'E'])
                && token[..token.len() - 1]
                    .chars()
                    .all(|d| d.is_ascii_digit() || d == '.');
            if !(is_word(next) || exponent_sign) {
                break;
            }
            end = at + next.len_utf8();
            chars.next();
        }
        check_literal(&expression[start..end])?;
        previous_is_word = true;
    }
    Ok(())
}

fn check_literal(token: &str) -> Result<(), String> {
    if token.chars().all(|c| c.is_ascii_digit()) {
        return match token.parse::<i64>() {
            Ok(_) => Ok(()),
            Err(_) => Err(format!(
                "the integer literal {token} is too large for the engine"
            )),
        };
    }
    let Ok(value) = token.parse::<f64>() else {
        return Err(format!("the literal {token} is not a decimal number"));
    };
    let mantissa = token.split(['e', 'E']).next().unwrap_or(token);
    let significant = mantissa
        .chars()
        .filter(char::is_ascii_digit)
        .skip_while(|d| *d == '0')
        .count();
    if significant > DECIMAL_LITERAL_MAX_DIGITS {
        return Err(format!(
            "the literal {token} has more than {DECIMAL_LITERAL_MAX_DIGITS} significant \
             digits, which the engine cannot represent exactly"
        ));
    }
    if significant > 0 && !value.is_normal() {
        return Err(format!(
            "the literal {token} is outside the range the engine can represent exactly"
        ));
    }
    Ok(())
}

fn eval(node: &Node, depth: usize) -> Result<Val, String> {
    if depth > DEPTH_MAX {
        return Err("the expression is nested too deeply".to_owned());
    }
    let value = eval_node(node, depth + 1)?;
    if let Val::Num(n) = &value {
        let (low, high) = bounds(n);
        let bits = |r: &BigRational| r.numer().bits().max(r.denom().bits());
        if bits(low).max(bits(high)) > VALUE_MAX_BITS {
            return Err("a value is too large for exact arithmetic".to_owned());
        }
    }
    Ok(value)
}

fn eval_node(node: &Node, depth: usize) -> Result<Val, String> {
    let children = node.children();
    match node.operator() {
        Operator::RootNode => match children {
            [only] => eval(only, depth),
            _ => Err("empty expression".to_owned()),
        },
        Operator::Const { value } => constant(value),
        Operator::Neg => Ok(Val::Num(match number(unary(children)?, depth)? {
            Num::Exact(n) => Num::Exact(-n),
            Num::Range(low, high) => Num::Range(-high, -low),
        })),
        Operator::Add | Operator::Sub | Operator::Mul => {
            let (a, b) = numbers(children, depth)?;
            Ok(Val::Num(arithmetic(node.operator(), &a, &b)))
        }
        Operator::Div => {
            let (a, b) = numbers(children, depth)?;
            divide(&a, &b).map(Val::Num)
        }
        Operator::Mod => {
            let (a, b) = numbers(children, depth)?;
            remainder(&a, &b).map(Val::Num)
        }
        Operator::Exp => {
            // evalexpr reads `a^b^c` as `(a^b)^c`; mathematics reads it as
            // `a^(b^c)`. Refuse to pick.
            if children
                .first()
                .is_some_and(|base| matches!(base.operator(), Operator::Exp))
            {
                return Err(
                    "a chained power a^b^c is ambiguous - parenthesise it as (a^b)^c or a^(b^c)"
                        .to_owned(),
                );
            }
            let (a, b) = numbers(children, depth)?;
            power(&a, &b).map(Val::Num)
        }
        Operator::Eq | Operator::Neq => {
            let (a, b) = numbers(children, depth)?;
            let (Num::Exact(a), Num::Exact(b)) = (a, b) else {
                return Err(
                    "== or != over a fractional power, which has no exact value - rewrite \
                     the comparison as math::abs(a - b) <= 0.0001 (an explicit numeric \
                     tolerance)"
                        .to_owned(),
                );
            };
            Ok(Val::Bool(
                (a == b) == matches!(node.operator(), Operator::Eq),
            ))
        }
        Operator::Gt | Operator::Lt | Operator::Geq | Operator::Leq => {
            let (a, b) = numbers(children, depth)?;
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
            let (a, b) = (boolean(a, depth)?, boolean(b, depth)?);
            Ok(Val::Bool(if matches!(node.operator(), Operator::And) {
                a && b
            } else {
                a || b
            }))
        }
        Operator::Not => Ok(Val::Bool(!boolean(unary(children)?, depth)?)),
        Operator::FunctionIdentifier { identifier } => {
            if !FUNCTIONS.contains(&identifier.as_str()) {
                return Err(format!("the function `{identifier}` is not in the dialect"));
            }
            let arguments = collect_arguments(unary(children)?, depth)?;
            function(identifier, &arguments).map(Val::Num)
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

fn number(node: &Node, depth: usize) -> Result<Num, String> {
    match eval(node, depth)? {
        Val::Num(n) => Ok(n),
        Val::Bool(_) => Err("a boolean was used where a number is required".to_owned()),
    }
}

fn numbers(children: &[Node], depth: usize) -> Result<(Num, Num), String> {
    match children {
        [a, b] => Ok((number(a, depth)?, number(b, depth)?)),
        _ => Err("malformed operator".to_owned()),
    }
}

fn boolean(node: &Node, depth: usize) -> Result<bool, String> {
    match eval(node, depth)? {
        Val::Bool(b) => Ok(b),
        Val::Num(_) => Err("a number was used where a boolean is required".to_owned()),
    }
}

/// Function arguments: one expression, or a comma tuple of them. A
/// parenthesised tuple inside the list is not flattened.
fn collect_arguments(node: &Node, depth: usize) -> Result<Vec<Num>, String> {
    let inner = match (node.operator(), node.children()) {
        (Operator::RootNode, [only]) => only,
        _ => node,
    };
    if matches!(inner.operator(), Operator::Tuple) {
        inner
            .children()
            .iter()
            .map(|child| number(child, depth))
            .collect()
    } else {
        Ok(vec![number(inner, depth)?])
    }
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
/// shortest decimal that round-trips, with no exponent; for a literal of at
/// most 15 significant digits in the normal range — all `check_literals`
/// admits — that is the literal itself.
fn decimal(f: f64) -> Result<BigRational, String> {
    if !f.is_finite() {
        return Err("a literal is out of range".to_owned());
    }
    let text = f.to_string();
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let numerator = format!("{whole}{fraction}")
        .parse::<BigInt>()
        .map_err(|_| format!("the literal {text} could not be read"))?;
    let scale = u32::try_from(fraction.len()).map_err(|_| "a literal is out of range")?;
    Ok(BigRational::new(numerator, BigInt::from(10).pow(scale)))
}

fn bounds(n: &Num) -> (&BigRational, &BigRational) {
    match n {
        Num::Exact(x) => (x, x),
        Num::Range(low, high) => (low, high),
    }
}

fn range(low: BigRational, high: BigRational) -> Num {
    if low == high {
        Num::Exact(low)
    } else {
        Num::Range(low, high)
    }
}

/// The range spanned by a set of candidate bounds.
fn span(candidates: &[BigRational; 4]) -> Num {
    let low = candidates.iter().min().cloned().unwrap_or_else(Zero::zero);
    let high = candidates.iter().max().cloned().unwrap_or_else(Zero::zero);
    range(low, high)
}

fn arithmetic(operator: &Operator, a: &Num, b: &Num) -> Num {
    let ((a_low, a_high), (b_low, b_high)) = (bounds(a), bounds(b));
    match operator {
        Operator::Add => range(a_low + b_low, a_high + b_high),
        Operator::Sub => range(a_low - b_high, a_high - b_low),
        _ => multiply(a, b),
    }
}

fn multiply(a: &Num, b: &Num) -> Num {
    let ((a_low, a_high), (b_low, b_high)) = (bounds(a), bounds(b));
    if let (Num::Exact(x), Num::Exact(y)) = (a, b) {
        return Num::Exact(x * y);
    }
    span(&[
        a_low * b_low,
        a_low * b_high,
        a_high * b_low,
        a_high * b_high,
    ])
}

fn divide(a: &Num, b: &Num) -> Result<Num, String> {
    let ((a_low, a_high), (b_low, b_high)) = (bounds(a), bounds(b));
    if b_low.is_zero() && b_high.is_zero() {
        return Err("division by zero".to_owned());
    }
    if !b_low.is_positive() && !b_high.is_negative() {
        return Err(
            "division by a fractional-power result that cannot be told apart from zero \
             (possible division by zero)"
                .to_owned(),
        );
    }
    if let (Num::Exact(x), Num::Exact(y)) = (a, b) {
        return Ok(Num::Exact(x / y));
    }
    Ok(span(&[
        a_low / b_low,
        a_low / b_high,
        a_high / b_low,
        a_high / b_high,
    ]))
}

/// Truncated remainder: the sign follows the dividend, as in C and Rust.
fn remainder(a: &Num, b: &Num) -> Result<Num, String> {
    let (Num::Exact(x), Num::Exact(y)) = (a, b) else {
        return Err("% over a fractional power is not supported".to_owned());
    };
    if y.is_zero() {
        return Err("remainder by zero".to_owned());
    }
    Ok(Num::Exact(x - y * (x / y).trunc()))
}

fn power(base: &Num, exponent: &Num) -> Result<Num, String> {
    let Num::Exact(exponent) = exponent else {
        return Err("a fractional power used as an exponent is not supported".to_owned());
    };
    if !exponent.is_integer() {
        return root(base, exponent);
    }
    let limit = match base {
        Num::Exact(_) => EXACT_EXPONENT_MAX,
        Num::Range(..) => RANGE_EXPONENT_MAX,
    };
    let magnitude = exponent
        .to_integer()
        .abs()
        .to_u32()
        .filter(|m| *m <= limit)
        .ok_or_else(|| {
            format!("the exponent {exponent} is beyond the engine's range (±{limit})")
        })?;
    let positive = match base {
        Num::Exact(b) => {
            let bits = b.numer().bits().max(b.denom().bits());
            if bits.saturating_mul(u64::from(magnitude)) > VALUE_MAX_BITS {
                return Err("a power is too large for exact arithmetic".to_owned());
            }
            Num::Exact(Pow::pow(b, magnitude))
        }
        Num::Range(..) => {
            let mut product = Num::Exact(BigRational::one());
            for _ in 0..magnitude {
                product = multiply(&product, base);
            }
            product
        }
    };
    if exponent.is_negative() {
        divide(&Num::Exact(BigRational::one()), &positive)
    } else {
        Ok(positive)
    }
}

/// A fractional power of an exact base: an `f64` result, bracketed.
fn root(base: &Num, exponent: &BigRational) -> Result<Num, String> {
    let Num::Exact(base) = base else {
        return Err(
            "a fractional power of a fractional power is not supported - combine the \
             exponents"
                .to_owned(),
        );
    };
    if base.is_negative() {
        return Err("a fractional power of a negative number is undefined".to_owned());
    }
    if base.is_zero() {
        return if exponent.is_positive() {
            Ok(Num::Exact(BigRational::zero()))
        } else {
            Err("division by zero (zero raised to a negative power)".to_owned())
        };
    }
    let out_of_range = || "a fractional power is outside the engine's range".to_owned();
    let as_float = |r: &BigRational| r.to_f64().filter(|f| f.is_normal());
    let (b, e) = (
        as_float(base).ok_or_else(out_of_range)?,
        as_float(exponent).ok_or_else(out_of_range)?,
    );
    if e.abs() > f64::from(RANGE_EXPONENT_MAX) {
        return Err(out_of_range());
    }
    let value = Some(b.powf(e))
        .filter(|v| v.is_normal())
        .and_then(BigRational::from_float)
        .ok_or_else(out_of_range)?;
    let margin = &value / BigRational::from_integer(BigInt::from(ROOT_BRACKET_PARTS));
    Ok(Num::Range(&value - &margin, value + margin))
}

fn order(a: &Num, b: &Num) -> Result<Ordering, String> {
    let ((a_low, a_high), (b_low, b_high)) = (bounds(a), bounds(b));
    if let (Num::Exact(x), Num::Exact(y)) = (a, b) {
        return Ok(x.cmp(y));
    }
    if a_high < b_low {
        Ok(Ordering::Less)
    } else if a_low > b_high {
        Ok(Ordering::Greater)
    } else {
        Err(
            "the comparison involves a fractional power and the two sides are too close \
             to decide - state an explicit numeric tolerance, e.g. math::abs(a - b) <= \
             0.0001, or restate the claim root-free"
                .to_owned(),
        )
    }
}

fn function(identifier: &str, arguments: &[Num]) -> Result<Num, String> {
    match (identifier, arguments) {
        ("math::abs", [x]) => Ok(match x {
            Num::Exact(x) => Num::Exact(x.abs()),
            Num::Range(low, high) if !low.is_negative() => Num::Range(low.clone(), high.clone()),
            Num::Range(low, high) if !high.is_positive() => Num::Range(-high, -low),
            Num::Range(low, high) => Num::Range(BigRational::zero(), (-low).max(high.clone())),
        }),
        ("floor" | "ceil" | "round", [x]) => {
            let apply = |r: &BigRational| match identifier {
                "floor" => r.floor(),
                "ceil" => r.ceil(),
                _ => r.round(),
            };
            // All three are monotonic: if both bounds round alike, so does
            // everything between them.
            let (low, high) = bounds(x);
            let (low, high) = (apply(low), apply(high));
            if low == high {
                Ok(Num::Exact(low))
            } else {
                Err(format!(
                    "{identifier} of a fractional power falls too close to a rounding \
                     boundary to decide"
                ))
            }
        }
        ("min" | "max", [_, ..]) => {
            let pick = |side: fn(&Num) -> &BigRational| {
                let sides = arguments.iter().map(side);
                if identifier == "max" {
                    sides.max()
                } else {
                    sides.min()
                }
                .cloned()
                .unwrap_or_else(Zero::zero)
            };
            Ok(range(pick(|n| bounds(n).0), pick(|n| bounds(n).1)))
        }
        _ => Err(format!(
            "`{identifier}` was given {} arguments",
            arguments.len()
        )),
    }
}
