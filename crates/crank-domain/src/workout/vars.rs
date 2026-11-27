//! Workout variables (ADR-0008 §2.6–§2.10): the `Num` field type, declarations and their
//! expressions, exact rational evaluation, the per-position use checks and [`resolve_vars`].
//!
//! Declarations are evaluated as `num_rational::Ratio<i128>` with checked operations (ADR-0008
//! §2.8): every literal is a finite decimal, so `+ − × ÷` stay exact and the whole-number
//! checks are exact comparisons. A value is rounded only where a block uses it, to the
//! printer's precision for that position, and converted to `f64` by reading the rounded decimal
//! with the DSL's own number parser.
use super::dsl::{format_duration, number, parse_duration_word};
use super::{
    MAX_NESTING_DEPTH, PowerTarget, Progression, RepeatNode, StepEnd, StepNode, Workout,
    WorkoutNode,
};
use num_rational::Ratio;
use num_traits::{CheckedAdd, CheckedDiv, CheckedMul, CheckedSub};
use serde::{Deserialize, Serialize};

/// Most declarations a workout may hold (ADR-0008 §2.14).
pub const MAX_VARIABLES: usize = 100;
/// Deepest expression tree a declaration may hold: every operator and every unary minus is one
/// level, and so is every parenthesis while it is read (ADR-0008 §2.14).
pub const MAX_EXPRESSION_DEPTH: usize = 32;
/// Longest variable name, in characters after the `$` (ADR-0008 §2.6).
pub const MAX_VARIABLE_NAME: usize = 32;

/// A literal or a variable reference (ADR-0008 §2.10); serialises as `95` or
/// `{ "var": "thr" }`, so every stored tree, whose fields are bare numbers, reads unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(untagged)]
pub enum Num {
    /// A literal value in the field's unit.
    Lit(f64),
    /// A reference to a declared variable.
    Var {
        /// The variable's name, without the `$`.
        var: String,
    },
}
impl From<f64> for Num {
    fn from(value: f64) -> Self {
        Self::Lit(value)
    }
}
impl Num {
    /// A reference to the variable `name` (without the `$`).
    #[must_use]
    pub fn var(name: impl Into<String>) -> Self {
        Self::Var { var: name.into() }
    }
    /// The literal value; `None` for a reference.
    #[must_use]
    pub const fn lit(&self) -> Option<f64> {
        match self {
            Self::Lit(v) => Some(*v),
            Self::Var { .. } => None,
        }
    }
}

/// A variable's type: the unit of its value (ADR-0008 §2.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// Percent of FTP (`95%`).
    Pct,
    /// Watts (`250w`).
    Watts,
    /// A duration in seconds (`4m`, `1m30s`).
    Seconds,
    /// A plain number (`3`, `0.5`).
    Number,
}

/// A binary operator of a declaration's expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// `+`: two values of one type.
    Add,
    /// `-`: two values of one type.
    Sub,
    /// `*`: a value and a plain number, in either order.
    Mul,
    /// `/`: a value divided by a plain number.
    Div,
}
impl Op {
    const fn symbol(self) -> char {
        match self {
            Self::Add => '+',
            Self::Sub => '-',
            Self::Mul => '*',
            Self::Div => '/',
        }
    }
    const fn precedence(self) -> u8 {
        match self {
            Self::Add | Self::Sub => 1,
            Self::Mul | Self::Div => 2,
        }
    }
}

/// A declaration's expression (ADR-0008 §2.8); arithmetic is allowed in declarations only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expr {
    /// A literal: its exact, non-negative decimal text in `unit` (`"1.1"`; a duration in
    /// seconds, `"90"` for `1m30s`). Text keeps it exact and JSON-safe.
    Lit {
        /// Decimal digits with an optional fraction, as `NUMBER` in the DSL grammar.
        value: String,
        /// The literal's unit.
        unit: Unit,
    },
    /// A variable declared above.
    Var {
        /// The variable's name, without the `$`.
        name: String,
    },
    /// Unary minus.
    Neg {
        /// The negated expression.
        expr: Box<Expr>,
    },
    /// A binary operation.
    Bin {
        /// The operator.
        op: Op,
        /// Left operand.
        l: Box<Expr>,
        /// Right operand.
        r: Box<Expr>,
    },
}

/// One `$name = expr` declaration; its value is derived, never stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct VarDecl {
    /// The variable's name, without the `$`.
    pub name: String,
    /// The expression giving its value.
    pub expr: Expr,
}

/// A parsed workout text: its declarations, in order, and its tree.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DslDoc {
    /// The declarations, in declaration order.
    pub vars: Vec<VarDecl>,
    /// The workout tree.
    pub nodes: Vec<WorkoutNode>,
}

/// A refusal of [`resolve_vars`] for a tree built or deserialised directly; text never
/// reaches it unchecked, since the parser reports the same faults with spans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ResolveError {
    /// A use or a declaration names a variable not declared before it.
    UnknownVariable {
        /// The name, without `$`.
        name: String,
    },
    /// A name is declared twice.
    DuplicateVariable {
        /// The name, without `$`.
        name: String,
    },
    /// A name is not a letter followed by letters, digits and `_`, or is too long.
    InvalidName {
        /// The name as given.
        name: String,
    },
    /// More than [`MAX_VARIABLES`] declarations.
    TooManyVariables {
        /// The bound.
        max: usize,
    },
    /// A declaration's expression is deeper than [`MAX_EXPRESSION_DEPTH`].
    ExpressionTooDeep {
        /// The declaration.
        name: String,
        /// The bound.
        max: usize,
    },
    /// Repeats nest deeper than [`MAX_NESTING_DEPTH`].
    NestingTooDeep {
        /// The bound.
        max: usize,
    },
    /// A literal's text is not a decimal number.
    InvalidLiteral {
        /// The declaration.
        name: String,
        /// The literal text.
        value: String,
    },
    /// A declaration's arithmetic fails: mismatched units, division by zero, or a value
    /// outside the exact range.
    InvalidExpression {
        /// The declaration.
        name: String,
        /// What is wrong, as the parser words it.
        message: String,
    },
    /// A field uses a variable of another type.
    TypeMismatch {
        /// The variable, without `$`.
        name: String,
        /// The node holding the use.
        node_id: String,
        /// The variable's type.
        found: Unit,
    },
    /// A variable's value cannot stand in the field that uses it (a negative power, a
    /// duration of a fraction of a second, a repeat count that is no whole number ≥ 1, a
    /// reversed range, an implausible power).
    InvalidValue {
        /// The variable, without `$`.
        name: String,
        /// The node holding the use.
        node_id: String,
        /// What is wrong, as the parser words it.
        message: String,
    },
    /// A reference reached a function that takes resolved values only; call
    /// [`resolve_vars`] first.
    Unresolved {
        /// The variable, without `$`.
        name: String,
    },
}
impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownVariable { name } => write!(f, "Unknown variable '${name}'"),
            Self::DuplicateVariable { name } => write!(f, "'${name}' is already declared"),
            Self::InvalidName { name } => write!(f, "'${name}' is not a variable name"),
            Self::TooManyVariables { max } => write!(f, "More than {max} variables"),
            Self::ExpressionTooDeep { name, max } => {
                write!(f, "'${name}': expression nesting exceeds {max} levels")
            }
            Self::NestingTooDeep { max } => write!(f, "Repeat nesting exceeds {max} levels"),
            Self::InvalidLiteral { name, value } => {
                write!(f, "'${name}': '{value}' is not a decimal number")
            }
            Self::InvalidExpression { name, message } => write!(f, "'${name}': {message}"),
            Self::TypeMismatch {
                name,
                node_id,
                found,
            } => write!(
                f,
                "'${name}' is {} and cannot stand in node {node_id}",
                type_phrase(*found)
            ),
            Self::InvalidValue { message, .. } => f.write_str(message),
            Self::Unresolved { name } => {
                write!(f, "'${name}' is not resolved (resolve_vars first)")
            }
        }
    }
}
impl std::error::Error for ResolveError {}

pub(crate) type Rational = Ratio<i128>;

/// A value with its type.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Value {
    pub(crate) unit: Unit,
    pub(crate) r: Rational,
}

/// A failed operation of an expression.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ArithError {
    /// A literal or a result does not fit `Ratio<i128>`.
    Overflow,
    /// A zero divisor.
    DivisionByZero,
    /// The operands' types do not combine with the operator.
    Units { op: Op, l: Unit, r: Unit },
}
impl ArithError {
    pub(crate) fn message(self) -> String {
        match self {
            Self::Overflow => "Number exceeds the exact numeric range".into(),
            Self::DivisionByZero => "Division by zero".into(),
            Self::Units { op: Op::Add, l, r } => {
                format!("Cannot add {} and {}", unit_name(l), unit_name(r))
            }
            Self::Units { op: Op::Sub, l, r } => {
                format!("Cannot subtract {} from {}", unit_name(r), unit_name(l))
            }
            Self::Units { op: Op::Mul, l, .. } => {
                format!("{} can be multiplied only by a plain number", subject(l))
            }
            Self::Units { op: Op::Div, l, .. } => {
                format!("{} can be divided only by a plain number", subject(l))
            }
        }
    }
}
const fn unit_name(u: Unit) -> &'static str {
    match u {
        Unit::Pct => "% FTP",
        Unit::Watts => "watts",
        Unit::Seconds => "a duration",
        Unit::Number => "a plain number",
    }
}
const fn subject(u: Unit) -> &'static str {
    match u {
        Unit::Pct => "A % FTP value",
        Unit::Watts => "A watts value",
        Unit::Seconds => "A duration",
        Unit::Number => "A plain number",
    }
}
/// How a message names a type: "'$d' is a duration".
pub(crate) const fn type_phrase(u: Unit) -> &'static str {
    match u {
        Unit::Pct => "in % FTP",
        Unit::Watts => "in watts",
        Unit::Seconds => "a duration",
        Unit::Number => "a number",
    }
}

// i128::MIN cannot be negated; refusing it keeps every later operation panic-free.
fn checked(r: Option<Rational>) -> Result<Rational, ArithError> {
    r.filter(|r| *r.numer() != i128::MIN)
        .ok_or(ArithError::Overflow)
}

/// The exact value of a decimal `INT ["." INT]`; `None` when `text` is not one.
pub(crate) fn decimal(text: &str) -> Option<Result<Rational, ArithError>> {
    number(text, true)?;
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    let mut numer: i128 = 0;
    let mut denom: i128 = 1;
    for (i, b) in int.bytes().chain(frac.bytes()).enumerate() {
        let next = numer
            .checked_mul(10)
            .and_then(|n| n.checked_add(i128::from(b - b'0')));
        let scale = if i >= int.len() {
            denom.checked_mul(10)
        } else {
            Some(denom)
        };
        let (Some(n), Some(d)) = (next, scale) else {
            return Some(Err(ArithError::Overflow));
        };
        numer = n;
        denom = d;
    }
    Some(Ok(Ratio::new(numer, denom)))
}
/// The exact seconds of an integer h/m/s duration word (lower case); `None` when it is none.
pub(crate) fn duration_seconds(word: &str) -> Option<Result<Rational, ArithError>> {
    parse_duration_word(word)?;
    let mut rest = word;
    let mut total: i128 = 0;
    for (unit, scale) in [('h', 3600), ('m', 60), ('s', 1)] {
        if let Some((n, tail)) = rest.split_once(unit) {
            let Some(Ok(n)) = decimal(n) else {
                return Some(Err(ArithError::Overflow));
            };
            let Some(t) = n
                .to_integer()
                .checked_mul(scale)
                .and_then(|p| total.checked_add(p))
            else {
                return Some(Err(ArithError::Overflow));
            };
            total = t;
            rest = tail;
        }
    }
    Some(Ok(Ratio::from_integer(total)))
}
/// Canonical text of a non-negative decimal: no leading zeros, no trailing fraction zeros.
pub(crate) fn canonical_decimal(text: &str) -> String {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    let int = int.trim_start_matches('0');
    let frac = frac.trim_end_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    if frac.is_empty() {
        int.into()
    } else {
        format!("{int}.{frac}")
    }
}

/// `r` scaled by `10^places` and rounded, ties towards +∞ (the printer's rule).
fn round_scaled(r: &Rational, places: u32) -> Result<i128, ArithError> {
    let scale = 10_i128.checked_pow(places).ok_or(ArithError::Overflow)?;
    let (n, d) = (*r.numer(), *r.denom());
    // floor((2·n·scale + d) / 2d), with d > 0.
    let twice = n
        .checked_mul(scale)
        .and_then(|x| x.checked_mul(2))
        .and_then(|x| x.checked_add(d))
        .ok_or(ArithError::Overflow)?;
    let den = d.checked_mul(2).ok_or(ArithError::Overflow)?;
    Ok(twice.div_euclid(den))
}
/// `scaled / 10^places` as the shortest decimal text.
fn scaled_text(scaled: i128, places: u32) -> String {
    let digits = scaled.unsigned_abs().to_string();
    let places = places as usize;
    let padded = format!("{digits:0>width$}", width = places + 1);
    let (int, frac) = padded.split_at(padded.len() - places);
    let frac = frac.trim_end_matches('0');
    let sign = if scaled < 0 { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}
/// `r` rounded to `places` decimals, read back as the DSL reads that literal.
fn rounded_f64(r: &Rational, places: u32) -> Result<f64, ArithError> {
    let text = scaled_text(round_scaled(r, places)?, places);
    let (negative, digits) = text
        .strip_prefix('-')
        .map_or((false, text.as_str()), |d| (true, d));
    let v = number(digits, true).ok_or(ArithError::Overflow)?;
    Ok(if negative { -v } else { v })
}
/// `r` to three decimals, for messages ("34.286"); a value too large to round shows as a fraction.
pub(crate) fn show(r: &Rational) -> String {
    round_scaled(r, 3).map_or_else(
        |_| format!("{}/{}", r.numer(), r.denom()),
        |s| scaled_text(s, 3),
    )
}
/// Whole, non-negative seconds as `f64`, read as the DSL reads a number (no cast).
fn whole_seconds(r: &Rational) -> Option<f64> {
    (r.is_integer() && *r.numer() >= 0)
        .then(|| number(&r.numer().to_string(), false))
        .flatten()
}
/// A value as the literal that writes it ("4m", "95%", "250w", "3"), for messages.
pub(crate) fn show_value(v: Value) -> String {
    match v.unit {
        Unit::Seconds => {
            whole_seconds(&v.r).map_or_else(|| format!("{} s", show(&v.r)), format_duration)
        }
        Unit::Pct => format!("{}%", show(&v.r)),
        Unit::Watts => format!("{}w", show(&v.r)),
        Unit::Number => show(&v.r),
    }
}

/// The value of a literal of `unit` whose text is `value` (as [`Expr::Lit`] holds it).
pub(crate) fn literal(value: &str, unit: Unit) -> Option<Result<Rational, ArithError>> {
    match unit {
        Unit::Seconds => match decimal(value) {
            Some(Ok(r)) if !r.is_integer() => None,
            other => other,
        },
        _ => decimal(value),
    }
}
pub(crate) fn negate(v: Value) -> Result<Value, ArithError> {
    let n = v.r.numer().checked_neg().ok_or(ArithError::Overflow)?;
    Ok(Value {
        unit: v.unit,
        r: checked(Some(Ratio::new_raw(n, *v.r.denom())))?,
    })
}
pub(crate) fn apply(op: Op, l: Value, r: Value) -> Result<Value, ArithError> {
    let units = ArithError::Units {
        op,
        l: l.unit,
        r: r.unit,
    };
    let unit = match op {
        Op::Add | Op::Sub if l.unit == r.unit => l.unit,
        Op::Mul if l.unit == Unit::Number => r.unit,
        Op::Mul | Op::Div if r.unit == Unit::Number => l.unit,
        _ => return Err(units),
    };
    let value = match op {
        Op::Add => l.r.checked_add(&r.r),
        Op::Sub => l.r.checked_sub(&r.r),
        Op::Mul => l.r.checked_mul(&r.r),
        Op::Div if *r.r.numer() == 0 => return Err(ArithError::DivisionByZero),
        Op::Div => l.r.checked_div(&r.r),
    };
    Ok(Value {
        unit,
        r: checked(value)?,
    })
}

/// The position of a field a variable may stand in (ADR-0008 §2.7, §2.9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// A time step or an `hr<` bound: whole seconds, 0 or more.
    Duration,
    /// A target on its own: % FTP or watts, 0 or more.
    Power,
    /// A range end: % FTP, 0 or more.
    RangeEnd,
    /// A ramp end: % FTP, 0 or more.
    RampEnd,
    /// A repeat count: a whole number ≥ 1.
    Count,
    /// A progression: watts or % FTP, any sign.
    Progression,
}
/// Why a variable's value cannot stand in a slot.
#[derive(Clone, Copy, Debug)]
pub(crate) enum UseError {
    /// Its type is not the slot's.
    Type,
    /// A negative power or duration.
    Negative,
    /// A duration of a fraction of a second.
    Fractional,
    /// A repeat count that is no whole number ≥ 1.
    Count,
    /// The value cannot be rounded within the exact range.
    Overflow,
}
/// The literal a use of `v` in `slot` becomes: its unit and its rounded `f64`
/// (ADR-0008 §2.8 "Rounding: once, at each use").
pub(crate) fn use_value(slot: Slot, v: Value) -> Result<(Unit, f64), UseError> {
    let negative = *v.r.numer() < 0;
    let round = |places| rounded_f64(&v.r, places).map_err(|_| UseError::Overflow);
    match (slot, v.unit) {
        (Slot::Duration, Unit::Seconds) if negative => Err(UseError::Negative),
        (Slot::Duration, Unit::Seconds) if !v.r.is_integer() => Err(UseError::Fractional),
        (Slot::Duration, Unit::Seconds) => Ok((Unit::Seconds, round(0)?)),
        (Slot::Power | Slot::RangeEnd | Slot::RampEnd, Unit::Pct) | (Slot::Power, Unit::Watts)
            if negative =>
        {
            Err(UseError::Negative)
        }
        (Slot::Power | Slot::RangeEnd | Slot::RampEnd | Slot::Progression, Unit::Pct)
        | (Slot::Progression, Unit::Watts) => Ok((v.unit, round(3)?)),
        (Slot::Power, Unit::Watts) => Ok((Unit::Watts, round(0)?)),
        (Slot::Count, Unit::Number) if !v.r.is_integer() || *v.r.numer() < 1 => {
            Err(UseError::Count)
        }
        (Slot::Count, Unit::Number) => Ok((Unit::Number, round(0)?)),
        _ => Err(UseError::Type),
    }
}
/// The parser's message for a use of `$name` (value `v`) refused in `slot`.
pub(crate) fn use_message(name: &str, slot: Slot, e: UseError, v: Value) -> String {
    let shown = show(&v.r);
    match e {
        UseError::Type => match slot {
            Slot::RampEnd => format!("'${name}' is {}; a ramp is in % FTP", type_phrase(v.unit)),
            Slot::RangeEnd => {
                format!("'${name}' is {}; a range is in % FTP", type_phrase(v.unit))
            }
            _ => format!(
                "'${name}' is {} ({}); {}",
                type_phrase(v.unit),
                show_value(v),
                match slot {
                    Slot::Duration => "a duration is expected here, e.g. 4m",
                    Slot::Count => "a repeat count is a plain number, e.g. $n = 3",
                    Slot::Progression => "a progression is in watts or % FTP, e.g. $p = 5w",
                    _ => "a power target is expected here, e.g. 95% or 250w",
                }
            ),
        },
        UseError::Negative if v.unit == Unit::Seconds => {
            format!("'${name}' is {shown} s; a duration must be 0 or more")
        }
        UseError::Negative => format!(
            "'${name}' is {shown} {}; a power must be 0 or more",
            if v.unit == Unit::Watts { "W" } else { "% FTP" }
        ),
        UseError::Fractional => {
            format!("'${name}' is {shown} s; a duration must be whole seconds")
        }
        UseError::Count => {
            format!("Repeat count '${name}' is {shown}; it must be a whole number ≥ 1")
        }
        UseError::Overflow => format!("'${name}' exceeds the exact numeric range"),
    }
}

/// Why a name is no variable name: `None` when it is one.
pub(crate) fn name_fault(name: &str) -> Option<NameFault> {
    if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        Some(NameFault::Start)
    } else if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(NameFault::Chars)
    } else if name.len() > MAX_VARIABLE_NAME {
        Some(NameFault::TooLong)
    } else {
        None
    }
}
/// A fault of a variable name.
#[derive(Clone, Copy, Debug)]
pub(crate) enum NameFault {
    /// It does not start with an ASCII letter.
    Start,
    /// It holds a character other than an ASCII letter, digit or `_`.
    Chars,
    /// It is longer than [`MAX_VARIABLE_NAME`].
    TooLong,
}

/// Declared values, in declaration order.
#[derive(Default)]
pub(crate) struct Env(Vec<(String, Value)>);
impl Env {
    pub(crate) fn get(&self, name: &str) -> Option<Value> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }
    pub(crate) fn insert(&mut self, name: String, v: Value) {
        self.0.push((name, v));
    }
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

fn eval(e: &Expr, env: &Env, decl: &str, depth: usize) -> Result<Value, ResolveError> {
    let deep = || ResolveError::ExpressionTooDeep {
        name: decl.into(),
        max: MAX_EXPRESSION_DEPTH,
    };
    let arith = |e: ArithError| ResolveError::InvalidExpression {
        name: decl.into(),
        message: e.message(),
    };
    match e {
        Expr::Lit { value, unit } => match literal(value, *unit) {
            Some(r) => Ok(Value {
                unit: *unit,
                r: r.map_err(arith)?,
            }),
            None => Err(ResolveError::InvalidLiteral {
                name: decl.into(),
                value: value.clone(),
            }),
        },
        Expr::Var { name } => env
            .get(name)
            .ok_or_else(|| ResolveError::UnknownVariable { name: name.clone() }),
        Expr::Neg { expr } if depth < MAX_EXPRESSION_DEPTH => {
            negate(eval(expr, env, decl, depth + 1)?).map_err(arith)
        }
        Expr::Bin { op, l, r } if depth < MAX_EXPRESSION_DEPTH => apply(
            *op,
            eval(l, env, decl, depth + 1)?,
            eval(r, env, decl, depth + 1)?,
        )
        .map_err(arith),
        Expr::Neg { .. } | Expr::Bin { .. } => Err(deep()),
    }
}
/// The values of `decls`, evaluated top to bottom.
fn declare(decls: &[VarDecl]) -> Result<Env, ResolveError> {
    if decls.len() > MAX_VARIABLES {
        return Err(ResolveError::TooManyVariables { max: MAX_VARIABLES });
    }
    let mut env = Env::default();
    for d in decls {
        if name_fault(&d.name).is_some() {
            return Err(ResolveError::InvalidName {
                name: d.name.clone(),
            });
        }
        if env.contains(&d.name) {
            return Err(ResolveError::DuplicateVariable {
                name: d.name.clone(),
            });
        }
        let v = eval(&d.expr, &env, &d.name, 0)?;
        env.insert(d.name.clone(), v);
    }
    Ok(env)
}

struct Resolver<'a> {
    env: &'a Env,
    ftp: f64,
}
impl Resolver<'_> {
    fn num(&self, n: &mut Num, slot: Slot, node: &str) -> Result<bool, ResolveError> {
        let Num::Var { var } = n else {
            return Ok(false);
        };
        let v = self
            .env
            .get(var)
            .ok_or_else(|| ResolveError::UnknownVariable { name: var.clone() })?;
        let (_, value) = use_value(slot, v).map_err(|e| match e {
            UseError::Type => ResolveError::TypeMismatch {
                name: var.clone(),
                node_id: node.into(),
                found: v.unit,
            },
            e => ResolveError::InvalidValue {
                name: var.clone(),
                node_id: node.into(),
                message: use_message(var, slot, e, v),
            },
        })?;
        *n = Num::Lit(value);
        Ok(true)
    }
    /// A power field of `unit` (a % or a w target, a progression field) using `n`.
    fn typed(&self, n: &mut Num, slot: Slot, unit: Unit, node: &str) -> Result<bool, ResolveError> {
        if let Num::Var { var } = n
            && let Some(v) = self.env.get(var)
            && v.unit != unit
        {
            return Err(ResolveError::TypeMismatch {
                name: var.clone(),
                node_id: node.into(),
                found: v.unit,
            });
        }
        self.num(n, slot, node)
    }
    fn step(&self, s: &mut StepNode) -> Result<(), ResolveError> {
        let id = s.id.clone();
        match &mut s.end {
            StepEnd::Time { s: t } => {
                self.num(t, Slot::Duration, &id)?;
            }
            StepEnd::HrRecovery { min_s, max_s, .. } => {
                self.num(min_s, Slot::Duration, &id)?;
                self.num(max_s, Slot::Duration, &id)?;
            }
            StepEnd::Kj { .. } | StepEnd::Lap => {}
        }
        let names = target_vars(&s.target);
        let bound = match &mut s.target {
            PowerTarget::PctFtp { value } => self.typed(value, Slot::Power, Unit::Pct, &id)?,
            PowerTarget::Watts { value } => self.typed(value, Slot::Power, Unit::Watts, &id)?,
            PowerTarget::Range { lo, hi } => {
                let a = self.num(lo, Slot::RangeEnd, &id)?;
                self.num(hi, Slot::RangeEnd, &id)? || a
            }
            PowerTarget::Ramp { from, to } => {
                let a = self.num(from, Slot::RampEnd, &id)?;
                self.num(to, Slot::RampEnd, &id)? || a
            }
            PowerTarget::Hr { .. } | PowerTarget::Free => false,
        };
        if !bound {
            return Ok(());
        }
        let invalid = |message: String| ResolveError::InvalidValue {
            name: names.first().cloned().unwrap_or_default(),
            node_id: id.clone(),
            message,
        };
        if let PowerTarget::Range {
            lo: Num::Lit(lo),
            hi: Num::Lit(hi),
        } = s.target
            && lo > hi
        {
            return Err(invalid(reversed_range_message(lo, hi)));
        }
        let pct = max_pct(&s.target, self.ftp);
        if pct > 400. {
            return Err(invalid(implausible_message(pct)));
        }
        Ok(())
    }
    fn nodes(&self, nodes: &mut [WorkoutNode], depth: usize) -> Result<(), ResolveError> {
        for n in nodes {
            match n {
                WorkoutNode::Step(s) => self.step(s)?,
                WorkoutNode::Repeat(r) => self.repeat(r, depth)?,
            }
        }
        Ok(())
    }
    fn repeat(&self, r: &mut RepeatNode, depth: usize) -> Result<(), ResolveError> {
        if depth >= MAX_NESTING_DEPTH {
            return Err(ResolveError::NestingTooDeep {
                max: MAX_NESTING_DEPTH,
            });
        }
        let id = r.id.clone();
        self.num(&mut r.count, Slot::Count, &id)?;
        if let Some(Progression {
            watts_per_rep,
            pct_per_rep,
        }) = &mut r.progression
        {
            if let Some(w) = watts_per_rep {
                self.typed(w, Slot::Progression, Unit::Watts, &id)?;
            }
            if let Some(p) = pct_per_rep {
                self.typed(p, Slot::Progression, Unit::Pct, &id)?;
            }
        }
        self.nodes(&mut r.children, depth + 1)
    }
}
fn target_vars(t: &PowerTarget) -> Vec<String> {
    let nums: Vec<&Num> = match t {
        PowerTarget::PctFtp { value } | PowerTarget::Watts { value } => vec![value],
        PowerTarget::Range { lo, hi } => vec![lo, hi],
        PowerTarget::Ramp { from, to } => vec![from, to],
        PowerTarget::Hr { .. } | PowerTarget::Free => vec![],
    };
    nums.into_iter()
        .filter_map(|n| match n {
            Num::Var { var } => Some(var.clone()),
            Num::Lit(_) => None,
        })
        .collect()
}
/// The message of a reversed range, naming the ramp that writes the descent.
pub(crate) fn reversed_range_message(lo: f64, hi: f64) -> String {
    format!(
        "Range low end is above high end (use 'ramp {}-{}%' for a descending ramp)",
        super::dsl::fmt_num(lo),
        super::dsl::fmt_num(hi)
    )
}
pub(crate) fn implausible_message(pct: f64) -> String {
    format!(
        "{} % FTP is implausible (max 400 %)",
        super::dsl::fmt_whole(pct)
    )
}
/// The highest intensity of a resolved target, % FTP (0 for one that holds a reference).
pub(crate) fn max_pct(target: &PowerTarget, ftp: f64) -> f64 {
    let lit = |n: &Num| n.lit().unwrap_or(0.);
    match target {
        PowerTarget::PctFtp { value } => lit(value),
        PowerTarget::Watts { value } if ftp > 0.0 => lit(value) / ftp * 100.0,
        PowerTarget::Range { hi, .. } => lit(hi),
        PowerTarget::Ramp { from, to } => lit(from).max(lit(to)),
        _ => 0.0,
    }
}

/// Whether any field of `nodes` holds a reference; iterative, so safe at any depth.
pub(crate) fn has_refs(nodes: &[WorkoutNode]) -> bool {
    let is_var = |n: &Num| matches!(n, Num::Var { .. });
    let mut stack = vec![nodes];
    while let Some(level) = stack.pop() {
        for n in level {
            match n {
                WorkoutNode::Step(s) => {
                    let end = match &s.end {
                        StepEnd::Time { s } => is_var(s),
                        StepEnd::HrRecovery { min_s, max_s, .. } => is_var(min_s) || is_var(max_s),
                        StepEnd::Kj { .. } | StepEnd::Lap => false,
                    };
                    if end || !target_vars(&s.target).is_empty() {
                        return true;
                    }
                }
                WorkoutNode::Repeat(r) => {
                    let prog = r.progression.as_ref().is_some_and(|p| {
                        p.watts_per_rep.as_ref().is_some_and(is_var)
                            || p.pct_per_rep.as_ref().is_some_and(is_var)
                    });
                    if is_var(&r.count) || prog {
                        return true;
                    }
                    stack.push(&r.children);
                }
            }
        }
    }
    false
}

/// Evaluate the declarations top to bottom and return the workout with every reference
/// replaced by its value, rounded for its position (ADR-0008 §2.8, §2.10); `vars` are kept,
/// so the result still prints as written. Literal fields are not checked here: they keep the
/// compiler's existing diagnostics.
///
/// # Errors
/// [`ResolveError`] for an unknown name, a duplicate or invalid declaration, a type that
/// does not match its field, a value the field cannot hold, or a bound exceeded.
pub fn resolve_vars(w: &Workout) -> Result<Workout, ResolveError> {
    let env = declare(&w.vars)?;
    let mut out = w.clone();
    Resolver {
        env: &env,
        ftp: w.ftp,
    }
    .nodes(&mut out.nodes, 0)?;
    Ok(out)
}

fn print_expr_into(e: &Expr, out: &mut String) {
    let child = |e: &Expr, parens: bool, out: &mut String| {
        if parens {
            out.push('(');
            print_expr_into(e, out);
            out.push(')');
        } else {
            print_expr_into(e, out);
        }
    };
    match e {
        Expr::Lit { value, unit } => match unit {
            Unit::Pct => {
                match decimal(value)
                    .and_then(Result::ok)
                    .map(|r| round_scaled(&r, 3))
                {
                    Some(Ok(s)) => out.push_str(&scaled_text(s, 3)),
                    _ => out.push_str(value),
                }
                out.push('%');
            }
            Unit::Watts => {
                out.push_str(value);
                out.push('w');
            }
            Unit::Seconds => {
                if let Some(s) = literal(value, Unit::Seconds)
                    .and_then(Result::ok)
                    .as_ref()
                    .and_then(whole_seconds)
                {
                    out.push_str(&format_duration(s));
                } else {
                    out.push_str(value);
                    out.push('s');
                }
            }
            Unit::Number => out.push_str(value),
        },
        Expr::Var { name } => {
            out.push('$');
            out.push_str(name);
        }
        Expr::Neg { expr } => {
            out.push('-');
            child(expr, matches!(**expr, Expr::Bin { .. }), out);
        }
        Expr::Bin { op, l, r } => {
            let prec = |e: &Expr| match e {
                Expr::Bin { op, .. } => op.precedence(),
                _ => u8::MAX,
            };
            child(l, prec(l) < op.precedence(), out);
            out.push(' ');
            out.push(op.symbol());
            out.push(' ');
            child(r, prec(r) <= op.precedence(), out);
        }
    }
}
/// Canonical text of an expression (ADR-0008 §2.11): one space around every binary operator,
/// parentheses only where the tree needs them, literals in their canonical forms.
#[must_use]
pub fn print_expr(e: &Expr) -> String {
    let mut out = String::new();
    print_expr_into(e, &mut out);
    out
}
