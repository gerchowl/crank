//! Compiler contract; seconds, watts and dimensionless training scores (ADR-0001 §3–4).
use serde::{Deserialize, Serialize};

/// Reference athlete parameters used for compile-time HR estimates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Athlete {
    /// Functional threshold power, watts; compilation overrides it with workout FTP.
    pub ftp: f64,
    /// Body mass, kilograms.
    pub weight_kg: f64,
    /// Maximum heart rate, beats per minute.
    pub max_hr: f64,
    /// Resting heart rate, beats per minute.
    pub rest_hr: f64,
}
impl Default for Athlete {
    fn default() -> Self {
        Self {
            ftp: 280.,
            weight_kg: 72.,
            max_hr: 190.,
            rest_hr: 50.,
        }
    }
}
/// Optional physiological inputs; defaults match the frozen reference.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CompileOptions {
    /// Athlete parameters; workout FTP always wins.
    pub athlete: Option<Athlete>,
    /// Usual HR recovery time constant, seconds (default 55).
    pub usual_tau_s: Option<f64>,
}
/// Additive repeat progression, applied only to work steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Offset {
    /// Additional watts.
    pub watts: f64,
    /// Additional percentage points of FTP.
    pub pct: f64,
}
/// Display phase, classified before progression changes intensity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Easy blocks before work, or explicitly labelled warm.
    Warmup,
    /// Work intensity, or the highest intensities in an easy ride.
    Work,
    /// Easy block between work or HR-triggered recovery.
    Recovery,
    /// Easy blocks after work, or explicitly labelled cool.
    Cooldown,
    /// Unconstrained riding.
    Free,
}
/// Flattened occurrence of a step; repeated occurrences retain the source identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct FlatBlock {
    /// Zero-based child indices from the root to the source step.
    pub path: Vec<usize>,
    /// Zero-based enclosing repeat iterations.
    pub iteration: Vec<usize>,
    /// Source step identity.
    pub node_id: String,
    /// Start on the workout timeline, seconds.
    pub start_s: f64,
    /// Expected duration, seconds.
    pub dur_s: f64,
    /// Initial target, watts (rounded to tenths).
    pub power_from: f64,
    /// Final target, watts (rounded to tenths).
    pub power_to: f64,
    /// Display phase.
    pub phase: Phase,
    /// Open-ended duration bounds, seconds; omitted for fixed-duration steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dur_range_s: Option<[f64; 2]>,
}
/// Validation severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Invalid prescription.
    Error,
    /// Potentially unsuitable prescription.
    Warning,
}
/// Machine-readable prescription diagnostics; messages retain reference wording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum ValidationKind {
    /// FTP is not positive.
    InvalidFtp,
    /// Repeat count is not a positive integer.
    InvalidRepeatCount,
    /// Repeat contains no children.
    EmptyRepeat,
    /// Time duration is not positive.
    InvalidDuration,
    /// Work target is not positive.
    InvalidKilojoules,
    /// Recovery minimum exceeds maximum.
    ReversedRecoveryBounds,
    /// Recovery HR threshold is outside the athlete's resting and maximum HR.
    RecoveryTargetOutsideRange,
    /// Recovery HR threshold is likely unreachable.
    UnreachableRecovery,
    /// Target exceeds the hardware or four-times-FTP ceiling.
    ExcessivePower,
    /// Sprint intensity lasts over 60 seconds.
    LongSprint,
    /// W′ reserve is negative.
    DepletedReserve,
    /// W′ reserve is below 15 percent.
    LowReserve,
    /// No root nodes.
    EmptyWorkout,
    /// A ride longer than 20 minutes lacks 180 seconds of final cooldown.
    MissingCooldown,
}
/// Prescription diagnostic with a typed kind and human-readable reference message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    /// Machine-readable diagnosis.
    pub kind: ValidationKind,
    /// Error or warning.
    pub severity: Severity,
    /// Source identity, absent for whole-workout diagnostics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Display message; consumers should branch on kind.
    pub message: String,
}
/// Compiled timeline and estimated training load.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CompiledWorkout {
    /// Ordered blocks tiling the expected timeline.
    pub blocks: Vec<FlatBlock>,
    /// Expected total duration, seconds.
    pub total_s: f64,
    /// Sum of lower and upper duration bounds, seconds.
    pub total_range_s: [f64; 2],
    /// Training stress score, rounded to tenths.
    pub tss: f64,
    /// Intensity factor (NP / FTP), rounded to thousandths.
    #[serde(rename = "if")]
    pub intensity_factor: f64,
    /// Rounded one-second trace sample counts in seven power zones.
    pub time_in_zone_s: [u32; 7],
    /// Prescription diagnostics in reference order.
    pub issues: Vec<ValidationIssue>,
}
/// Compilation cannot safely produce finite, bounded output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum CompileError {
    /// An input or intermediate calculation is nonfinite.
    NonFinite,
    /// Repeat nesting exceeds the DSL's bound.
    NestingTooDeep,
    /// More than 100,000 expanded node visits.
    ExpansionLimit,
    /// More than 604,800 rounded one-second samples, or duration seconds.
    TraceLimit,
    /// A positive duration cannot advance the floating-point timeline.
    TimelinePrecision,
}
impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for CompileError {}
