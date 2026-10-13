use serde::{Deserialize, Serialize};

/// Requested power or heart-rate intensity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PowerTarget {
    /// Fixed percentage of FTP.
    PctFtp {
        /// Percentage.
        value: f64,
    },
    /// Fixed watts.
    Watts {
        /// Watts.
        value: f64,
    },
    /// Percentage range; ERG uses its midpoint.
    Range {
        /// Lower percentage.
        lo: f64,
        /// Upper percentage.
        hi: f64,
    },
    /// Linear percentage ramp, possibly descending.
    Ramp {
        /// Initial percentage.
        from: f64,
        /// Final percentage.
        to: f64,
    },
    /// Heart-rate target.
    Hr {
        /// Beats per minute.
        bpm: f64,
    },
    /// Unconstrained riding.
    Free,
}

/// Policy when recovery heart rate is unreachable.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum OnUnreachable {
    /// Reduce recovery power.
    LowerPower,
    /// Continue until the maximum duration.
    Cap,
    /// Ask the coach.
    AskCoach,
}

/// Condition ending a workout step; boundary times are seconds (ADR-0001 §4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum StepEnd {
    /// Fixed duration.
    Time {
        /// Seconds.
        s: f64,
    },
    /// Accumulated work.
    Kj {
        /// Kilojoules.
        kj: f64,
    },
    /// Ends on manual skip.
    Lap,
    /// Heart rate below a threshold within duration bounds.
    HrRecovery {
        /// Threshold in beats per minute.
        target_bpm: f64,
        /// Earliest end, seconds.
        min_s: f64,
        /// Latest end, seconds.
        max_s: f64,
        /// Unreachable threshold policy.
        on_unreachable: OnUnreachable,
    },
}

/// Cadence bounds in revolutions per minute.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Cadence {
    /// Lower cadence.
    pub lo: f64,
    /// Upper cadence.
    pub hi: f64,
}

/// Intensity increments applied per repeat to work steps.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Progression {
    /// Additional watts per iteration.
    pub watts_per_rep: Option<f64>,
    /// Additional percentage points per iteration.
    pub pct_per_rep: Option<f64>,
}

/// A single workout step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct StepNode {
    /// Identity supplied by the caller, unrelated to tree position.
    pub id: String,
    /// Optional display label (serialized as null when absent).
    pub label: Option<String>,
    /// End condition.
    pub end: StepEnd,
    /// Intensity target.
    pub target: PowerTarget,
    /// Optional cadence bounds.
    pub cadence: Option<Cadence>,
}

/// A nested repeated sequence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct RepeatNode {
    /// Identity supplied by the caller.
    pub id: String,
    /// Optional display label.
    pub label: Option<String>,
    /// Number of iterations; represented as a JS number for reference parity.
    pub count: f64,
    /// Nested sequence.
    pub children: Vec<WorkoutNode>,
    /// Optional progression increments.
    pub progression: Option<Progression>,
    /// Skip the last child on the final iteration.
    pub skip_last_rest: Option<bool>,
}

/// Recursive workout tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkoutNode {
    /// Single step.
    Step(StepNode),
    /// Repeated sequence.
    Repeat(RepeatNode),
}

/// A named workout with its reference FTP.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Workout {
    /// Workout identity.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Functional threshold power in watts.
    pub ftp: f64,
    /// Workout tree.
    pub nodes: Vec<WorkoutNode>,
}

/// Machine-readable parser failure classes; token text remains available via the error span.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DslErrorKind {
    /// A quoted label has no closing quote.
    UnterminatedLabel,
    /// A repeat has no closing parenthesis.
    UnclosedRepeat,
    /// A token cannot start an item (including an unmatched closing parenthesis).
    UnexpectedToken,
    /// Consecutive items need a separator.
    MissingSeparator,
    /// A label must be followed by an item.
    MissingItemAfterLabel,
    /// Repeat count is below one.
    InvalidRepeatCount {
        /// Supplied iteration count.
        count: f64,
    },
    /// Repeat options have no closing bracket.
    UnclosedOptions,
    /// A non-word token occurs inside repeat options.
    UnexpectedRepeatOption,
    /// A word is not a supported repeat option.
    UnknownRepeatOption,
    /// Repeat count/options must be followed by an opening parenthesis.
    MissingRepeatBody,
    /// Repeat body contains no items.
    EmptyRepeat,
    /// A timed step must have positive duration.
    NonPositiveDuration,
    /// Heart-rate recovery needs min..max duration bounds.
    MissingRecoveryBounds,
    /// Recovery duration bounds are reversed.
    ReversedRecoveryBounds {
        /// Supplied minimum seconds.
        min_s: f64,
        /// Supplied maximum seconds.
        max_s: f64,
    },
    /// A bare integer needs a duration/work unit or repeat suffix.
    MissingUnit,
    /// A word is not a supported step end condition.
    InvalidStepEnd,
    /// Ramp keyword must be followed by a percentage range.
    MissingRampRange,
    /// A constant percentage range has reversed bounds.
    ReversedPowerRange,
    /// A step has more than one intensity target.
    DuplicatePowerTarget,
    /// A step has more than one cadence attribute.
    DuplicateCadence,
    /// Cadence bounds are reversed.
    ReversedCadenceRange,
    /// Recovery policy flag occurs on another kind of step.
    RecoveryFlagNotApplicable,
    /// A step attribute is not recognized.
    UnknownToken,
    /// A step needs an intensity target.
    MissingPowerTarget,
    /// Target intensity exceeds 400 percent FTP.
    ImplausiblePower {
        /// Computed intensity before message rounding.
        pct: f64,
    },
}

/// Parse failure selecting a span in the original text using UTF-16 code units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct DslError {
    /// Stable failure class for programmatic handling.
    pub kind: DslErrorKind,
    /// Human-readable reference error message.
    pub message: String,
    /// UTF-16 offset (as in JavaScript editors).
    pub offset: usize,
    /// UTF-16 length; zero selects end of input.
    pub length: usize,
}
impl std::fmt::Display for DslError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} @{}", self.message, self.offset)
    }
}
impl std::error::Error for DslError {}
