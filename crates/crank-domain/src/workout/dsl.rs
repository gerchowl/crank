use super::vars::{
    self, ArithError, Env, NameFault, Slot, UseError, Value, apply, canonical_decimal, decimal,
    duration_seconds, name_fault, negate, use_message, use_value,
};
use super::{
    Cadence, DslDoc, DslError, DslErrorKind, Expr, MAX_EXPRESSION_DEPTH, MAX_VARIABLE_NAME,
    MAX_VARIABLES, Num, OnUnreachable, Op, PowerTarget, Progression, RepeatNode, StepEnd, StepNode,
    Unit, VarDecl, WorkoutNode,
};
/// Maximum number of enclosing repeats accepted from untrusted text.
pub const MAX_NESTING_DEPTH: usize = 32;
// The route grade-profile grammar (ADR-0005 §2.2.5) shares this lexer and its number rules.
#[derive(Clone)]
pub(crate) struct Token {
    pub(crate) kind: char,
    pub(crate) text: String,
    pub(crate) offset: usize,
    pub(crate) length: usize,
}
impl Token {
    fn error(&self, kind: DslErrorKind, message: impl Into<String>) -> DslError {
        DslError {
            kind,
            message: message.into(),
            offset: self.offset,
            length: self.length,
        }
    }
    /// An error selecting the bytes `start..end` of this word (UTF-16 span).
    fn error_at(
        &self,
        start: usize,
        end: usize,
        kind: DslErrorKind,
        message: impl Into<String>,
    ) -> DslError {
        let utf16 = |s: &str| s.encode_utf16().count();
        DslError {
            kind,
            message: message.into(),
            offset: self.offset + utf16(&self.text[..start]),
            length: utf16(&self.text[start..end]),
        }
    }
}
// ECMAScript whitespace differs from Rust's Unicode White_Space (notably FEFF/0085).
fn whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
pub(crate) fn lex(src: &str) -> Result<Vec<Token>, DslError> {
    let mut chars = src.chars().peekable();
    let mut offset = 0;
    let mut tokens = Vec::new();
    while let Some(c) = chars.next() {
        let start = offset;
        offset += c.len_utf16();
        if c == '#' {
            while chars.peek().is_some_and(|c| *c != '\n') {
                if let Some(c) = chars.next() {
                    offset += c.len_utf16();
                }
            }
            continue;
        }
        let kind = match c {
            '\n' | ',' | ';' => ',',
            '(' | ')' | '[' | ']' => c,
            '"' => 's',
            c if whitespace(c) => continue,
            _ => 'w',
        };
        let mut text = String::new();
        if kind == 's' {
            loop {
                match chars.peek().copied() {
                    None | Some('\n') => {
                        return Err(Token {
                            kind,
                            text,
                            offset: start,
                            length: offset - start,
                        }
                        .error(DslErrorKind::UnterminatedLabel, "Unterminated label string"));
                    }
                    Some('"') => {
                        chars.next();
                        offset += 1;
                        break;
                    }
                    Some(c) => {
                        chars.next();
                        offset += c.len_utf16();
                        if c == '\\' && chars.peek().is_some() {
                            if let Some(c) = chars.next() {
                                text.push(c);
                                offset += c.len_utf16();
                            }
                        } else {
                            text.push(c);
                        }
                    }
                }
            }
        } else {
            text.push(c);
            if kind == 'w' {
                while chars
                    .peek()
                    .is_some_and(|c| !whitespace(*c) && !"()[],;\"#".contains(*c))
                {
                    if let Some(c) = chars.next() {
                        text.push(c);
                        offset += c.len_utf16();
                    }
                }
            }
        }
        let token = Token {
            kind,
            text,
            offset: start,
            length: offset - start,
        };
        if kind == 'w'
            && is_numeric_token(&token.text)
            && token
                .text
                .split(|c: char| !c.is_ascii_digit() && c != '.')
                .filter_map(|part| part.parse::<f64>().ok())
                .any(|n| !n.is_finite())
        {
            return Err(token.error(
                DslErrorKind::NumericOverflow,
                "Number exceeds finite numeric range",
            ));
        }
        tokens.push(token);
    }
    tokens.push(Token {
        kind: 'e',
        text: String::new(),
        offset,
        length: 0,
    });
    Ok(tokens)
}
// Only tokens shaped like a DSL number can overflow; any other word must reach the parser, which
// reports `UnknownToken` as TS does (#174). A sign is legal only on a repeat-progression
// increment (`+5w`, `-2%`: digits and `.` before `w`/`%`); an unsigned token is digits, `.`/`-`/
// `h`/`m`/`s` (ranges, durations), an optional `hr<` prefix, then an optional unit. TS also
// accepts an overflowing `hr<999…` and carries an infinite target; Rust reports it as
// `NumericOverflow` because compiled output must be finite (#160).
fn is_numeric_token(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if let Some(body) = lower.strip_prefix(['+', '-']) {
        let digits = body.strip_suffix(['w', '%']).unwrap_or("");
        return digits.starts_with(|c: char| c.is_ascii_digit())
            && digits.chars().all(|c| c.is_ascii_digit() || c == '.');
    }
    let body = lower.strip_prefix("hr<").unwrap_or(&lower);
    let body = ["kj", "bpm", "rpm", "x", "w", "%"]
        .iter()
        .find_map(|unit| body.strip_suffix(unit))
        .unwrap_or(body);
    body.starts_with(|c: char| c.is_ascii_digit())
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | 'h' | 'm' | 's'))
}
pub(crate) fn number(s: &str, decimal: bool) -> Option<f64> {
    let mut parts = s.split('.');
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    if !digits(parts.next()?) {
        return None;
    }
    if let Some(part) = parts.next()
        && (!decimal || !digits(part) || parts.next().is_some())
    {
        return None;
    }
    s.parse().ok()
}
pub(crate) fn suffix(s: &str, unit: &str, decimal: bool) -> Option<f64> {
    number(s.strip_suffix(unit)?, decimal)
}
fn range(s: &str, unit: &str, decimal: bool) -> Option<(f64, f64)> {
    let (lo, hi) = s.strip_suffix(unit)?.split_once('-')?;
    Some((number(lo, decimal)?, number(hi, decimal)?))
}
/// Parse an integer h/m/s duration word, case-insensitively; zero is allowed here.
#[must_use]
pub fn parse_duration_word(word: &str) -> Option<f64> {
    let word = word.to_ascii_lowercase();
    let mut rest = word.as_str();
    let mut total = 0.0;
    for (unit, scale) in [('h', 3600.0), ('m', 60.0), ('s', 1.0)] {
        if let Some((n, tail)) = rest.split_once(unit) {
            total += number(n, false)? * scale;
            rest = tail;
        }
    }
    (!word.is_empty() && rest.is_empty()).then_some(total)
}
struct Parser<'a> {
    ids: &'a mut dyn FnMut() -> String,
    tokens: Vec<Token>,
    i: usize,
    ftp: f64,
    /// Declarations so far, with their values.
    decls: Vec<VarDecl>,
    env: Env,
}
/// An expression as read: its tree, its value and its tree depth.
struct Read {
    expr: Expr,
    value: Value,
    depth: usize,
}
/// A field read from a word: as written (`Num`) and as its value (the use's rounded literal).
struct Field {
    num: Num,
    value: f64,
    unit: Unit,
}
fn expected_value(t: &Token) -> DslError {
    let message = if t.kind == 'e' || t.kind == ',' {
        "Expected a value (95%, 250w, 4m, 3) or a variable".to_owned()
    } else {
        format!(
            "Expected a value (95%, 250w, 4m, 3) or a variable; got '{}'",
            t.text
        )
    };
    t.error(DslErrorKind::ExpectedExpression, message)
}
fn expected_operator(t: &Token) -> DslError {
    t.error(
        DslErrorKind::ExpectedOperator,
        format!(
            "Expected an operator (+, -, *, /) or the end of the declaration; got '{}'",
            t.text
        ),
    )
}
const OPERATORS: [char; 4] = ['+', '-', '*', '/'];
fn op_of(text: &str) -> Option<Op> {
    match text {
        "+" => Some(Op::Add),
        "-" => Some(Op::Sub),
        "*" => Some(Op::Mul),
        "/" => Some(Op::Div),
        _ => None,
    }
}
fn arith_kind(e: ArithError) -> DslErrorKind {
    match e {
        ArithError::Overflow => DslErrorKind::NumericOverflow,
        ArithError::DivisionByZero => DslErrorKind::DivisionByZero,
        ArithError::Units {
            op: Op::Add | Op::Sub,
            ..
        } => DslErrorKind::MismatchedUnits,
        ArithError::Units { op: Op::Mul, .. } => DslErrorKind::InvalidProduct,
        ArithError::Units { op: Op::Div, .. } => DslErrorKind::InvalidQuotient,
    }
}
/// `NUMBER "%"` on its own (`60%`), as a % FTP value.
fn pct_literal(text: &str) -> Option<f64> {
    suffix(&text.to_ascii_lowercase(), "%", true)
}
impl Parser<'_> {
    fn peek(&self) -> &Token {
        &self.tokens[self.i]
    }
    fn next(&mut self) -> Token {
        let t = self.peek().clone();
        if t.kind != 'e' {
            self.i += 1;
        }
        t
    }
    /// Whether the next word starts a declaration: `$name =`, or a `$` word holding `=`.
    fn at_declaration(&self) -> bool {
        let t = self.peek();
        t.kind == 'w'
            && t.text.starts_with('$')
            && (t.text.contains('=')
                || self
                    .tokens
                    .get(self.i + 1)
                    .is_some_and(|n| n.kind == 'w' && n.text.starts_with('=')))
    }
    fn seq(&mut self, open: Option<&Token>, depth: usize) -> Result<Vec<WorkoutNode>, DslError> {
        let mut nodes = Vec::new();
        loop {
            while self.peek().kind == ',' {
                self.next();
            }
            if self.at_declaration() {
                if open.is_some() || !nodes.is_empty() {
                    let t = self.peek();
                    let end = t.text.find('=').unwrap_or(t.text.len());
                    return Err(t.error_at(
                        0,
                        end,
                        DslErrorKind::DeclarationAfterBlock,
                        "Variables are declared before the first block",
                    ));
                }
                self.declaration()?;
                continue;
            }
            let t = self.peek();
            match t.kind {
                'e' => {
                    return open.map_or(Ok(nodes), |o| {
                        Err(o.error(DslErrorKind::UnclosedRepeat, "Unclosed '(' — missing ')'"))
                    });
                }
                ')' => {
                    return if open.is_some() {
                        Ok(nodes)
                    } else {
                        Err(t.error(DslErrorKind::UnexpectedToken, "Unexpected ')'"))
                    };
                }
                _ => nodes.push(self.item(depth)?),
            }
            let after = self.peek();
            if !matches!(after.kind, ',' | 'e' | ')') {
                return Err(after.error(
                    DslErrorKind::MissingSeparator,
                    format!(
                        "Unexpected '{}' — separate items with ',', ';' or a newline",
                        after.text
                    ),
                ));
            }
        }
    }
    fn item(&mut self, depth: usize) -> Result<WorkoutNode, DslError> {
        let label = (self.peek().kind == 's').then(|| self.next());
        if let Some(l) = label.as_ref().filter(|_| self.at_declaration()) {
            return Err(l.error(
                DslErrorKind::LabelledDeclaration,
                "A declaration has no label — put the label on a block",
            ));
        }
        let label = label.map(|l| l.text);
        let t = self.next();
        if t.kind != 'w' {
            // Bare EOF is excluded by seq; only a dangling label reaches EOF here.
            return Err(t.error(
                if t.kind == 'e' {
                    DslErrorKind::MissingItemAfterLabel
                } else {
                    DslErrorKind::UnexpectedToken
                },
                if t.kind == 'e' {
                    "Expected a step or repeat after the label".into()
                } else {
                    format!("Unexpected '{}'", t.text)
                },
            ));
        }
        if let Some((count, head)) = self.repeat_head(&t, depth)? {
            self.repeat(count, &t, &head, label, depth)
                .map(WorkoutNode::Repeat)
        } else {
            self.step(&t, label).map(WorkoutNode::Step)
        }
    }
    /// A repeat's count and the words that wrote it: `3x`, or `$n x` (two words, ADR-0008
    /// §2.9); `None` for a step. Kept out of the recursive `seq`/`item`/`repeat` frames, which
    /// bound the stack at the deepest nesting.
    #[inline(never)]
    fn repeat_head(&mut self, t: &Token, depth: usize) -> Result<Option<(Num, String)>, DslError> {
        if let Some(count) = suffix(&t.text.to_ascii_lowercase(), "x", false) {
            Self::nesting(t, depth)?;
            if count < 1.0 {
                return Err(t.error(
                    DslErrorKind::InvalidRepeatCount { count },
                    "Repeat count must be ≥ 1",
                ));
            }
            return Ok(Some((Num::Lit(count), t.text.clone())));
        }
        if t.text.starts_with('$')
            && self.peek().kind == 'w'
            && self.peek().text.eq_ignore_ascii_case("x")
        {
            Self::nesting(t, depth)?;
            let count = self.use_var(t, 0, t.text.len(), Slot::Count)?;
            let x = self.next();
            return Ok(Some((count.num, format!("{} {}", t.text, x.text))));
        }
        Ok(None)
    }
    fn nesting(t: &Token, depth: usize) -> Result<(), DslError> {
        if depth == MAX_NESTING_DEPTH {
            return Err(t.error(
                DslErrorKind::NestingTooDeep {
                    max: MAX_NESTING_DEPTH,
                },
                format!("Repeat nesting exceeds {MAX_NESTING_DEPTH} levels"),
            ));
        }
        Ok(())
    }
    fn repeat(
        &mut self,
        count: Num,
        t: &Token,
        head: &str,
        label: Option<String>,
        depth: usize,
    ) -> Result<RepeatNode, DslError> {
        let mut node = RepeatNode {
            id: (self.ids)(),
            label,
            count,
            children: Vec::new(),
            progression: None,
            skip_last_rest: None,
        };
        if self.peek().kind == '[' {
            self.options(&mut node)?;
        }
        let open = self.next();
        if open.kind != '(' {
            return Err(open.error(
                DslErrorKind::MissingRepeatBody,
                format!("Expected '(' after '{head}'"),
            ));
        }
        node.children = self.seq(Some(&open), depth + 1)?;
        let close = self.next();
        if node.children.is_empty() {
            return Err(DslError {
                kind: DslErrorKind::EmptyRepeat,
                message: "Empty repeat".into(),
                offset: t.offset,
                length: close.offset + 1 - t.offset,
            });
        }
        Ok(node)
    }
    /// `[…]` repeat options into `node`; out of line for the same stack bound as `repeat_head`.
    #[inline(never)]
    fn options(&mut self, node: &mut RepeatNode) -> Result<(), DslError> {
        let open = self.next();
        let mut prog = Progression::default();
        loop {
            let o = self.next();
            match o.kind {
                ']' => break,
                ',' => continue,
                'e' => {
                    return Err(
                        open.error(DslErrorKind::UnclosedOptions, "Unclosed '[' — missing ']'")
                    );
                }
                'w' => {}
                _ => {
                    return Err(o.error(
                        DslErrorKind::UnexpectedRepeatOption,
                        format!("Unexpected '{}' in repeat options", o.text),
                    ));
                }
            }
            if o.text.starts_with("+$") {
                let f = self.use_var(&o, 1, o.text.len(), Slot::Progression)?;
                let unit = self.env.get(&o.text[2..]).map(|v| v.unit);
                if unit == Some(Unit::Watts) {
                    prog.watts_per_rep = Some(f.num);
                } else {
                    prog.pct_per_rep = Some(f.num);
                }
                continue;
            }
            if let Some(name) = o
                .text
                .strip_prefix("-$")
                .or_else(|| o.text.strip_prefix('$'))
            {
                return Err(o.error(
                    DslErrorKind::InvalidProgressionVariable,
                    if o.text.starts_with('-') {
                        format!(
                            "Write a falling progression as a negative variable: ${name} = -5w, then [+${name}]"
                        )
                    } else {
                        format!("Write a progression variable with its sign: [+${name}]")
                    },
                ));
            }
            let x = o.text.to_ascii_lowercase();
            let signed = |unit| {
                let (sign, tail) = x.split_at_checked(1)?;
                let factor = match sign {
                    "+" => 1.0,
                    "-" => -1.0,
                    _ => return None,
                };
                Some(factor * suffix(tail, unit, true)?)
            };
            if let Some(w) = signed("w") {
                prog.watts_per_rep = Some(Num::Lit(w));
            } else if let Some(p) = signed("%") {
                prog.pct_per_rep = Some(Num::Lit(p));
            } else if x == "!last" {
                node.skip_last_rest = Some(true);
            } else {
                return Err(o.error(
                    DslErrorKind::UnknownRepeatOption,
                    format!("Unknown repeat option '{}' (use +5w, +2%, !last)", o.text),
                ));
            }
        }
        if prog != Progression::default() {
            node.progression = Some(prog);
        }
        Ok(())
    }
    fn end(&mut self, t: &Token) -> Result<StepEnd, DslError> {
        if t.text.starts_with('$') {
            // A duration variable; 0 s is the compiler's `InvalidDuration`, as ADR-0008 §2.8 says.
            let f = self.use_var(t, 0, t.text.len(), Slot::Duration)?;
            return Ok(StepEnd::Time { s: f.num });
        }
        let w = t.text.to_ascii_lowercase();
        if let Some(s) = parse_duration_word(&w) {
            if !s.is_finite() {
                return Err(t.error(
                    DslErrorKind::NumericOverflow,
                    "Duration exceeds finite numeric range",
                ));
            }
            return if s <= 0.0 {
                Err(t.error(DslErrorKind::NonPositiveDuration, "Duration must be > 0"))
            } else {
                Ok(StepEnd::Time { s: Num::Lit(s) })
            };
        }
        if let Some(kj) = suffix(&w, "kj", true) {
            return Ok(StepEnd::Kj { kj });
        }
        if w == "lap" {
            return Ok(StepEnd::Lap);
        }
        if let Some(target_bpm) = w.strip_prefix("hr<").and_then(|s| number(s, false)) {
            let rt = self.peek().clone();
            let bounds = match rt.text.split_once("..").filter(|_| rt.kind == 'w') {
                Some((lo, hi)) => {
                    let lo = self.bound(&rt, 0, lo)?;
                    let hi = self.bound(&rt, rt.text.len() - hi.len(), hi)?;
                    lo.zip(hi)
                }
                None => None,
            };
            let Some((min, max)) = bounds else {
                return Err(if rt.kind == 'w' { &rt } else { t }.error(
                    DslErrorKind::MissingRecoveryBounds,
                    format!("Expected min..max duration after '{}', e.g. 2m..4m", t.text),
                ));
            };
            let rt = self.next();
            let (min_s, max_s) = (min.value, max.value);
            if !min_s.is_finite() || !max_s.is_finite() {
                return Err(rt.error(
                    DslErrorKind::NumericOverflow,
                    "Duration exceeds finite numeric range",
                ));
            }
            if min_s > max_s {
                return Err(rt.error(
                    DslErrorKind::ReversedRecoveryBounds { min_s, max_s },
                    "Min duration is longer than max",
                ));
            }
            return Ok(StepEnd::HrRecovery {
                target_bpm,
                min_s: min.num,
                max_s: max.num,
                on_unreachable: OnUnreachable::LowerPower,
            });
        }
        let (kind, message) = if number(&w, false).is_some() {
            (
                DslErrorKind::MissingUnit,
                format!(
                    "'{}' needs a unit: 30s, 4m, 1h, 10kj — or '{}x(...)' for a repeat",
                    t.text, t.text
                ),
            )
        } else {
            (
                DslErrorKind::InvalidStepEnd,
                format!(
                    "Expected a duration (30s, 4m, 1m30s), kJ (10kj), 'lap', 'hr<N' or a repeat (3x(...)); got '{}'",
                    t.text
                ),
            )
        };
        Err(t.error(kind, message))
    }
    // ---- variables (ADR-0008 §2.6–§2.9) ----------------------------------------------------
    fn check_name(t: &Token, start: usize, end: usize) -> Result<(), DslError> {
        let name = &t.text[start + 1..end];
        let (kind, message) = match name_fault(name) {
            None => return Ok(()),
            Some(NameFault::Start) => (
                DslErrorKind::InvalidVariableName,
                "A variable name starts with a letter, e.g. $thr, $rest_2".to_owned(),
            ),
            Some(NameFault::Chars) => (
                DslErrorKind::InvalidVariableName,
                "A variable name has only letters, digits and '_', e.g. $thr, $rest_2".to_owned(),
            ),
            Some(NameFault::TooLong) => (
                DslErrorKind::VariableNameTooLong {
                    max: MAX_VARIABLE_NAME,
                },
                format!("A variable name has at most {MAX_VARIABLE_NAME} characters"),
            ),
        };
        Err(t.error_at(start, end, kind, message))
    }
    /// The use of the variable at bytes `start..end` of `t` in `slot`, checked with its span.
    fn use_var(&self, t: &Token, start: usize, end: usize, slot: Slot) -> Result<Field, DslError> {
        Self::check_name(t, start, end)?;
        let name = &t.text[start + 1..end];
        let Some(v) = self.env.get(name) else {
            return Err(t.error_at(
                start,
                end,
                DslErrorKind::UnknownVariable,
                format!(
                    "Unknown variable '${name}' — declare it before the first block, e.g. ${name} = 95%"
                ),
            ));
        };
        let (unit, value) = use_value(slot, v).map_err(|e| {
            let kind = match e {
                UseError::Type => DslErrorKind::VariableTypeMismatch,
                UseError::Negative if v.unit == Unit::Seconds => DslErrorKind::NegativeDuration,
                UseError::Negative => DslErrorKind::NegativePower,
                UseError::Fractional => DslErrorKind::FractionalDuration,
                UseError::Count => DslErrorKind::InvalidRepeatCount {
                    count: vars::show(&v.r).parse().unwrap_or(f64::NAN),
                },
                UseError::Overflow => DslErrorKind::NumericOverflow,
            };
            let message = use_message(name, slot, e, v);
            // A ramp's or a range's type error names the form, so it selects the whole word.
            if matches!(e, UseError::Type) && matches!(slot, Slot::RampEnd | Slot::RangeEnd) {
                t.error(kind, message)
            } else {
                t.error_at(start, end, kind, message)
            }
        })?;
        Ok(Field {
            num: Num::var(name),
            value,
            unit,
        })
    }
    /// One `hr<` bound of `rt` at byte `start`: a duration word or a duration variable.
    fn bound(&self, rt: &Token, start: usize, part: &str) -> Result<Option<Field>, DslError> {
        if part.starts_with('$') {
            return self
                .use_var(rt, start, start + part.len(), Slot::Duration)
                .map(Some);
        }
        Ok(parse_duration_word(part).map(|s| Field {
            num: Num::Lit(s),
            value: s,
            unit: Unit::Seconds,
        }))
    }
    /// A target word holding a `$`: a variable on its own, or a range with one.
    fn var_target(&self, a: &Token) -> Result<PowerTarget, DslError> {
        if let Some(pt) = self.pair(a, false)? {
            return Ok(pt);
        }
        if a.text.starts_with('$') {
            let f = self.use_var(a, 0, a.text.len(), Slot::Power)?;
            return Ok(if f.unit == Unit::Watts {
                PowerTarget::Watts { value: f.num }
            } else {
                PowerTarget::PctFtp { value: f.num }
            });
        }
        Err(a.error(
            DslErrorKind::UnknownToken,
            format!("Unknown token '{}'", a.text),
        ))
    }
    /// The new form of a range or a ramp's ends (`$lo-$hi`, `$a-75%`), each number with its own
    /// `%`; `None` for a word of neither form. `60%-80%` is refused with today's spelling.
    fn pair(&self, t: &Token, ramp: bool) -> Result<Option<PowerTarget>, DslError> {
        let Some((l, r)) = t.text.split_once('-') else {
            return Ok(None);
        };
        let what = if ramp { "ramp " } else { "" };
        let (lvar, rvar) = (l.starts_with('$'), r.starts_with('$'));
        if !lvar && !rvar {
            return match (pct_literal(l), pct_literal(r)) {
                (Some(_), Some(_)) => Err(t.error(
                    DslErrorKind::RedundantPercent,
                    format!(
                        "Write a {} between two numbers as {what}{}-{}%",
                        if ramp { "ramp" } else { "range" },
                        &l[..l.len() - 1],
                        &r[..r.len() - 1]
                    ),
                )),
                _ => Ok(None),
            };
        }
        let slot = if ramp { Slot::RampEnd } else { Slot::RangeEnd };
        let side = |part: &str, start: usize| -> Result<Option<Field>, DslError> {
            if part.starts_with('$') {
                return self.use_var(t, start, start + part.len(), slot).map(Some);
            }
            if let Some(v) = pct_literal(part) {
                return Ok(Some(Field {
                    num: Num::Lit(v),
                    value: v,
                    unit: Unit::Pct,
                }));
            }
            if number(part, true).is_some() {
                let fix = if start == 0 {
                    format!("{what}{l}%-{r}")
                } else {
                    format!("{what}{l}-{r}%")
                };
                return Err(t.error(
                    DslErrorKind::MissingPercent,
                    format!("Write the % on the number next to a variable, e.g. {fix}"),
                ));
            }
            Ok(None)
        };
        let Some(lo) = side(l, 0)? else {
            return Ok(None);
        };
        let Some(hi) = side(r, l.len() + 1)? else {
            return Ok(None);
        };
        if ramp {
            return Ok(Some(PowerTarget::Ramp {
                from: lo.num,
                to: hi.num,
            }));
        }
        if lo.value > hi.value {
            return Err(t.error(
                DslErrorKind::ReversedPowerRange,
                vars::reversed_range_message(lo.value, hi.value),
            ));
        }
        Ok(Some(PowerTarget::Range {
            lo: lo.num,
            hi: hi.num,
        }))
    }
    /// `t` with every reference replaced by the value its use gives (for the parser's checks).
    fn resolved(&self, t: &PowerTarget) -> PowerTarget {
        let value = |n: &Num, slot| match n {
            Num::Lit(v) => Num::Lit(*v),
            Num::Var { var } => Num::Lit(
                self.env
                    .get(var)
                    .and_then(|v| use_value(slot, v).ok())
                    .map_or(0., |(_, v)| v),
            ),
        };
        match t {
            PowerTarget::PctFtp { value: v } => PowerTarget::PctFtp {
                value: value(v, Slot::Power),
            },
            PowerTarget::Watts { value: v } => PowerTarget::Watts {
                value: value(v, Slot::Power),
            },
            PowerTarget::Range { lo, hi } => PowerTarget::Range {
                lo: value(lo, Slot::RangeEnd),
                hi: value(hi, Slot::RangeEnd),
            },
            PowerTarget::Ramp { from, to } => PowerTarget::Ramp {
                from: value(from, Slot::RampEnd),
                to: value(to, Slot::RampEnd),
            },
            other => other.clone(),
        }
    }
    /// `$name = expr`; the caller has seen the `$` word and the `=` (or a word holding it).
    fn declaration(&mut self) -> Result<(), DslError> {
        let name_tok = self.next();
        if let Some((lhs, rhs)) = name_tok.text.split_once('=') {
            return Err(name_tok.error(
                DslErrorKind::UnspacedEquals,
                format!("Put spaces around '=': {lhs} = {rhs}"),
            ));
        }
        let eq = self.next();
        if eq.text != "=" {
            return Err(eq.error(
                DslErrorKind::UnspacedEquals,
                format!(
                    "Put spaces around '=': {} = {}",
                    name_tok.text,
                    &eq.text[1..]
                ),
            ));
        }
        Self::check_name(&name_tok, 0, name_tok.text.len())?;
        let name = name_tok.text[1..].to_owned();
        if self.decls.len() == MAX_VARIABLES {
            return Err(name_tok.error(
                DslErrorKind::TooManyVariables { max: MAX_VARIABLES },
                format!("At most {MAX_VARIABLES} variables can be declared"),
            ));
        }
        if self.env.contains(&name) {
            return Err(name_tok.error(
                DslErrorKind::DuplicateVariable,
                format!("'${name}' is already declared"),
            ));
        }
        let read = self.expr(0)?;
        let after = self.peek();
        if !matches!(after.kind, ',' | 'e') {
            return Err(expected_operator(after));
        }
        self.env.insert(name.clone(), read.value);
        self.decls.push(VarDecl {
            name,
            expr: read.expr,
        });
        Ok(())
    }
    fn deeper(t: &Token, depth: usize) -> Result<usize, DslError> {
        if depth > MAX_EXPRESSION_DEPTH {
            return Err(t.error(
                DslErrorKind::ExpressionTooDeep {
                    max: MAX_EXPRESSION_DEPTH,
                },
                format!("Expression nesting exceeds {MAX_EXPRESSION_DEPTH} levels"),
            ));
        }
        Ok(depth)
    }
    fn binary(op_tok: &Token, op: Op, l: Read, r: Read) -> Result<Read, DslError> {
        let depth = Self::deeper(op_tok, 1 + l.depth.max(r.depth))?;
        let value =
            apply(op, l.value, r.value).map_err(|e| op_tok.error(arith_kind(e), e.message()))?;
        Ok(Read {
            expr: Expr::Bin {
                op,
                l: Box::new(l.expr),
                r: Box::new(r.expr),
            },
            value,
            depth,
        })
    }
    fn neg(t: &Token, inner: Read) -> Result<Read, DslError> {
        let depth = Self::deeper(t, inner.depth + 1)?;
        let value = negate(inner.value).map_err(|e| t.error(arith_kind(e), e.message()))?;
        Ok(Read {
            expr: Expr::Neg {
                expr: Box::new(inner.expr),
            },
            value,
            depth,
        })
    }
    fn peek_op(&self, ops: [Op; 2]) -> Option<Op> {
        let t = self.peek();
        (t.kind == 'w')
            .then(|| op_of(&t.text))
            .flatten()
            .filter(|op| ops.contains(op))
    }
    /// `term { ("+" | "-") term }`; `nest` counts the parentheses and prefix minus words
    /// being read, so recursion stays within the bound.
    fn expr(&mut self, nest: usize) -> Result<Read, DslError> {
        let mut l = self.term(nest)?;
        while let Some(op) = self.peek_op([Op::Add, Op::Sub]) {
            let op_tok = self.next();
            let r = self.term(nest)?;
            l = Self::binary(&op_tok, op, l, r)?;
        }
        Ok(l)
    }
    fn term(&mut self, nest: usize) -> Result<Read, DslError> {
        let mut l = self.factor(nest)?;
        while let Some(op) = self.peek_op([Op::Mul, Op::Div]) {
            let op_tok = self.next();
            let r = self.factor(nest)?;
            l = Self::binary(&op_tok, op, l, r)?;
        }
        Ok(l)
    }
    fn factor(&mut self, nest: usize) -> Result<Read, DslError> {
        let t = self.peek().clone();
        match t.kind {
            // Prefix minus words: `-`, and `--` as the printer writes `-(-(…))`.
            'w' if t.text.bytes().all(|b| b == b'-') => {
                self.next();
                Self::deeper(&t, nest + 1)?;
                let mut read = self.factor(nest + 1)?;
                for _ in 0..t.text.len() {
                    read = Self::neg(&t, read)?;
                }
                Ok(read)
            }
            '(' => {
                self.next();
                Self::deeper(&t, nest + 1)?;
                let inner = self.expr(nest + 1)?;
                let close = self.peek();
                match close.kind {
                    ')' => {
                        self.next();
                        Ok(inner)
                    }
                    ',' | 'e' => Err(t.error(
                        DslErrorKind::UnclosedParenthesis,
                        "Unclosed '(' — missing ')'",
                    )),
                    _ => Err(expected_operator(close)),
                }
            }
            'w' if op_of(&t.text).is_none() => {
                self.next();
                self.operand(&t)
            }
            _ => Err(expected_value(&t)),
        }
    }
    /// One word of an expression: a literal, `$name`, either with attached minus signs.
    fn operand(&self, t: &Token) -> Result<Read, DslError> {
        let dashes = t.text.len() - t.text.trim_start_matches('-').len();
        if dashes == t.text.len() {
            return Err(expected_value(t));
        }
        let mut read = self.atom(t, dashes)?;
        for _ in 0..dashes {
            read = Self::neg(t, read)?;
        }
        Ok(read)
    }
    fn atom(&self, t: &Token, start: usize) -> Result<Read, DslError> {
        let body = &t.text[start..];
        if let Some(op) = body.chars().skip(1).find(|c| OPERATORS.contains(c)) {
            let mut fixed = String::new();
            for (i, c) in t.text.char_indices() {
                if i > start && OPERATORS.contains(&c) {
                    fixed.push(' ');
                    fixed.push(c);
                    fixed.push(' ');
                } else {
                    fixed.push(c);
                }
            }
            return Err(t.error(
                DslErrorKind::UnspacedOperator,
                format!("Put spaces around '{op}' in an expression: {fixed}"),
            ));
        }
        if let Some(name) = body.strip_prefix('$') {
            Self::check_name(t, start, t.text.len())?;
            let Some(value) = self.env.get(name) else {
                return Err(t.error_at(
                    start,
                    t.text.len(),
                    DslErrorKind::UnknownVariable,
                    format!(
                        "Unknown variable '${name}' — a declaration can use only variables declared above it"
                    ),
                ));
            };
            return Ok(Read {
                expr: Expr::Var { name: name.into() },
                value,
                depth: 0,
            });
        }
        let lower = body.to_ascii_lowercase();
        let unit_lit = |unit: char| {
            lower
                .strip_suffix(unit)
                .and_then(|n| decimal(n).map(|r| (canonical_decimal(n), r)))
        };
        let (unit, (text, r)) = if let Some(l) = unit_lit('%') {
            (Unit::Pct, l)
        } else if let Some(l) = unit_lit('w') {
            (Unit::Watts, l)
        } else if let Some(r) = duration_seconds(&lower) {
            let text = r
                .as_ref()
                .map_or_else(|_| String::new(), |r| r.numer().to_string());
            (Unit::Seconds, (text, r))
        } else if let Some(r) = decimal(&lower) {
            (Unit::Number, (canonical_decimal(&lower), r))
        } else {
            return Err(expected_value(t));
        };
        let r = r.map_err(|e| t.error(arith_kind(e), e.message()))?;
        Ok(Read {
            expr: Expr::Lit { value: text, unit },
            value: Value { unit, r },
            depth: 0,
        })
    }
    /// The ends after `ramp` (the token `a`): today's `45-75%`, or the variable form.
    fn ramp(&mut self, a: &Token) -> Result<(PowerTarget, Token), DslError> {
        let r = self.next();
        let pt = if let Some((from, to)) = range(&r.text, "%", true).filter(|_| r.kind == 'w') {
            PowerTarget::Ramp {
                from: Num::Lit(from),
                to: Num::Lit(to),
            }
        } else if r.kind == 'w'
            && let Some(pt) = self.pair(&r, true)?
        {
            pt
        } else {
            return Err(if r.kind == 'e' { a } else { &r }.error(
                DslErrorKind::MissingRampRange,
                "Expected a range after 'ramp', e.g. ramp 50-75%",
            ));
        };
        Ok((pt, r))
    }
    fn step(&mut self, t: &Token, label: Option<String>) -> Result<StepNode, DslError> {
        let mut end = self.end(t)?;
        let mut target = None;
        let mut cadence = None;
        let mut target_tok = t.clone();
        while self.peek().kind == 'w' {
            let a = self.next();
            let x = a.text.to_ascii_lowercase();
            let pt = if x == "ramp" {
                let (pt, r) = self.ramp(&a)?;
                target_tok = r;
                pt
            } else if a.text.contains('$') {
                self.var_target(&a)?
            } else if let Some((lo, hi)) = range(&x, "%", true) {
                if lo > hi {
                    return Err(a.error(
                        DslErrorKind::ReversedPowerRange,
                        "Range low end is above high end (use 'ramp 75-50%' for a descending ramp)",
                    ));
                }
                PowerTarget::Range {
                    lo: Num::Lit(lo),
                    hi: Num::Lit(hi),
                }
            } else if let Some(value) = suffix(&x, "%", true) {
                PowerTarget::PctFtp {
                    value: Num::Lit(value),
                }
            } else if let Some(value) = suffix(&x, "w", false) {
                PowerTarget::Watts {
                    value: Num::Lit(value),
                }
            } else if let Some(bpm) = suffix(&x, "bpm", false) {
                PowerTarget::Hr { bpm }
            } else if x == "free" {
                PowerTarget::Free
            } else if let Some((lo, hi)) =
                range(&x, "rpm", false).or_else(|| suffix(&x, "rpm", false).map(|n| (n, n)))
            {
                if cadence.is_some() {
                    return Err(a.error(DslErrorKind::DuplicateCadence, "Duplicate cadence"));
                }
                if lo > hi {
                    return Err(a.error(
                        DslErrorKind::ReversedCadenceRange,
                        "Cadence range low end is above high end",
                    ));
                }
                cadence = Some(Cadence { lo, hi });
                continue;
            } else if matches!(x.as_str(), "!cap" | "!lower" | "!coach") {
                let StepEnd::HrRecovery { on_unreachable, .. } = &mut end else {
                    return Err(a.error(
                        DslErrorKind::RecoveryFlagNotApplicable,
                        format!("'{}' only applies to hr< recovery steps", a.text),
                    ));
                };
                *on_unreachable = match x.as_str() {
                    "!cap" => OnUnreachable::Cap,
                    "!coach" => OnUnreachable::AskCoach,
                    _ => OnUnreachable::LowerPower,
                };
                continue;
            } else if let Some(pt) = self.pair(&a, false)? {
                // `60%-80%`: refused by `pair` with the spelling to use.
                pt
            } else {
                return Err(a.error(
                    DslErrorKind::UnknownToken,
                    format!("Unknown token '{}'", a.text),
                ));
            };
            if target.is_some() {
                return Err(if x == "ramp" { &target_tok } else { &a }
                    .error(DslErrorKind::DuplicatePowerTarget, "Duplicate power target"));
            }
            if x != "ramp" {
                target_tok = a;
            }
            target = Some(pt);
        }
        let target = target.ok_or_else(|| {
            t.error(
                DslErrorKind::MissingPowerTarget,
                "Missing power target (e.g. 120%, 250w, 50-75%, ramp 50-75%, 140bpm, free)",
            )
        })?;
        let pct = vars::max_pct(&self.resolved(&target), self.ftp);
        if pct > 400.0 {
            return Err(target_tok.error(
                DslErrorKind::ImplausiblePower { pct },
                vars::implausible_message(pct),
            ));
        }
        Ok(StepNode {
            id: (self.ids)(),
            label,
            end,
            target,
            cadence,
        })
    }
}
/// Parse text using an explicit ID source; the crate has no global state.
/// Reuse a caller-owned source across documents for unique IDs, or reset it for
/// reproducible trees. Parsing errors may consume IDs; lexical errors consume none.
/// Error spans use JavaScript UTF-16 offsets. Repeat nesting is limited to
/// [`MAX_NESTING_DEPTH`]; overflowing numeric literals/durations are rejected.
///
/// The declarations come first (ADR-0008 §2.6); every use is checked against its declaration
/// with the use's span, so a parsed document always resolves ([`super::resolve_vars`]).
///
/// # Errors
/// Returns the first lexical/grammatical error or a Rust input-safety bound error.
pub fn parse_dsl(
    text: &str,
    ftp: f64,
    ids: &mut impl FnMut() -> String,
) -> Result<DslDoc, DslError> {
    let mut p = Parser {
        ids,
        tokens: lex(text)?,
        i: 0,
        ftp,
        decls: Vec::new(),
        env: Env::default(),
    };
    let nodes = p.seq(None, 0)?;
    Ok(DslDoc {
        vars: p.decls,
        nodes,
    })
}
// Math.round breaks ties towards positive infinity. Adding 0.5 first loses
// precision immediately below a half-integer (and for large integers).
fn js_round(n: f64) -> f64 {
    if n - n.floor() < 0.5 {
        n.floor()
    } else {
        n.ceil()
    }
}
pub(crate) fn fmt_whole(n: f64) -> String {
    (js_round(n) + 0.0).to_string()
}
pub(crate) fn fmt_num(n: f64) -> String {
    (js_round(n * 1000.0) / 1000.0 + 0.0).to_string()
}
/// Canonical h/m/s duration, rounded to whole seconds and clamped to zero.
#[must_use]
pub fn format_duration(s: f64) -> String {
    let s = js_round(s).max(0.0);
    if s <= 0.0 {
        return "0s".into();
    }
    let mut text = String::new();
    for (n, unit) in [
        (s / 3600.0).floor(),
        ((s % 3600.0) / 60.0).floor(),
        s % 60.0,
    ]
    .into_iter()
    .zip(['h', 'm', 's'])
    .filter(|(n, _)| *n > 0.0)
    {
        text.push_str(&n.to_string());
        text.push(unit);
    }
    text
}
/// A label as quoted text that [`lex`] reads back as the same label: the lexer takes the
/// character after a `\` as itself, and a raw newline ends a label unterminated, so `\`, `"`
/// and the newline are written escaped and every other character as itself.
pub(crate) fn quote_label(label: &str) -> String {
    let mut out = String::with_capacity(label.len() + 2);
    out.push('"');
    for c in label.chars() {
        if matches!(c, '\\' | '"' | '\n') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn num_text(n: &Num, lit: impl Fn(f64) -> String) -> String {
    match n {
        Num::Lit(v) => lit(*v),
        Num::Var { var } => format!("${var}"),
    }
}
/// Two % FTP ends: today's `lo-hi%` for two numbers, else each number with its own `%`.
fn pct_pair(a: &Num, b: &Num) -> String {
    if let (Num::Lit(a), Num::Lit(b)) = (a, b) {
        format!("{}-{}%", fmt_num(*a), fmt_num(*b))
    } else {
        let end = |n: &Num| num_text(n, |v| format!("{}%", fmt_num(v)));
        format!("{}-{}", end(a), end(b))
    }
}
fn print_node(node: &WorkoutNode) -> String {
    let label = match node {
        WorkoutNode::Step(n) => &n.label,
        WorkoutNode::Repeat(n) => &n.label,
    };
    let label = label
        .as_ref()
        .map_or_else(String::new, |s| format!("{} ", quote_label(s)));
    match node {
        WorkoutNode::Repeat(n) => {
            let mut opts = Vec::new();
            if let Some(p) = &n.progression {
                for (value, unit) in [(&p.watts_per_rep, 'w'), (&p.pct_per_rep, '%')] {
                    match value {
                        Some(Num::Lit(v)) => opts.push(format!(
                            "{}{}{unit}",
                            if *v >= 0.0 { "+" } else { "" },
                            fmt_num(*v)
                        )),
                        Some(Num::Var { var }) => opts.push(format!("+${var}")),
                        None => {}
                    }
                }
            }
            if n.skip_last_rest == Some(true) {
                opts.push("!last".into());
            }
            let options = if opts.is_empty() {
                String::new()
            } else {
                format!("[{}]", opts.join(", "))
            };
            let count = match &n.count {
                Num::Lit(c) => format!("{c}x"),
                Num::Var { var } => format!("${var} x"),
            };
            format!(
                "{label}{count}{options}({})",
                n.children
                    .iter()
                    .map(print_node)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        WorkoutNode::Step(n) => {
            let mut flag = "";
            let end = match &n.end {
                StepEnd::Time { s } => num_text(s, format_duration),
                StepEnd::Kj { kj } => format!("{}kj", fmt_num(*kj)),
                StepEnd::Lap => "lap".into(),
                StepEnd::HrRecovery {
                    target_bpm,
                    min_s,
                    max_s,
                    on_unreachable,
                } => {
                    flag = match on_unreachable {
                        OnUnreachable::Cap => " !cap",
                        OnUnreachable::AskCoach => " !coach",
                        OnUnreachable::LowerPower => "",
                    };
                    format!(
                        "hr<{} {}..{}",
                        fmt_whole(*target_bpm),
                        num_text(min_s, format_duration),
                        num_text(max_s, format_duration)
                    )
                }
            };
            let target = match &n.target {
                PowerTarget::PctFtp { value } => num_text(value, |v| format!("{}%", fmt_num(v))),
                PowerTarget::Watts { value } => num_text(value, |v| format!("{}w", fmt_whole(v))),
                PowerTarget::Range { lo, hi } => pct_pair(lo, hi),
                PowerTarget::Ramp { from, to } => format!("ramp {}", pct_pair(from, to)),
                PowerTarget::Hr { bpm } => format!("{}bpm", fmt_whole(*bpm)),
                PowerTarget::Free => "free".into(),
            };
            let cad = n.cadence.as_ref().map_or_else(String::new, |c| {
                if c.lo.partial_cmp(&c.hi) == Some(std::cmp::Ordering::Equal) {
                    format!(" {}rpm", c.lo + 0.0)
                } else {
                    format!(" {}-{}rpm", c.lo + 0.0, c.hi + 0.0)
                }
            });
            format!("{label}{end} {target}{cad}{flag}")
        }
    }
}
/// Canonical DSL: the declarations first, one per line (`$name = expr`, ADR-0008 §2.11), a
/// blank line, then one top-level item per line, nested items comma-separated. A workout
/// without declarations prints exactly as before variables existed.
/// Numeric values are rounded as in the TS printer; round trips require DSL-representable trees.
/// Canonical numeric parity covers finite rounded magnitudes below 1e21; see `docs/DSL.md`.
#[must_use]
pub fn print_dsl(vars: &[VarDecl], nodes: &[WorkoutNode]) -> String {
    let decls = vars
        .iter()
        .map(|d| format!("${} = {}", d.name, vars::print_expr(&d.expr)))
        .collect::<Vec<_>>()
        .join("\n");
    let blocks = nodes.iter().map(print_node).collect::<Vec<_>>().join("\n");
    match (decls.is_empty(), blocks.is_empty()) {
        (true, _) => blocks,
        (false, true) => decls,
        (false, false) => format!("{decls}\n\n{blocks}"),
    }
}
