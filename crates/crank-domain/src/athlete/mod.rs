//! The athlete model: parameters, estimates, sources, plausibility bounds, priors and the ride's snapshot
//! (ADR-0009 §2.1.1, §2.1.2, §2.1.4). Pure: the store keeps the append-only estimate rows (#714), the core
//! appends and resolves them (#715). A history row is an [`Entry`]: a value ([`Estimate`]) or a clear ([`Cleared`]).
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

/// A value row of a parameter's history (ADR-0009 §2.1.1); see [`Entry`] for the history itself.
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

impl Estimate {
    /// The prior row of `param` (source [`Source::Prior`], method [`METHOD_PRIOR`], `n` 0), or `None` when the
    /// parameter has no prior for `age_years`.
    #[must_use]
    pub fn prior_row(
        id: i64,
        param: AthleteParam,
        age_years: Option<u32>,
        updated_at: i64,
    ) -> Option<Self> {
        prior::prior(param, age_years).map(|p| Self {
            id,
            param,
            value: p.value,
            sd: p.sd,
            source: Source::Prior,
            method: METHOD_PRIOR.to_owned(),
            ride_id: None,
            proposal_id: None,
            n: 0,
            updated_at,
        })
    }
}

/// A Clear of a parameter that has no prior: it carries no value, and while it is the parameter's newest entry the
/// parameter is unset (ADR-0009 §2.1.3 "marks the param unset when there is no prior").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Cleared {
    /// The row; stable, in the same id sequence as the estimates.
    pub id: i64,
    /// The parameter.
    pub param: AthleteParam,
    /// When the row was written, UTC seconds.
    pub updated_at: i64,
}

/// One row of a parameter's append-only history (ADR-0009 §2.1.1): a value, or a clear. The current state of a
/// parameter is its newest entry ([`AthleteSnapshot::resolve`]); a manual change, an accepted proposal, a revert or
/// a Clear each append one.
///
/// **Clear** (`athlete_clear`, ADR-0009 §2.1.3) appends exactly one entry, [`Entry::clear`]: the prior's row
/// ([`Estimate::prior_row`]) when the parameter has a prior, else [`Entry::Cleared`].
///
/// **Store contract (#714):** every entry is checked with [`check_entry`] before it is appended, so every stored
/// entry passes it. A [`Cleared`] has no value, sd, source, method, ride, proposal or count: the table needs a row
/// kind (`estimate` or `cleared`) with those columns NULL exactly for `cleared`, never a placeholder value. History
/// lists a cleared row as "cleared"; revert refuses it (`InvalidInput`), as it refuses a prior row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Entry {
    /// A value of the parameter.
    Estimate(Estimate),
    /// The parameter was cleared and has no prior: unset.
    Cleared(Cleared),
}

impl Entry {
    /// The entry Clear appends for `param` (ADR-0009 §2.1.3): the prior's row if the parameter has a prior for
    /// `age_years`, else [`Entry::Cleared`].
    #[must_use]
    pub fn clear(id: i64, param: AthleteParam, age_years: Option<u32>, updated_at: i64) -> Self {
        Estimate::prior_row(id, param, age_years, updated_at).map_or(
            Self::Cleared(Cleared {
                id,
                param,
                updated_at,
            }),
            Self::Estimate,
        )
    }

    /// The row id.
    #[must_use]
    pub fn id(&self) -> i64 {
        match self {
            Self::Estimate(e) => e.id,
            Self::Cleared(c) => c.id,
        }
    }

    /// The parameter.
    #[must_use]
    pub fn param(&self) -> AthleteParam {
        match self {
            Self::Estimate(e) => e.param,
            Self::Cleared(c) => c.param,
        }
    }

    /// When the row was written, UTC seconds.
    #[must_use]
    pub fn updated_at(&self) -> i64 {
        match self {
            Self::Estimate(e) => e.updated_at,
            Self::Cleared(c) => c.updated_at,
        }
    }
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
/// below HR max against the `current` model. The cross check compares two measured (non-prior) values only: a
/// prior is never a zone reference (ADR-0009 decision 5, §2.1.2), so neither a Tanaka HR max in `current` refuses a
/// measured LTHR nor a new prior HR max row (Clear) is refused by a measured LTHR.
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
    if source == Source::Prior {
        return Ok(());
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

/// Checks an entry before it is appended: an estimate with [`check_estimate`]; a [`Cleared`] is allowed for every
/// parameter (it holds no value to check). The store appends only entries that pass (#714).
///
/// # Errors
/// Any [`check_estimate`] error of an [`Entry::Estimate`].
pub fn check_entry(entry: &Entry, current: &AthleteSnapshot) -> Result<(), AthleteError> {
    match entry {
        Entry::Estimate(e) => check_estimate(e.param, e.value, e.sd, e.source, current),
        Entry::Cleared(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests;
