//! The athlete model a ride is loaded with (ADR-0009 §2.1.4), stored as `ride.athlete_json`.
use super::prior::prior;
use super::{AthleteParam, Estimate, Source};
use crate::hr_zone::{HrRefs, HrZone, hr_zone};
use serde::{Deserialize, Serialize};

/// A parameter's resolved value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Resolved {
    /// The value, in the parameter's unit.
    pub value: f64,
    /// Its standard deviation.
    pub sd: f64,
    /// Where it came from.
    pub source: Source,
    /// The estimate row it was read from; `None` for a prior of [`super::prior`] that no row holds (a parameter
    /// never set nor cleared).
    pub estimate_id: Option<i64>,
}

impl Resolved {
    /// Whether the value is a measurement or the rider's own, not a prior. Only such a value is a zone reference
    /// (ADR-0009 decision 5).
    #[must_use]
    pub fn is_measured(&self) -> bool {
        self.source != Source::Prior
    }
}

/// The athlete model frozen at `ride_load`: each parameter resolved, or `None` when unset (never 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct AthleteSnapshot {
    /// Functional threshold power, W.
    pub ftp: Option<Resolved>,
    /// Critical power, W.
    pub cp: Option<Resolved>,
    /// W′, J.
    pub w_prime: Option<Resolved>,
    /// Peak one-second power, W.
    pub pmax: Option<Resolved>,
    /// Maximum heart rate, bpm.
    pub hr_max: Option<Resolved>,
    /// Lactate-threshold heart rate, bpm.
    pub lthr: Option<Resolved>,
    /// Resting heart rate, bpm.
    pub hr_rest: Option<Resolved>,
    /// Body weight, kg.
    pub weight: Option<Resolved>,
}

impl AthleteSnapshot {
    /// Resolves every parameter from its rows: the newest row (largest `updated_at`, then largest `id`: rows are
    /// append-only, so of two rows written in the same second the later one has the larger id); with no row, or
    /// when the newest is a clear marker ([`Estimate::cleared`]), the parameter's prior ([`super::prior::prior`],
    /// HR max from `age_years`) with no estimate id; with neither, unset.
    ///
    /// `estimates` may hold several parameters in any order. The rows are trusted as stored: every append is
    /// checked by [`super::check_estimate`].
    #[must_use]
    pub fn resolve(estimates: &[Estimate], age_years: Option<u32>) -> Self {
        let mut snapshot = Self::default();
        for param in AthleteParam::ALL {
            let newest = estimates
                .iter()
                .filter(|e| e.param == param)
                .max_by_key(|e| (e.updated_at, e.id))
                .filter(|e| !e.is_cleared());
            *snapshot.slot(param) = match newest {
                Some(e) => Some(Resolved {
                    value: e.value,
                    sd: e.sd,
                    source: e.source,
                    estimate_id: Some(e.id),
                }),
                None => prior(param, age_years).map(|p| Resolved {
                    value: p.value,
                    sd: p.sd,
                    source: Source::Prior,
                    estimate_id: None,
                }),
            };
        }
        snapshot
    }

    /// The resolved value of `param`, or `None` when unset.
    #[must_use]
    pub fn get(&self, param: AthleteParam) -> Option<Resolved> {
        match param {
            AthleteParam::Ftp => self.ftp,
            AthleteParam::Cp => self.cp,
            AthleteParam::WPrime => self.w_prime,
            AthleteParam::Pmax => self.pmax,
            AthleteParam::HrMax => self.hr_max,
            AthleteParam::Lthr => self.lthr,
            AthleteParam::HrRest => self.hr_rest,
            AthleteParam::Weight => self.weight,
        }
    }

    fn slot(&mut self, param: AthleteParam) -> &mut Option<Resolved> {
        match param {
            AthleteParam::Ftp => &mut self.ftp,
            AthleteParam::Cp => &mut self.cp,
            AthleteParam::WPrime => &mut self.w_prime,
            AthleteParam::Pmax => &mut self.pmax,
            AthleteParam::HrMax => &mut self.hr_max,
            AthleteParam::Lthr => &mut self.lthr,
            AthleteParam::HrRest => &mut self.hr_rest,
            AthleteParam::Weight => &mut self.weight,
        }
    }

    /// The HR zone references of this snapshot: a measured (non-prior) LTHR and HR max only, so a new rider sees no
    /// HR zone rather than one drawn from a prior HR max (ADR-0009 §2.1.2, decision 5).
    #[must_use]
    pub fn hr_refs(&self) -> HrRefs {
        let measured = |r: Option<Resolved>| r.filter(Resolved::is_measured).map(|r| r.value);
        HrRefs {
            lthr_bpm: measured(self.lthr),
            hr_max_bpm: measured(self.hr_max),
        }
    }

    /// The HR zone of `hr` under this snapshot ([`hr_zone`] over [`Self::hr_refs`]).
    #[must_use]
    pub fn hr_zone(&self, hr: Option<f64>) -> Option<HrZone> {
        hr_zone(hr, self.hr_refs())
    }
}
