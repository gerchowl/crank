//! Grade-profile text (ADR-0005 §2.2.5): the workout DSL's lexer, labels, separators, comments,
//! repeats and canonical printing, with a distance end and a grade target.
use super::{MAX_ROUTE_M, Route, RoutePoint, RouteSegment, RouteSource, SegmentGrade, Surface};
use crate::workout::{
    DslErrorKind, MAX_NESTING_DEPTH,
    dsl::{Token, lex, number, quote_label, suffix},
    parse_duration_word,
};
use serde::{Deserialize, Serialize};

/// Shortest grade-profile segment, metres (ADR-0005 §2.2.5).
pub const MIN_SEGMENT_M: u32 = 10;
/// Longest grade-profile segment, metres (ADR-0005 §2.2.5).
pub const MAX_SEGMENT_M: u32 = 100_000;
/// Steepest descent a grade-profile segment may state, percent (ADR-0005 §2.2.5).
pub const MIN_GRADE_PCT: f64 = -25.0;
/// Steepest climb a grade-profile segment may state, percent (ADR-0005 §2.2.5).
pub const MAX_GRADE_PCT: f64 = 30.0;
/// The name of a grade profile without a non-blank label.
pub const UNLABELLED_NAME: &str = "Grade profile";
/// Longest grade-profile text, UTF-8 bytes, for the input and for its canonical form
/// ([`print_grade_profile`]), so every accepted profile prints to text that parses again
/// (ADR-0005 §2.2.4): a 1 000 km profile written out without repeats at 100 m per segment is
/// 10 000 segments of about 15 bytes, 150 KB.
pub const MAX_PROFILE_TEXT_BYTES: usize = 256 * 1024;
/// Longest label, characters (ADR-0005 §2.2.4): a climb's or a stretch's name for the ride
/// screen, where a few words fit.
pub const MAX_LABEL_CHARS: usize = 100;
/// Most segments a grade profile expands to: 1 000 km of the shortest (10 m) segments. The
/// 1 000 km total, checked while parsing, enforces it before anything is expanded.
pub const MAX_PROFILE_SEGMENTS: u64 = 100_000;
/// Most label bytes the expanded route may hold, counting a repeated label once per segment it
/// names (ADR-0005 §2.2.4): checked before expanding, so `100000x("…" 0.01km 0%)` cannot
/// allocate gigabytes. 10 000 labelled segments of 100 bytes fit.
pub const MAX_EXPANDED_LABEL_BYTES: u64 = 1024 * 1024;

/// One item of a grade profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GradeProfileItem {
    /// A distance at a grade.
    Segment(GradeProfileSegment),
    /// Items repeated a number of times.
    Repeat(GradeProfileRepeat),
}

/// A distance at a grade, `DIST grade [surface]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GradeProfileSegment {
    /// Its label, if any.
    pub label: Option<String>,
    /// Length, whole metres ([`MIN_SEGMENT_M`]..=[`MAX_SEGMENT_M`]; the text's resolution is 0.001 km).
    pub length_m: u32,
    /// Its grade.
    pub grade: ProfileGrade,
    /// Its surface, if stated.
    pub surface: Option<Surface>,
}

/// `COUNT x (profile)`; a route repeat takes no progression options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct GradeProfileRepeat {
    /// Its label, if any.
    pub label: Option<String>,
    /// Times the children are ridden, at least 1.
    pub count: u32,
    /// The repeated items, at least one.
    pub children: Vec<GradeProfileItem>,
}

/// A grade-profile segment's grade, percent, each within [`MIN_GRADE_PCT`]..=[`MAX_GRADE_PCT`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProfileGrade {
    /// `6.5%`.
    Constant {
        /// Grade, percent.
        pct: f64,
    },
    /// `ramp 6.5%..9%`: linear over the segment's distance.
    Ramp {
        /// Grade at the start, percent.
        from_pct: f64,
        /// Grade at the end, percent.
        to_pct: f64,
    },
}

/// Stable class of a grade-profile parse failure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum RouteDslErrorKind {
    /// A quoted label has no closing quote.
    UnterminatedLabel,
    /// A number exceeds the finite range.
    NumericOverflow,
    /// A token that cannot start or continue an item.
    UnexpectedToken,
    /// Two items without a separator between them.
    MissingSeparator,
    /// A `(` without its `)`.
    UnclosedRepeat,
    /// A label at the end of the text.
    MissingItemAfterLabel,
    /// Repeats nested deeper than [`MAX_NESTING_DEPTH`].
    NestingTooDeep {
        /// Maximum number of enclosing repeats.
        max: usize,
    },
    /// A repeat count below 1.
    InvalidRepeatCount,
    /// `[...]` after a repeat count: routes have no progression options.
    RepeatOptions,
    /// A repeat count without `(`.
    MissingRepeatBody,
    /// `N x()`.
    EmptyRepeat,
    /// A distance in metres: `m` is minutes in a workout.
    MetresNotKm,
    /// A workout duration or step end where a distance belongs.
    WorkoutStepEnd,
    /// A number without the `km` unit.
    MissingUnit,
    /// Not a distance.
    InvalidDistance,
    /// A distance finer than 0.001 km.
    DistanceTooPrecise,
    /// A segment outside [`MIN_SEGMENT_M`]..=[`MAX_SEGMENT_M`].
    SegmentLength,
    /// A workout power target where a grade belongs.
    WorkoutTarget,
    /// A segment without a grade.
    MissingGrade,
    /// `ramp` without `from%..to%`.
    MissingRampRange,
    /// A grade outside [`MIN_GRADE_PCT`]..=[`MAX_GRADE_PCT`].
    ImplausibleGrade {
        /// The grade, percent.
        pct: f64,
    },
    /// Two surfaces on one segment.
    DuplicateSurface,
    /// A word that is no surface class.
    UnknownToken,
    /// More than [`MAX_ROUTE_M`] in all.
    RouteTooLong,
    /// No segment at all.
    Empty,
    /// The text is longer than [`MAX_PROFILE_TEXT_BYTES`].
    TextTooLong,
    /// The text's canonical form ([`print_grade_profile`]) would be longer than
    /// [`MAX_PROFILE_TEXT_BYTES`], so it could not be parsed back.
    CanonicalTooLong,
    /// A label longer than [`MAX_LABEL_CHARS`].
    LabelTooLong,
    /// The expanded route's labels exceed [`MAX_EXPANDED_LABEL_BYTES`].
    LabelsTooLarge,
    /// The parser built a route that breaks an ADR-0005 §2.1 invariant: a bug, never a property
    /// of the text.
    Invariant,
}

/// Grade-profile parse failure; the span is in UTF-16 code units, as in the workout DSL.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct RouteDslError {
    /// Stable failure class.
    pub kind: RouteDslErrorKind,
    /// Human-readable message.
    pub message: String,
    /// UTF-16 offset.
    pub offset: usize,
    /// UTF-16 length; zero selects the end of the input.
    pub length: usize,
}
impl std::fmt::Display for RouteDslError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} @{}", self.message, self.offset)
    }
}
impl std::error::Error for RouteDslError {}

fn err(t: &Token, kind: RouteDslErrorKind, message: impl Into<String>) -> RouteDslError {
    RouteDslError {
        kind,
        message: message.into(),
        offset: t.offset,
        length: t.length,
    }
}

const MAX_ROUTE_WHOLE_M: u64 = 1_000_000;
const _: () = assert!(MAX_ROUTE_M == 1_000_000.0);

/// `+`/`-` then a decimal number.
fn signed(s: &str) -> Option<f64> {
    let (sign, body) = match s.as_bytes().first() {
        Some(b'-') => (-1.0, &s[1..]),
        Some(b'+') => (1.0, &s[1..]),
        _ => (1.0, s),
    };
    number(body, true).map(|n| sign * n)
}

/// Whole metres of a `km` number with at most three decimals; `Err(true)` if finer, `Err(false)`
/// if longer than any segment can be.
fn metres(km: &str) -> Result<u64, bool> {
    let (int, frac) = km.split_once('.').unwrap_or((km, ""));
    if frac.len() > 3 {
        return Err(true);
    }
    let int = int.trim_start_matches('0');
    if int.len() > 6 {
        return Err(false);
    }
    let int: u64 = if int.is_empty() {
        0
    } else {
        int.parse().map_err(|_| false)?
    };
    let frac: u64 = format!("{frac:0<3}").parse().map_err(|_| false)?;
    Ok(int * 1000 + frac)
}

fn length_m(item: &GradeProfileItem) -> u64 {
    match item {
        GradeProfileItem::Segment(s) => u64::from(s.length_m),
        GradeProfileItem::Repeat(r) => r
            .children
            .iter()
            .map(length_m)
            .fold(0, u64::saturating_add)
            .saturating_mul(u64::from(r.count)),
    }
}

struct Parser {
    tokens: Vec<Token>,
    i: usize,
}

impl Parser {
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

    /// The span from `start` to the end of the last consumed token.
    fn span(&self, start: &Token) -> (usize, usize) {
        let end = self.tokens[..self.i]
            .last()
            .map_or(start.offset, |t| t.offset + t.length);
        (start.offset, end.saturating_sub(start.offset))
    }

    fn seq(
        &mut self,
        open: Option<&Token>,
        depth: usize,
    ) -> Result<Vec<GradeProfileItem>, RouteDslError> {
        let mut items = Vec::new();
        let mut total: u64 = 0;
        loop {
            while self.peek().kind == ',' {
                self.next();
            }
            let t = self.peek();
            match t.kind {
                'e' => {
                    return open.map_or(Ok(items), |o| {
                        Err(err(
                            o,
                            RouteDslErrorKind::UnclosedRepeat,
                            "Unclosed '(' — missing ')'",
                        ))
                    });
                }
                ')' => {
                    return if open.is_some() {
                        Ok(items)
                    } else {
                        Err(err(t, RouteDslErrorKind::UnexpectedToken, "Unexpected ')'"))
                    };
                }
                _ => {
                    let start = t.clone();
                    let item = self.item(depth)?;
                    total = total.saturating_add(length_m(&item));
                    if total > MAX_ROUTE_WHOLE_M {
                        let (offset, length) = self.span(&start);
                        return Err(RouteDslError {
                            kind: RouteDslErrorKind::RouteTooLong,
                            message: format!(
                                "A route is at most {} km in all",
                                MAX_ROUTE_WHOLE_M / 1000
                            ),
                            offset,
                            length,
                        });
                    }
                    items.push(item);
                }
            }
            let after = self.peek();
            if !matches!(after.kind, ',' | 'e' | ')') {
                return Err(err(
                    after,
                    RouteDslErrorKind::MissingSeparator,
                    format!(
                        "Unexpected '{}' — separate items with ',', ';' or a newline",
                        after.text
                    ),
                ));
            }
        }
    }

    fn item(&mut self, depth: usize) -> Result<GradeProfileItem, RouteDslError> {
        let label = if self.peek().kind == 's' {
            let l = self.next();
            if l.text.chars().count() > MAX_LABEL_CHARS {
                return Err(err(
                    &l,
                    RouteDslErrorKind::LabelTooLong,
                    format!("A label is at most {MAX_LABEL_CHARS} characters"),
                ));
            }
            Some(l.text)
        } else {
            None
        };
        let t = self.next();
        if t.kind != 'w' {
            return Err(if t.kind == 'e' {
                err(
                    &t,
                    RouteDslErrorKind::MissingItemAfterLabel,
                    "Expected a segment or repeat after the label",
                )
            } else {
                err(
                    &t,
                    RouteDslErrorKind::UnexpectedToken,
                    format!("Unexpected '{}'", t.text),
                )
            });
        }
        let lower = t.text.to_ascii_lowercase();
        if let Some(count) = lower
            .strip_suffix('x')
            .filter(|n| number(n, false).is_some())
        {
            self.repeat(count, &t, label, depth)
                .map(GradeProfileItem::Repeat)
        } else {
            self.segment(&t, label).map(GradeProfileItem::Segment)
        }
    }

    fn repeat(
        &mut self,
        count_text: &str,
        t: &Token,
        label: Option<String>,
        depth: usize,
    ) -> Result<GradeProfileRepeat, RouteDslError> {
        if depth == MAX_NESTING_DEPTH {
            return Err(err(
                t,
                RouteDslErrorKind::NestingTooDeep {
                    max: MAX_NESTING_DEPTH,
                },
                format!("Repeat nesting exceeds {MAX_NESTING_DEPTH} levels"),
            ));
        }
        if count_text.bytes().all(|b| b == b'0') {
            return Err(err(
                t,
                RouteDslErrorKind::InvalidRepeatCount,
                "Repeat count must be ≥ 1",
            ));
        }
        // Every segment is at least MIN_SEGMENT_M, so a larger count can never fit MAX_ROUTE_M.
        let max_count = MAX_PROFILE_SEGMENTS;
        let Some(count) = count_text
            .parse::<u64>()
            .ok()
            .filter(|n| *n <= max_count)
            .and_then(|n| u32::try_from(n).ok())
        else {
            return Err(err(
                t,
                RouteDslErrorKind::RouteTooLong,
                format!("A route is at most {} km in all", MAX_ROUTE_WHOLE_M / 1000),
            ));
        };
        if self.peek().kind == '[' {
            return Err(err(
                self.peek(),
                RouteDslErrorKind::RepeatOptions,
                "A route repeat takes no options ('[...]' is a workout progression)",
            ));
        }
        let open = self.next();
        if open.kind != '(' {
            return Err(err(
                &open,
                RouteDslErrorKind::MissingRepeatBody,
                format!("Expected '(' after '{}'", t.text),
            ));
        }
        let children = self.seq(Some(&open), depth + 1)?;
        let close = self.next();
        if children.is_empty() {
            return Err(RouteDslError {
                kind: RouteDslErrorKind::EmptyRepeat,
                message: "Empty repeat".into(),
                offset: t.offset,
                length: close.offset + 1 - t.offset,
            });
        }
        Ok(GradeProfileRepeat {
            label,
            count,
            children,
        })
    }

    fn distance(t: &Token) -> Result<u32, RouteDslError> {
        let w = t.text.to_ascii_lowercase();
        if let Some(km) = w.strip_suffix("km").filter(|km| number(km, true).is_some()) {
            if number(km, true).is_some_and(|n| !n.is_finite()) {
                return Err(err(
                    t,
                    RouteDslErrorKind::NumericOverflow,
                    "Number exceeds finite numeric range",
                ));
            }
            let m = metres(km).map_err(|too_precise| {
                if too_precise {
                    err(
                        t,
                        RouteDslErrorKind::DistanceTooPrecise,
                        "Distances are in whole metres: at most 3 decimals of km",
                    )
                } else {
                    segment_length(t)
                }
            })?;
            return u32::try_from(m)
                .ok()
                .filter(|m| (MIN_SEGMENT_M..=MAX_SEGMENT_M).contains(m))
                .ok_or_else(|| segment_length(t));
        }
        if let Some(n) = w.strip_suffix('m').and_then(|n| number(n, true)) {
            return Err(err(
                t,
                RouteDslErrorKind::MetresNotKm,
                format!(
                    "routes are measured in km: write {}km ('m' is minutes in a workout)",
                    n / 1000.0 + 0.0
                ),
            ));
        }
        let step_end = parse_duration_word(&w).is_some()
            || suffix(&w, "kj", true).is_some()
            || w == "lap"
            || w.starts_with("hr<");
        if step_end {
            return Err(err(
                t,
                RouteDslErrorKind::WorkoutStepEnd,
                format!(
                    "'{}' ends a workout step: a route segment ends at a distance in km, e.g. 2km",
                    t.text
                ),
            ));
        }
        if number(&w, true).is_some() {
            return Err(err(
                t,
                RouteDslErrorKind::MissingUnit,
                format!(
                    "'{}' needs a unit: routes are measured in km (2km) — or '{}x(...)' for a repeat",
                    t.text, t.text
                ),
            ));
        }
        Err(err(
            t,
            RouteDslErrorKind::InvalidDistance,
            format!(
                "Expected a distance in km (2km, 0.4km) or a repeat (3x(...)); got '{}'",
                t.text
            ),
        ))
    }

    fn grade_value(t: &Token, text: &str) -> Result<Option<f64>, RouteDslError> {
        let Some(pct) = text.strip_suffix('%').and_then(signed) else {
            return Ok(None);
        };
        if !pct.is_finite() {
            return Err(err(
                t,
                RouteDslErrorKind::NumericOverflow,
                "Number exceeds finite numeric range",
            ));
        }
        if !(MIN_GRADE_PCT..=MAX_GRADE_PCT).contains(&pct) {
            return Err(err(
                t,
                RouteDslErrorKind::ImplausibleGrade { pct },
                format!(
                    "{} % is implausible for a road ({MIN_GRADE_PCT} % to +{MAX_GRADE_PCT} %)",
                    pct + 0.0
                ),
            ));
        }
        Ok(Some(pct + 0.0))
    }

    fn grade(&mut self, end: &Token) -> Result<ProfileGrade, RouteDslError> {
        let missing = |t: &Token| {
            err(
                t,
                RouteDslErrorKind::MissingGrade,
                "Missing grade (e.g. 6%, -3%, ramp 2%..6%)",
            )
        };
        if self.peek().kind != 'w' {
            return Err(missing(end));
        }
        let a = self.next();
        let x = a.text.to_ascii_lowercase();
        if x == "ramp" {
            let r = self.next();
            let range = (r.kind == 'w').then(|| r.text.to_ascii_lowercase());
            let Some((from, to)) = range.as_deref().and_then(|s| s.split_once("..")) else {
                return Err(err(
                    if r.kind == 'e' { &a } else { &r },
                    RouteDslErrorKind::MissingRampRange,
                    "Expected a range after 'ramp', e.g. ramp 2%..6%",
                ));
            };
            let (Some(from_pct), Some(to_pct)) =
                (Self::grade_value(&r, from)?, Self::grade_value(&r, to)?)
            else {
                return Err(err(
                    &r,
                    RouteDslErrorKind::MissingRampRange,
                    "Expected a range after 'ramp', e.g. ramp 2%..6%",
                ));
            };
            return Ok(ProfileGrade::Ramp { from_pct, to_pct });
        }
        if let Some(pct) = Self::grade_value(&a, &x)? {
            return Ok(ProfileGrade::Constant { pct });
        }
        let workout_target = x == "free"
            || suffix(&x, "w", false).is_some()
            || suffix(&x, "bpm", false).is_some()
            || x.strip_suffix('%')
                .and_then(|r| r.split_once('-'))
                .is_some_and(|(lo, hi)| number(lo, true).is_some() && number(hi, true).is_some());
        if workout_target {
            return Err(err(
                &a,
                RouteDslErrorKind::WorkoutTarget,
                format!(
                    "'{}' is a workout power target: a route segment's target is a grade, e.g. 6% or ramp 2%..6%",
                    a.text
                ),
            ));
        }
        Err(missing(&a))
    }

    fn segment(
        &mut self,
        t: &Token,
        label: Option<String>,
    ) -> Result<GradeProfileSegment, RouteDslError> {
        let length_m = Self::distance(t)?;
        let grade = self.grade(t)?;
        let mut surface = None;
        while self.peek().kind == 'w' {
            let a = self.next();
            let x = a.text.to_ascii_lowercase();
            let Some(s) = Surface::ALL.into_iter().find(|s| s.name() == x) else {
                return Err(err(
                    &a,
                    RouteDslErrorKind::UnknownToken,
                    format!(
                        "Unknown token '{}' (a surface is one of {})",
                        a.text,
                        Surface::ALL.map(Surface::name).join(", ")
                    ),
                ));
            };
            if surface.is_some() {
                return Err(err(
                    &a,
                    RouteDslErrorKind::DuplicateSurface,
                    "Duplicate surface",
                ));
            }
            surface = Some(s);
        }
        Ok(GradeProfileSegment {
            label,
            length_m,
            grade,
            surface,
        })
    }
}

fn segment_length(t: &Token) -> RouteDslError {
    err(
        t,
        RouteDslErrorKind::SegmentLength,
        format!(
            "A segment is {} km to {} km",
            f64::from(MIN_SEGMENT_M) / 1000.0,
            MAX_SEGMENT_M / 1000
        ),
    )
}

/// Parse grade-profile text into its items (ADR-0005 §2.2.5). Empty text is an empty profile.
///
/// # Errors
/// The first lexical or grammatical error, a workout token where a distance or grade belongs
/// (`500m` names the km form), or a limit: segment length, grade range, nesting, 1 000 km in all,
/// a label over [`MAX_LABEL_CHARS`], or the text or its canonical form over [`MAX_PROFILE_TEXT_BYTES`].
pub fn parse_grade_profile(text: &str) -> Result<Vec<GradeProfileItem>, RouteDslError> {
    if text.len() > MAX_PROFILE_TEXT_BYTES {
        return Err(RouteDslError {
            kind: RouteDslErrorKind::TextTooLong,
            message: format!(
                "A grade profile is at most {} KiB of text",
                MAX_PROFILE_TEXT_BYTES / 1024
            ),
            offset: 0,
            length: 0,
        });
    }
    let tokens = lex(text).map_err(|e| RouteDslError {
        kind: match e.kind {
            DslErrorKind::UnterminatedLabel => RouteDslErrorKind::UnterminatedLabel,
            // The lexer reports no other kind than these two.
            _ => RouteDslErrorKind::NumericOverflow,
        },
        message: e.message,
        offset: e.offset,
        length: e.length,
    })?;
    let items = Parser { tokens, i: 0 }.seq(None, 0)?;
    // Every accepted profile must print to text that parses again (ADR-0005 §2.1 round trip):
    // the canonical form can be longer than the input (`,` becomes `, `), so bound it too.
    let canonical = print_grade_profile(&items).len();
    if canonical > MAX_PROFILE_TEXT_BYTES {
        return Err(RouteDslError {
            kind: RouteDslErrorKind::CanonicalTooLong,
            message: format!(
                "Written out canonically this profile is {canonical} bytes, more than {} KiB",
                MAX_PROFILE_TEXT_BYTES / 1024
            ),
            offset: 0,
            length: text.encode_utf16().count(),
        });
    }
    Ok(items)
}

fn print_label(label: Option<&String>) -> String {
    label.map_or_else(String::new, |s| format!("{} ", quote_label(s)))
}

fn print_km(m: u32) -> String {
    let frac = format!("{:03}", m % 1000);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        format!("{}km", m / 1000)
    } else {
        format!("{}.{frac}km", m / 1000)
    }
}

fn print_pct(pct: f64) -> String {
    format!("{}%", pct + 0.0)
}

fn print_item(item: &GradeProfileItem) -> String {
    match item {
        GradeProfileItem::Segment(s) => {
            let grade = match s.grade {
                ProfileGrade::Constant { pct } => print_pct(pct),
                ProfileGrade::Ramp { from_pct, to_pct } => {
                    format!("ramp {}..{}", print_pct(from_pct), print_pct(to_pct))
                }
            };
            let surface = s
                .surface
                .map_or_else(String::new, |s| format!(" {}", s.name()));
            format!(
                "{}{} {grade}{surface}",
                print_label(s.label.as_ref()),
                print_km(s.length_m)
            )
        }
        GradeProfileItem::Repeat(r) => format!(
            "{}{}x({})",
            print_label(r.label.as_ref()),
            r.count,
            r.children
                .iter()
                .map(print_item)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Canonical text: one top-level item per line, nested items comma-separated, lower case,
/// distances in km with at most three decimals and grades in their shortest exact form, so
/// parsing it gives the same items and printing those gives the same bytes.
#[must_use]
pub fn print_grade_profile(items: &[GradeProfileItem]) -> String {
    items.iter().map(print_item).collect::<Vec<_>>().join("\n")
}

struct Flat<'a> {
    label: Option<&'a String>,
    length_m: u32,
    grade: ProfileGrade,
    surface: Option<Surface>,
}

/// Segments in riding order; each takes its own label, else its innermost labelled repeat's.
fn flatten<'a>(
    items: &'a [GradeProfileItem],
    inherited: Option<&'a String>,
    out: &mut Vec<Flat<'a>>,
) {
    for item in items {
        match item {
            GradeProfileItem::Segment(s) => out.push(Flat {
                label: s.label.as_ref().or(inherited),
                length_m: s.length_m,
                grade: s.grade,
                surface: s.surface,
            }),
            GradeProfileItem::Repeat(r) => {
                for _ in 0..r.count {
                    flatten(&r.children, r.label.as_ref().or(inherited), out);
                }
            }
        }
    }
}

fn first_label(items: &[GradeProfileItem]) -> Option<&String> {
    fn usable(l: Option<&String>) -> Option<&String> {
        l.filter(|l| !l.trim().is_empty())
    }
    items.iter().find_map(|item| match item {
        GradeProfileItem::Segment(s) => usable(s.label.as_ref()),
        GradeProfileItem::Repeat(r) => {
            usable(r.label.as_ref()).or_else(|| first_label(&r.children))
        }
    })
}

/// Label bytes of the expanded segments, as [`flatten`] would copy them, without expanding.
fn expanded_label_bytes(items: &[GradeProfileItem], inherited: u64) -> u64 {
    items
        .iter()
        .map(|item| match item {
            GradeProfileItem::Segment(s) => s
                .label
                .as_ref()
                .map_or(inherited, |l| u64::try_from(l.len()).unwrap_or(u64::MAX)),
            GradeProfileItem::Repeat(r) => {
                let label = r
                    .label
                    .as_ref()
                    .map_or(inherited, |l| u64::try_from(l.len()).unwrap_or(u64::MAX));
                expanded_label_bytes(&r.children, label).saturating_mul(u64::from(r.count))
            }
        })
        .fold(0, u64::saturating_add)
}

/// Parse grade-profile text into a route (ADR-0005 §2.2.5): points at 0 and at every segment
/// end, elevation the integral of the grade from 0 m ([`ElevationKind::Exact`](super::ElevationKind::Exact)), segments
/// carrying their grades, and the canonical text as [`RouteSource::GradeProfile`]. The name is
/// the first label, else [`UNLABELLED_NAME`]. That text is the route's one source of truth
/// ([`Route::check`] reparses it), so `parse_route_dsl(print_route_dsl(r))` equals `r`.
///
/// # Errors
/// Every error of [`parse_grade_profile`], and [`RouteDslErrorKind::Empty`] for a profile
/// without a segment.
pub fn parse_route_dsl(text: &str) -> Result<Route, RouteDslError> {
    let items = parse_grade_profile(text)?;
    if items.is_empty() {
        return Err(RouteDslError {
            kind: RouteDslErrorKind::Empty,
            message: "A grade profile needs at least one segment, e.g. 2km 3%".into(),
            offset: 0,
            length: 0,
        });
    }
    if expanded_label_bytes(&items, 0) > MAX_EXPANDED_LABEL_BYTES {
        return Err(RouteDslError {
            kind: RouteDslErrorKind::LabelsTooLarge,
            message: format!(
                "Repeated labels would take more than {} MiB: label fewer segments or shorten the labels",
                MAX_EXPANDED_LABEL_BYTES / (1024 * 1024)
            ),
            offset: 0,
            length: text.encode_utf16().count(),
        });
    }
    let mut flat = Vec::new();
    flatten(&items, None, &mut flat);
    let mut points = vec![RoutePoint {
        dist_m: 0.0,
        ele_m: Some(0.0),
        lat_lon: None,
        surface: None,
    }];
    let mut segments = Vec::with_capacity(flat.len());
    let (mut dist_m, mut ele_m) = (0.0, 0.0);
    for f in &flat {
        let len = f64::from(f.length_m);
        let (grade, mean_pct) = match f.grade {
            ProfileGrade::Constant { pct } => (SegmentGrade::Constant { pct }, pct),
            ProfileGrade::Ramp { from_pct, to_pct } => (
                SegmentGrade::Ramp { from_pct, to_pct },
                f64::midpoint(from_pct, to_pct),
            ),
        };
        let start_m = dist_m;
        dist_m += len;
        ele_m += len * mean_pct / 100.0;
        segments.push(RouteSegment {
            start_m,
            end_m: dist_m,
            label: f.label.cloned(),
            grade,
            surface: f.surface,
        });
        points.push(RoutePoint {
            dist_m,
            ele_m: Some(ele_m),
            lat_lon: None,
            surface: None,
        });
    }
    let from_file = flat.iter().any(|f| f.surface.is_some());
    Route::new_from_text(
        first_label(&items).map_or_else(|| UNLABELLED_NAME.to_owned(), Clone::clone),
        print_grade_profile(&items),
        points,
        segments,
        from_file,
    )
    .map_err(|e| RouteDslError {
        kind: RouteDslErrorKind::Invariant,
        message: e.to_string(),
        offset: 0,
        length: 0,
    })
}

/// The canonical text of a grade-profile route (what [`parse_route_dsl`] stored), `None` for a
/// route from a file. `parse_route_dsl(print_route_dsl(r))` equals `r`.
#[must_use]
pub fn print_route_dsl(route: &Route) -> Option<String> {
    match route.source() {
        RouteSource::GradeProfile { dsl } => Some(dsl.clone()),
        _ => None,
    }
}
