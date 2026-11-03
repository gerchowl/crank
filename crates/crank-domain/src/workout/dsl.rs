use super::{
    Cadence, DslError, DslErrorKind, OnUnreachable, PowerTarget, Progression, RepeatNode, StepEnd,
    StepNode, WorkoutNode,
};
/// Maximum number of enclosing repeats accepted from untrusted text.
pub const MAX_NESTING_DEPTH: usize = 32;
#[derive(Clone)]
struct Token {
    kind: char,
    text: String,
    offset: usize,
    length: usize,
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
fn lex(src: &str) -> Result<Vec<Token>, DslError> {
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
fn number(s: &str, decimal: bool) -> Option<f64> {
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
fn suffix(s: &str, unit: &str, decimal: bool) -> Option<f64> {
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
    fn seq(&mut self, open: Option<&Token>, depth: usize) -> Result<Vec<WorkoutNode>, DslError> {
        let mut nodes = Vec::new();
        loop {
            while self.peek().kind == ',' {
                self.next();
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
        let label = (self.peek().kind == 's').then(|| self.next().text);
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
        if let Some(count) = suffix(&t.text.to_ascii_lowercase(), "x", false) {
            self.repeat(count, &t, label, depth)
                .map(WorkoutNode::Repeat)
        } else {
            self.step(&t, label).map(WorkoutNode::Step)
        }
    }
    fn repeat(
        &mut self,
        count: f64,
        t: &Token,
        label: Option<String>,
        depth: usize,
    ) -> Result<RepeatNode, DslError> {
        if depth == MAX_NESTING_DEPTH {
            return Err(t.error(
                DslErrorKind::NestingTooDeep {
                    max: MAX_NESTING_DEPTH,
                },
                format!("Repeat nesting exceeds {MAX_NESTING_DEPTH} levels"),
            ));
        }
        if count < 1.0 {
            return Err(t.error(
                DslErrorKind::InvalidRepeatCount { count },
                "Repeat count must be ≥ 1",
            ));
        }
        let mut node = RepeatNode {
            id: (self.ids)(),
            label,
            count,
            children: Vec::new(),
            progression: None,
            skip_last_rest: None,
        };
        if self.peek().kind == '[' {
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
                    prog.watts_per_rep = Some(w);
                } else if let Some(p) = signed("%") {
                    prog.pct_per_rep = Some(p);
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
        }
        let open = self.next();
        if open.kind != '(' {
            return Err(open.error(
                DslErrorKind::MissingRepeatBody,
                format!("Expected '(' after '{}'", t.text),
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
    fn end(&mut self, t: &Token) -> Result<StepEnd, DslError> {
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
                Ok(StepEnd::Time { s })
            };
        }
        if let Some(kj) = suffix(&w, "kj", true) {
            return Ok(StepEnd::Kj { kj });
        }
        if w == "lap" {
            return Ok(StepEnd::Lap);
        }
        if let Some(target_bpm) = w.strip_prefix("hr<").and_then(|s| number(s, false)) {
            let rt = self.peek();
            let bounds = rt
                .text
                .split_once("..")
                .and_then(|(lo, hi)| Some((parse_duration_word(lo)?, parse_duration_word(hi)?)));
            let Some((min_s, max_s)) = bounds.filter(|_| rt.kind == 'w') else {
                return Err(if rt.kind == 'w' { rt } else { t }.error(
                    DslErrorKind::MissingRecoveryBounds,
                    format!("Expected min..max duration after '{}', e.g. 2m..4m", t.text),
                ));
            };
            let rt = self.next();
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
                min_s,
                max_s,
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
    fn step(&mut self, t: &Token, label: Option<String>) -> Result<StepNode, DslError> {
        let mut end = self.end(t)?;
        let mut target = None;
        let mut cadence = None;
        let mut target_tok = t.clone();
        while self.peek().kind == 'w' {
            let a = self.next();
            let x = a.text.to_ascii_lowercase();
            let pt = if x == "ramp" {
                let r = self.next();
                let Some((from, to)) = range(&r.text, "%", true).filter(|_| r.kind == 'w') else {
                    return Err(if r.kind == 'e' { &a } else { &r }.error(
                        DslErrorKind::MissingRampRange,
                        "Expected a range after 'ramp', e.g. ramp 50-75%",
                    ));
                };
                target_tok = r;
                PowerTarget::Ramp { from, to }
            } else if let Some((lo, hi)) = range(&x, "%", true) {
                if lo > hi {
                    return Err(a.error(
                        DslErrorKind::ReversedPowerRange,
                        "Range low end is above high end (use 'ramp 75-50%' for a descending ramp)",
                    ));
                }
                PowerTarget::Range { lo, hi }
            } else if let Some(value) = suffix(&x, "%", true) {
                PowerTarget::PctFtp { value }
            } else if let Some(value) = suffix(&x, "w", false) {
                PowerTarget::Watts { value }
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
        let pct = max_pct(&target, self.ftp);
        if pct > 400.0 {
            return Err(target_tok.error(
                DslErrorKind::ImplausiblePower { pct },
                format!("{} % FTP is implausible (max 400 %)", fmt_whole(pct)),
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
/// # Errors
/// Returns the first lexical/grammatical error or a Rust input-safety bound error.
pub fn parse_dsl(
    text: &str,
    ftp: f64,
    ids: &mut impl FnMut() -> String,
) -> Result<Vec<WorkoutNode>, DslError> {
    Parser {
        ids,
        tokens: lex(text)?,
        i: 0,
        ftp,
    }
    .seq(None, 0)
}
fn max_pct(target: &PowerTarget, ftp: f64) -> f64 {
    match *target {
        PowerTarget::PctFtp { value } => value,
        PowerTarget::Watts { value } if ftp > 0.0 => value / ftp * 100.0,
        PowerTarget::Range { hi, .. } => hi,
        PowerTarget::Ramp { from, to } => from.max(to),
        _ => 0.0,
    }
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
fn fmt_whole(n: f64) -> String {
    (js_round(n) + 0.0).to_string()
}
fn fmt_num(n: f64) -> String {
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
fn print_node(node: &WorkoutNode) -> String {
    let label = match node {
        WorkoutNode::Step(n) => &n.label,
        WorkoutNode::Repeat(n) => &n.label,
    };
    let label = label.as_ref().map_or_else(String::new, |s| {
        format!("\"{}\" ", s.replace('\\', "\\\\").replace('"', "\\\""))
    });
    match node {
        WorkoutNode::Repeat(n) => {
            let mut opts = Vec::new();
            if let Some(p) = &n.progression {
                for (value, unit) in [(p.watts_per_rep, 'w'), (p.pct_per_rep, '%')] {
                    if let Some(v) = value {
                        opts.push(format!(
                            "{}{}{unit}",
                            if v >= 0.0 { "+" } else { "" },
                            fmt_num(v)
                        ));
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
            format!(
                "{label}{}x{options}({})",
                n.count,
                n.children
                    .iter()
                    .map(print_node)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        WorkoutNode::Step(n) => {
            let mut flag = "";
            let end = match n.end {
                StepEnd::Time { s } => format_duration(s),
                StepEnd::Kj { kj } => format!("{}kj", fmt_num(kj)),
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
                        fmt_whole(target_bpm),
                        format_duration(min_s),
                        format_duration(max_s)
                    )
                }
            };
            let target = match n.target {
                PowerTarget::PctFtp { value } => format!("{}%", fmt_num(value)),
                PowerTarget::Watts { value } => format!("{}w", fmt_whole(value)),
                PowerTarget::Range { lo, hi } => format!("{}-{}%", fmt_num(lo), fmt_num(hi)),
                PowerTarget::Ramp { from, to } => {
                    format!("ramp {}-{}%", fmt_num(from), fmt_num(to))
                }
                PowerTarget::Hr { bpm } => format!("{}bpm", fmt_whole(bpm)),
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
/// Canonical DSL: one top-level item per line, nested items comma-separated.
/// Numeric values are rounded as in the TS printer; round trips require DSL-representable trees.
/// Canonical numeric parity covers finite rounded magnitudes below 1e21; see `docs/DSL.md`.
#[must_use]
pub fn print_dsl(nodes: &[WorkoutNode]) -> String {
    nodes.iter().map(print_node).collect::<Vec<_>>().join("\n")
}
