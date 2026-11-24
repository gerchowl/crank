//! The athlete model: parameters, estimates, sources, plausibility bounds, priors and the ride's snapshot
//! (ADR-0009 §2.1.1, §2.1.2, §2.1.4). Pure: the store keeps the append-only estimate rows (#714), the core
//! appends and resolves them (#715).
//!
//! A value outside its bounds is refused, never clamped ([`check_estimate`]); a parameter with neither a row nor a
//! prior is unset ([`AthleteSnapshot`]), never 0.
pub mod prior;
mod snapshot;

pub use snapshot::{AthleteSnapshot, Resolved};

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// One of the eight rider-visible parameters of ADR-0009 §2.1.1. The serde and [`AthleteParam::as_str`] names are
/// the store's `athlete_estimate.param` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum AthleteParam {
    /// Functional threshold power, W.
    Ftp,
    /// Critical power, W.
    Cp,
    /// W′, the work capacity above CP, J.
    WPrime,
    /// Peak one-second power, W.
    Pmax,
    /// Maximum heart rate, bpm.
    HrMax,
    /// Lactate-threshold heart rate, bpm.
    Lthr,
    /// Resting heart rate, bpm.
    HrRest,
    /// Body weight, kg.
    Weight,
}

impl AthleteParam {
    /// Every parameter, in ADR-0009 §2.1.1's order.
    pub const ALL: [Self; 8] = [
        Self::Ftp,
        Self::Cp,
        Self::WPrime,
        Self::Pmax,
        Self::HrMax,
        Self::Lthr,
        Self::HrRest,
        Self::Weight,
    ];

    /// The stored name (`ftp`, `w_prime`, …).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ftp => "ftp",
            Self::Cp => "cp",
            Self::WPrime => "w_prime",
            Self::Pmax => "pmax",
            Self::HrMax => "hr_max",
            Self::Lthr => "lthr",
            Self::HrRest => "hr_rest",
            Self::Weight => "weight",
        }
    }

    /// The plausibility bounds of ADR-0009 §2.1.1 (a convention), inclusive, in the parameter's unit.
    #[must_use]
    pub const fn bounds(self) -> (f64, f64) {
        match self {
            Self::Ftp | Self::Cp => (50., 600.),
            Self::WPrime => (2_000., 60_000.),
            Self::Pmax => (200., 2_500.),
            Self::HrMax => (120., 230.),
            Self::Lthr => (100., 220.),
            Self::HrRest => (30., 110.),
            Self::Weight => (30., 200.),
        }
    }

    /// Whether a value of `source` may be stored for this parameter (ADR-0009 §2.1.1 "Sources allowed").
    #[must_use]
    pub const fn allows(self, source: Source) -> bool {
        match source {
            Source::SelfReport | Source::Imported => true,
            Source::Prior => matches!(self, Self::Ftp | Self::Cp | Self::WPrime | Self::HrMax),
            Source::Test => matches!(
                self,
                Self::Ftp | Self::Cp | Self::WPrime | Self::HrMax | Self::Lthr
            ),
            Source::Rides => !matches!(self, Self::HrRest | Self::Weight),
        }
    }
}

impl fmt::Display for AthleteParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AthleteParam {
    type Err = AthleteError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or(AthleteError::UnknownParam)
    }
}

/// Where an estimate came from (ONBOARDING §3's names). The serde and [`Source::as_str`] names are the store's
/// `athlete_estimate.source` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A named prior of [`prior`], not a measurement.
    Prior,
    /// Typed in by the rider (Settings, onboarding).
    SelfReport,
    /// The result of a dedicated assessment ride (ADR-0004).
    Test,
    /// From the analysis of ordinary rides (ADR-0009 §2.2.4).
    Rides,
    /// Imported from another service.
    Imported,
}

impl Source {
    /// Every source.
    pub const ALL: [Self; 5] = [
        Self::Prior,
        Self::SelfReport,
        Self::Test,
        Self::Rides,
        Self::Imported,
    ];

    /// The stored name (`prior`, `self_report`, …).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prior => "prior",
            Self::SelfReport => "self_report",
            Self::Test => "test",
            Self::Rides => "rides",
            Self::Imported => "imported",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Source {
    type Err = AthleteError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or(AthleteError::UnknownSource)
    }
}

/// The method name of a prior row ([`prior`]).
pub const METHOD_PRIOR: &str = "prior_v1";
/// The method name of a value typed in by the rider.
pub const METHOD_MANUAL: &str = "manual";

/// One row of a parameter's append-only history (ADR-0009 §2.1.1). The current value of a parameter is its newest
/// row ([`AthleteSnapshot::resolve`]); a manual change, an accepted proposal or a revert each add a row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    /// The row; stable, referenced by rides and proposals.
    pub id: i64,
    /// The parameter.
    pub param: AthleteParam,
    /// The value, in the parameter's unit.
    pub value: f64,
    /// Its standard deviation, same unit.
    pub sd: f64,
    /// Where it came from.
    pub source: Source,
    /// How it was derived: [`METHOD_PRIOR`], [`METHOD_MANUAL`], `ramp_v1_0.75`, `peak_hr_v1`, `cp_ols_v1`, ….
    pub method: String,
    /// The ride the value came from, if any.
    pub ride_id: Option<String>,
    /// The accepted proposal that wrote it, if any.
    pub proposal_id: Option<i64>,
    /// Number of efforts or rides behind it.
    pub n: u32,
    /// When the row was written, UTC seconds.
    pub updated_at: i64,
}

/// Why a parameter value is refused; the core answers each with `InvalidInput`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AthleteError {
    /// The name is none of the eight parameters.
    UnknownParam,
    /// The name is none of the five sources.
    UnknownSource,
    /// The value or its sd is NaN or infinite.
    NonFinite {
        /// The parameter.
        param: AthleteParam,
    },
    /// The value is outside the parameter's plausibility bounds ([`AthleteParam::bounds`]).
    OutOfRange {
        /// The parameter.
        param: AthleteParam,
        /// Lowest allowed value.
        min: f64,
        /// Highest allowed value.
        max: f64,
    },
    /// The sd is not above 0, or above 50 % of the value.
    SdOutOfRange {
        /// The parameter.
        param: AthleteParam,
    },
    /// The parameter takes no value of this source (ADR-0009 §2.1.1 "Sources allowed").
    SourceNotAllowed {
        /// The parameter.
        param: AthleteParam,
        /// The refused source.
        source: Source,
    },
    /// LTHR would be at or above HR max.
    LthrNotBelowHrMax {
        /// The LTHR, bpm.
        lthr: f64,
        /// The HR max, bpm.
        hr_max: f64,
    },
}

impl fmt::Display for AthleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownParam => f.write_str("unknown athlete parameter"),
            Self::UnknownSource => f.write_str("unknown estimate source"),
            Self::NonFinite { param } => write!(f, "{param}: value and sd must be finite"),
            Self::OutOfRange { param, min, max } => {
                write!(f, "{param}: value must be within {min}–{max}")
            }
            Self::SdOutOfRange { param } => write!(
                f,
                "{param}: sd must be above 0 and at most 50 % of the value"
            ),
            Self::SourceNotAllowed { param, source } => {
                write!(f, "{param}: no {source} value allowed")
            }
            Self::LthrNotBelowHrMax { lthr, hr_max } => {
                write!(f, "LTHR {lthr} must be below HR max {hr_max}")
            }
        }
    }
}
impl std::error::Error for AthleteError {}

/// Checks a value and its sd on their own: both finite, the value within [`AthleteParam::bounds`], the sd above 0
/// and at most 50 % of the value (ADR-0009 §2.1.1).
///
/// # Errors
/// [`AthleteError::NonFinite`], [`AthleteError::OutOfRange`] or [`AthleteError::SdOutOfRange`].
pub fn check_value(param: AthleteParam, value: f64, sd: f64) -> Result<(), AthleteError> {
    if !value.is_finite() || !sd.is_finite() {
        return Err(AthleteError::NonFinite { param });
    }
    let (min, max) = param.bounds();
    if !(min..=max).contains(&value) {
        return Err(AthleteError::OutOfRange { param, min, max });
    }
    if sd <= 0. || sd > 0.5 * value {
        return Err(AthleteError::SdOutOfRange { param });
    }
    Ok(())
}

/// Checks a new row before it is appended: [`check_value`], the source is allowed for the parameter, and LTHR stays
/// below HR max against the `current` model. The cross check reads only a measured (non-prior) LTHR or HR max of
/// `current`: a prior is never a zone reference (ADR-0009 decision 5, §2.1.2), so a Tanaka HR max does not refuse a
/// measured LTHR.
///
/// # Errors
/// Any [`check_value`] error, [`AthleteError::SourceNotAllowed`] or [`AthleteError::LthrNotBelowHrMax`].
pub fn check_estimate(
    param: AthleteParam,
    value: f64,
    sd: f64,
    source: Source,
    current: &AthleteSnapshot,
) -> Result<(), AthleteError> {
    check_value(param, value, sd)?;
    if !param.allows(source) {
        return Err(AthleteError::SourceNotAllowed { param, source });
    }
    let pair = match param {
        AthleteParam::Lthr => current
            .hr_max
            .filter(Resolved::is_measured)
            .map(|m| (value, m.value)),
        AthleteParam::HrMax => current
            .lthr
            .filter(Resolved::is_measured)
            .map(|l| (l.value, value)),
        _ => None,
    };
    match pair {
        Some((lthr, hr_max)) if lthr >= hr_max => {
            Err(AthleteError::LthrNotBelowHrMax { lthr, hr_max })
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
