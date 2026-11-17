//! Exact evaluation of the arithmetic dialect.
//!
//! evalexpr parses; this module evaluates its tree. Integers and decimal
//! literals are arbitrary-precision fractions, so `+ - * / %`, integer
//! powers, and every comparison over them are exact: `90/144` is `5/8`,
//! `0.15 * 240 == 36` holds, and integers above 2^53 keep every digit. Numeric
//! literals are read from the expression text, never from a rounded `f64`.
//!
//! Only a root has no exact result: a fractional exponent, or `math::sqrt` of
//! anything but a perfect-square fraction (that one is exact). It becomes a
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
/// both memory and the cost of each operation (about 2,400 decimal digits).
const VALUE_MAX_BITS: u64 = 1 << 13;
/// Largest power-of-ten exponent a literal may carry. Larger than any value
/// the size limit admits, so the limit that matters is `VALUE_MAX_BITS`.
const LITERAL_EXPONENT_MAX: u32 = 2_500;
/// A fractional power — and an inexact square root, which is the half power —
/// is bracketed by its `f64` value ± one part in this.
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
pub(super) const FUNCTIONS: [&str; 7] = [
    "math::abs",
    "math::sqrt",
    "min",
    "max",
    "floor",
    "ceil",
    "round",
];

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
    Boolean {
        /// What the expression evaluated to.
        holds: bool,
        /// True for a false expression that is, as a whole, a bare
        /// `math::sqrt(a)` compared by `==` with an exact value.
        exact_root_equality: bool,
    },
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
    let (rewritten, literals) = lift_literals(expression)?;
    let tree = evalexpr::build_operator_tree::<DefaultNumericTypes>(&rewritten)
        .map_err(|e| e.to_string())?;
    Ok(match eval(&tree, 0, &literals)? {
        Val::Bool(holds) => Outcome::Boolean {
            holds,
            exact_root_equality: !holds && exact_root_equality(&tree, &literals)?,
        },
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

/// Lift every numeric literal out of `expression`, exactly.
///
/// evalexpr would parse a decimal to a rounded `f64` (and an integer beyond
/// `i64` likewise) before its tree exists. So each literal is parsed here,
/// from its text, into a fraction, and replaced in the text by its index in
/// the returned table; every integer constant in the tree is then such an
/// index. Non-decimal forms (hex and the like) are rejected.
fn lift_literals(expression: &str) -> Result<(String, Vec<BigRational>), String> {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.';
    let mut rewritten = String::with_capacity(expression.len());
    let mut literals = Vec::new();
    let mut previous_is_word = false;
    let mut previous_literal: Option<(usize, usize)> = None;
    let mut chars = expression.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if previous_is_word || !(c.is_ascii_digit() || c == '.') {
            previous_is_word = is_word(c);
            rewritten.push(c);
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
        // `max(12,500, 9)` would read as three arguments. A bare 3-digit
        // literal straight after `<1-3 digits>,` is taken for a digit group.
        let token = &expression[start..end];
        let is_group = |t: &str, lengths: std::ops::RangeInclusive<usize>| {
            lengths.contains(&t.len()) && t.chars().all(|d| d.is_ascii_digit())
        };
        if let Some((previous_start, previous_end)) = previous_literal {
            if previous_end + 1 == start
                && expression[previous_end..].starts_with(',')
                && is_group(token, 3..=3)
                && is_group(&expression[previous_start..previous_end], 1..=3)
            {
                return Err(format!(
                    "`{}` looks like a number with digit grouping - write it without the \
                     comma, or put a space after the comma if these are two arguments",
                    &expression[previous_start..end]
                ));
            }
        }
        previous_literal = Some((start, end));
        rewritten.push_str(&literals.len().to_string());
        literals.push(literal(token)?);
        previous_is_word = true;
    }
    Ok((rewritten, literals))
}

/// The exact value of one decimal literal: digits, an optional point, an
/// optional power-of-ten exponent.
fn literal(token: &str) -> Result<BigRational, String> {
    let malformed = || format!("the literal {token} is not a decimal number");
    let (mantissa, exponent) = match token.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (
            mantissa,
            exponent
                .parse::<i32>()
                .ok()
                .filter(|e| e.unsigned_abs() <= LITERAL_EXPONENT_MAX)
                .ok_or_else(|| {
                    format!(
                        "the literal {token} has a malformed exponent or one beyond \
                         ±{LITERAL_EXPONENT_MAX}"
                    )
                })?,
        ),
        None => (token, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all_digits = |part: &str| part.chars().all(|d| d.is_ascii_digit());
    if !all_digits(whole) || !all_digits(fraction) || whole.len() + fraction.len() == 0 {
        return Err(malformed());
    }
    // `1,000` arrives as `1` and `000`: a digit group, not a number.
    if whole.len() > 1 && whole.starts_with('0') {
        return Err(format!(
            "the literal {token} has a leading zero - write numbers without digit grouping \
             or padding"
        ));
    }
    let digits = format!("{whole}{fraction}")
        .parse::<BigInt>()
        .map_err(|_| malformed())?;
    let fraction_len = i32::try_from(fraction.len()).map_err(|_| malformed())?;
    let scale = exponent - fraction_len;
    let power = BigInt::from(10).pow(scale.unsigned_abs());
    Ok(if scale >= 0 {
        BigRational::from_integer(digits * power)
    } else {
        BigRational::new(digits, power)
    })
}

fn eval(node: &Node, depth: usize, literals: &[BigRational]) -> Result<Val, String> {
    if depth > DEPTH_MAX {
        return Err("the expression is nested too deeply".to_owned());
    }
    let value = eval_node(node, depth + 1, literals)?;
    match &value {
        Val::Num(n) if oversized(n) => Err(too_large()),
        _ => Ok(value),
    }
}

fn oversized(n: &Num) -> bool {
    let (low, high) = bounds(n);
    let bits = |r: &BigRational| r.numer().bits().max(r.denom().bits());
    bits(low).max(bits(high)) > VALUE_MAX_BITS
}

fn too_large() -> String {
    format!("a value exceeds the engine's size limit of {VALUE_MAX_BITS} bits (about 2,400 digits)")
}

fn eval_node(node: &Node, depth: usize, literals: &[BigRational]) -> Result<Val, String> {
    let children = node.children();
    match node.operator() {
        Operator::RootNode => match children {
            [only] => eval(only, depth, literals),
            _ => Err("empty expression".to_owned()),
        },
        Operator::Const { value } => constant(value, literals),
        Operator::Neg => Ok(Val::Num(match number(unary(children)?, depth, literals)? {
            Num::Exact(n) => Num::Exact(-n),
            Num::Range(low, high) => Num::Range(-high, -low),
        })),
        Operator::Add | Operator::Sub | Operator::Mul => {
            let (a, b) = numbers(children, depth, literals)?;
            Ok(Val::Num(arithmetic(node.operator(), &a, &b)))
        }
        Operator::Div => {
            let (a, b) = numbers(children, depth, literals)?;
            divide(&a, &b).map(Val::Num)
        }
        Operator::Mod => {
            let (a, b) = numbers(children, depth, literals)?;
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
            let (a, b) = numbers(children, depth, literals)?;
            power(&a, &b).map(Val::Num)
        }
        Operator::Eq | Operator::Neq => {
            let [a, b] = children else {
                return Err("malformed operator".to_owned());
            };
            let equal = equals(a, b, depth, literals)?;
            Ok(Val::Bool(equal == matches!(node.operator(), Operator::Eq)))
        }
        Operator::Gt | Operator::Lt | Operator::Geq | Operator::Leq => {
            let (a, b) = numbers(children, depth, literals)?;
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
            let (a, b) = (boolean(a, depth, literals)?, boolean(b, depth, literals)?);
            Ok(Val::Bool(if matches!(node.operator(), Operator::And) {
                a && b
            } else {
                a || b
            }))
        }
        Operator::Not => Ok(Val::Bool(!boolean(unary(children)?, depth, literals)?)),
        Operator::FunctionIdentifier { identifier } => {
            if !FUNCTIONS.contains(&identifier.as_str()) {
                return Err(format!("the function `{identifier}` is not in the dialect"));
            }
            // `floor 7/2` parses as `floor(7)/2`: require the parentheses.
            let argument = unary(children)?;
            if !matches!(argument.operator(), Operator::RootNode) {
                return Err(format!(
                    "`{identifier}` must be called with parentheses: {identifier}(...)"
                ));
            }
            let arguments = collect_arguments(argument, depth, literals)?;
            function(identifier, &arguments).map(Val::Num)
        }
        other => Err(format!("`{other}` is not in the dialect")),
    }
}

/// Whether the expression is, as a whole, `==` with a bare `math::sqrt(a)`
/// of exact `a` on exactly one side.
///
/// Called only for an expression that evaluated to false, so the other side
/// is already known to be an exact number: `equals` rejects anything else.
/// The caller is then told an exact equality was checked — and nothing about
/// how close the two sides are, because the translator may have rearranged
/// the claim to isolate the root, and a statement about the compared value
/// could sit beside a claim it does not describe.
fn exact_root_equality(tree: &Node, literals: &[BigRational]) -> Result<bool, String> {
    let mut top = tree;
    while let (Operator::RootNode, [only]) = (top.operator(), top.children()) {
        top = only;
    }
    let (Operator::Eq, [a, b]) = (top.operator(), top.children()) else {
        return Ok(false);
    };
    Ok(radicand(a, 0, literals)?.is_some() != radicand(b, 0, literals)?.is_some())
}

/// Whether the two sides of `==` or `!=` are equal, when that is decidable.
fn equals(a: &Node, b: &Node, depth: usize, literals: &[BigRational]) -> Result<bool, String> {
    if let Some(equal) = bare_root_equality(a, b, depth, literals)? {
        return Ok(equal);
    }
    match (eval(a, depth, literals)?, eval(b, depth, literals)?) {
        (Val::Bool(a), Val::Bool(b)) => Ok(a == b),
        (Val::Num(Num::Exact(a)), Val::Num(Num::Exact(b))) => Ok(a == b),
        // A bare square root the squaring rule could not settle: the prompt
        // forbids a tolerance for this shape, so the refusal must not advise
        // one.
        (Val::Num(_), Val::Num(_)) if is_bare_sqrt(a) || is_bare_sqrt(b) => Err(
            "an equality with a square root is decided exactly only when the square root \
             stands alone and the other side is exact or a single square root - rearrange \
             the claim into that shape (square both sides if needed), or decline it; do \
             not use a tolerance"
                .to_owned(),
        ),
        (Val::Num(_), Val::Num(_)) => Err(
            "== or != over a fractional power, which has no exact value - rewrite the \
             comparison as math::abs(a - b) <= 0.0001 (an explicit numeric tolerance)"
                .to_owned(),
        ),
        _ => Err("== or != between a number and a boolean".to_owned()),
    }
}

/// The argument of a bare `math::sqrt(...)` call, through any parentheses
/// around the call. `None` for any other node.
fn bare_sqrt_argument(node: &Node) -> Option<&Node> {
    let mut node = node;
    while let (Operator::RootNode, [only]) = (node.operator(), node.children()) {
        node = only;
    }
    match (node.operator(), node.children()) {
        (Operator::FunctionIdentifier { identifier }, [argument])
            if identifier == "math::sqrt" && matches!(argument.operator(), Operator::RootNode) =>
        {
            Some(argument)
        }
        _ => None,
    }
}

fn is_bare_sqrt(node: &Node) -> bool {
    bare_sqrt_argument(node).is_some()
}

/// What stands under a bare `math::sqrt(...)`, when it is exact and not
/// negative. `None` for any other node, so the ordinary rules decide.
fn radicand(
    node: &Node,
    depth: usize,
    literals: &[BigRational],
) -> Result<Option<BigRational>, String> {
    let Some(argument) = bare_sqrt_argument(node) else {
        return Ok(None);
    };
    Ok(
        match collect_arguments(argument, depth, literals)?.as_slice() {
            [Num::Exact(x)] if !x.is_negative() => Some(x.clone()),
            _ => None,
        },
    )
}

/// `math::sqrt(a) == b`, decided exactly.
///
/// For exact `a >= 0` and exact `b`, the root equals `b` exactly when
/// `b >= 0` and `b * b == a`: no float and no bracket, so an equality claim
/// about a square root never needs a tolerance. `None` when neither side is a
/// bare square root, or a value involved is inexact or not a number.
fn bare_root_equality(
    a: &Node,
    b: &Node,
    depth: usize,
    literals: &[BigRational],
) -> Result<Option<bool>, String> {
    let squares_to = |other: &Node, under: &BigRational| -> Result<Option<bool>, String> {
        Ok(match eval(other, depth, literals)? {
            Val::Num(Num::Exact(q)) => Some(!q.is_negative() && &q * &q == *under),
            _ => None,
        })
    };
    match (radicand(a, depth, literals)?, radicand(b, depth, literals)?) {
        (Some(x), Some(y)) => Ok(Some(x == y)),
        (Some(x), None) => squares_to(b, &x),
        (None, Some(y)) => squares_to(a, &y),
        (None, None) => Ok(None),
    }
}

fn unary(children: &[Node]) -> Result<&Node, String> {
    match children {
        [only] => Ok(only),
        _ => Err("malformed operator".to_owned()),
    }
}

fn number(node: &Node, depth: usize, literals: &[BigRational]) -> Result<Num, String> {
    match eval(node, depth, literals)? {
        Val::Num(n) => Ok(n),
        Val::Bool(_) => Err("a boolean was used where a number is required".to_owned()),
    }
}

fn numbers(
    children: &[Node],
    depth: usize,
    literals: &[BigRational],
) -> Result<(Num, Num), String> {
    match children {
        [a, b] => Ok((number(a, depth, literals)?, number(b, depth, literals)?)),
        _ => Err("malformed operator".to_owned()),
    }
}

fn boolean(node: &Node, depth: usize, literals: &[BigRational]) -> Result<bool, String> {
    match eval(node, depth, literals)? {
        Val::Bool(b) => Ok(b),
        Val::Num(_) => Err("a number was used where a boolean is required".to_owned()),
    }
}

/// Function arguments: one expression, or a comma tuple of them. A
/// parenthesised tuple inside the list is not flattened.
fn collect_arguments(
    node: &Node,
    depth: usize,
    literals: &[BigRational],
) -> Result<Vec<Num>, String> {
    let inner = match (node.operator(), node.children()) {
        (Operator::RootNode, [only]) => only,
        _ => node,
    };
    if matches!(inner.operator(), Operator::Tuple) {
        inner
            .children()
            .iter()
            .map(|child| number(child, depth, literals))
            .collect()
    } else {
        Ok(vec![number(inner, depth, literals)?])
    }
}

fn constant(value: &Value, literals: &[BigRational]) -> Result<Val, String> {
    match value {
        // Every integer the parser saw is an index written by `lift_literals`.
        Value::Int(index) => usize::try_from(*index)
            .ok()
            .and_then(|index| literals.get(index))
            .map(|n| Val::Num(Num::Exact(n.clone())))
            .ok_or_else(|| "a literal could not be read".to_owned()),
        Value::Boolean(b) => Ok(Val::Bool(*b)),
        other => Err(format!("the value {other} is not in the dialect")),
    }
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
            format!(
                "the exponent {} is beyond the engine's range (±{limit})",
                printed(exponent)
            )
        })?;
    let positive = match base {
        Num::Exact(b) => {
            let bits = b.numer().bits().max(b.denom().bits());
            if bits.saturating_mul(u64::from(magnitude)) > VALUE_MAX_BITS {
                return Err(too_large());
            }
            Num::Exact(Pow::pow(b, magnitude))
        }
        Num::Range(..) => {
            let mut product = Num::Exact(BigRational::one());
            for _ in 0..magnitude {
                product = multiply(&product, base);
                if oversized(&product) {
                    return Err(too_large());
                }
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
    let out_of_range = || {
        "a fractional power whose base or result is outside the range of a 64-bit float \
         is not supported"
            .to_owned()
    };
    let as_float = |r: &BigRational| r.to_f64().filter(|f| f.is_normal());
    let (b, e) = (
        as_float(base).ok_or_else(out_of_range)?,
        as_float(exponent).ok_or_else(out_of_range)?,
    );
    if e.abs() > f64::from(RANGE_EXPONENT_MAX) {
        return Err(format!(
            "a fractional exponent beyond ±{RANGE_EXPONENT_MAX} is not supported"
        ));
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

/// The square root: exact when the argument is a perfect-square fraction,
/// otherwise the bracketed half power.
fn square_root(x: &Num) -> Result<Num, String> {
    if let Num::Exact(value) = x {
        if value.is_negative() {
            return Err("the square root of a negative number is undefined".to_owned());
        }
        // A fraction in lowest terms is a perfect square exactly when its
        // numerator and denominator both are. Integer roots: no float.
        let (numer, denom) = (value.numer().sqrt(), value.denom().sqrt());
        if &numer * &numer == *value.numer() && &denom * &denom == *value.denom() {
            return Ok(Num::Exact(BigRational::new(numer, denom)));
        }
    }
    if matches!(x, Num::Range(..)) {
        return Err(
            "the square root of an inexact value is not supported - take the square root \
             of an exact number"
                .to_owned(),
        );
    }
    root(x, &BigRational::new(BigInt::one(), BigInt::from(2)))
}

fn function(identifier: &str, arguments: &[Num]) -> Result<Num, String> {
    match (identifier, arguments) {
        ("math::sqrt", [x]) => square_root(x),
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
