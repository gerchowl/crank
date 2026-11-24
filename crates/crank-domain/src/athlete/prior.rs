//! The priors of ADR-0009 §2.1.2 and the self-report sd by stated origin, as named constants.
//!
//! They replace the hidden constants (`library.rs` `FTP_W` and the W′ of `compile.rs`, removed with #715; #247's
//! fallbacks). LTHR, resting HR, Pmax and weight have no prior: unknown is shown as "Not set". A prior is never a
//! zone reference ([`super::AthleteSnapshot::hr_refs`]).
use super::{AthleteParam, check_value};

/// FTP prior, W: today's constant (convention).
pub const FTP_W: f64 = 250.;
/// FTP prior sd as a fraction of the value (convention).
pub const FTP_SD_FRACTION: f64 = 0.25;
/// CP prior, W: the FTP prior's value. On average CP sits 7 ± 13 W above a 20-min FTP (\[K21\] in ADR-0009), too
/// small to encode in a prior this wide (convention).
pub const CP_W: f64 = FTP_W;
/// CP prior sd as a fraction of the value (convention).
pub const CP_SD_FRACTION: f64 = 0.25;
/// W′ prior, J: today's `compile.rs` value (convention).
pub const W_PRIME_J: f64 = 20_000.;
/// W′ prior sd as a fraction of the value (convention).
pub const W_PRIME_SD_FRACTION: f64 = 0.30;
/// HR max prior sd, bpm: the individual error of Tanaka's formula is near ±10 bpm (\[T01\] in ADR-0009, a
/// meta-analysis of 351 studies; evidence).
pub const HR_MAX_SD_BPM: f64 = 10.;

/// A prior's value and sd, in the parameter's unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prior {
    /// The value.
    pub value: f64,
    /// Its standard deviation.
    pub sd: f64,
}

/// The HR max prior of Tanaka et al. (\[T01\]): 208 − 0.7 × age, bpm. `None` when the result is outside HR max's
/// plausibility bounds (an age above 125 years), which no stored row could hold either.
#[must_use]
pub fn tanaka_hr_max(age_years: u32) -> Option<f64> {
    let value = 208. - 0.7 * f64::from(age_years);
    check_value(AthleteParam::HrMax, value, HR_MAX_SD_BPM)
        .ok()
        .map(|()| value)
}

/// The prior of `param`, if it has one. HR max needs the rider's age (`athlete_profile.age`); without it there is
/// none.
#[must_use]
pub fn prior(param: AthleteParam, age_years: Option<u32>) -> Option<Prior> {
    match param {
        AthleteParam::Ftp => Some(Prior {
            value: FTP_W,
            sd: FTP_W * FTP_SD_FRACTION,
        }),
        AthleteParam::Cp => Some(Prior {
            value: CP_W,
            sd: CP_W * CP_SD_FRACTION,
        }),
        AthleteParam::WPrime => Some(Prior {
            value: W_PRIME_J,
            sd: W_PRIME_J * W_PRIME_SD_FRACTION,
        }),
        AthleteParam::HrMax => age_years.and_then(tanaka_hr_max).map(|value| Prior {
            value,
            sd: HR_MAX_SD_BPM,
        }),
        AthleteParam::Lthr | AthleteParam::HrRest | AthleteParam::Pmax | AthleteParam::Weight => {
            None
        }
    }
}

/// The stated origin of a self-reported number (ONBOARDING §1 item 3), which sets its sd (ADR-0009 §2.1.2,
/// convention; the 8 % is ADR-0004's floor).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum SelfReportOrigin {
    /// From a ramp or 20-min test: 8 %.
    Test,
    /// From another app's estimate: 10 %.
    AppEstimate,
    /// A guess: 20 %. The default sd of a typed-in value (ADR-0009 §2.1.3).
    Guess,
}

impl SelfReportOrigin {
    /// The sd as a fraction of the value.
    #[must_use]
    pub const fn sd_fraction(self) -> f64 {
        match self {
            Self::Test => 0.08,
            Self::AppEstimate => 0.10,
            Self::Guess => 0.20,
        }
    }

    /// The sd of a self-reported `value` of this origin.
    #[must_use]
    pub fn sd(self, value: f64) -> f64 {
        value * self.sd_fraction()
    }
}
