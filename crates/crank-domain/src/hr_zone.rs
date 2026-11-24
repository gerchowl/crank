//! The heart-rate zone of an HR value (DESIGN §18.3 "Heart-rate zone", #586, #645; ADR-0009 §2.1.4 "Zones").
//!
//! Two models, both from cited tables:
//!
//! - With an LTHR: Joe Friel's seven bike zones, as % of LTHR (J. Friel, "Joe Friel's Quick Guide to Setting
//!   Zones", `TrainingPeaks`, 2012, from his 2009 `TrainingBible` blog;
//!   <https://www.trainingpeaks.com/learn/articles/joe-friel-s-quick-guide-to-setting-zones/>): Z1 "Less than 81%",
//!   Z2 "81% to 89%", Z3 "90% to 93%", Z4 "94% to 99%", Z5a "100% to 102%", Z5b "103% to 106%", Z5c "More than
//!   106%".
//! - Otherwise, with an HR max: five zones as % of HR max (Polar, "Heart Rate Zones",
//!   <https://www.polar.com/blog/running-heart-rate-zones-basics/>): Z1 50–60 %, Z2 60–70 %, Z3 70–80 %,
//!   Z4 80–90 %, Z5 90–100 %.
//!
//! Both tables are written in whole percents, so the percent is rounded to a whole number first (half up, as
//! JavaScript's `Math.round`) and the table then applies as written: Friel's ranges leave no gap between them on
//! whole percents. Polar's adjacent zones share their edge (60, 70, 80, 90); the edge belongs to the higher zone
//! ("Z2 from 60 %"). Two choices of DESIGN §18.3: below 50 % of HR max is still Z1 (Friel's Z1 has no floor
//! either, and a blank would read as no data), and above 100 % is still Z5 (the HR max is an estimate the rider can
//! exceed).
//!
//! These tables are the canonical copy; `app/src/ride/hrZone.ts` reads them from here once the core exports them
//! (#715, #645). Which references a ride uses (never a prior HR max, ADR-0009 decision 5) is
//! [`crate::athlete::AthleteSnapshot::hr_refs`]'s rule, not this module's.
use crate::js_round;
use serde::{Deserialize, Serialize};

/// A zone's name as the tables write it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub enum HrZoneLabel {
    /// Friel Z1 (below 81 % of LTHR), or five-zone Z1 (below 60 % of HR max).
    Z1,
    /// Friel Z2 (81–89 % of LTHR), or five-zone Z2 (60–69 % of HR max).
    Z2,
    /// Friel Z3 (90–93 % of LTHR), or five-zone Z3 (70–79 % of HR max).
    Z3,
    /// Friel Z4 (94–99 % of LTHR), or five-zone Z4 (80–89 % of HR max).
    Z4,
    /// Five-zone Z5 (90 % of HR max and above).
    Z5,
    /// Friel Z5a (100–102 % of LTHR).
    Z5a,
    /// Friel Z5b (103–106 % of LTHR).
    Z5b,
    /// Friel Z5c (107 % of LTHR and above).
    Z5c,
}

/// The model a zone is read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum HrZoneModel {
    /// Friel's seven bike zones, % of LTHR.
    Lthr,
    /// Five zones, % of HR max.
    HrMax,
}

/// A zone: its label and the model it is read in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct HrZone {
    /// The zone's name, Z1 … Z5c.
    pub label: HrZoneLabel,
    /// Friel's % of LTHR, or % of HR max.
    pub model: HrZoneModel,
}

/// The athlete's HR references, bpm; `None` when not known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct HrRefs {
    /// Lactate-threshold heart rate.
    pub lthr_bpm: Option<f64>,
    /// Maximum heart rate.
    pub hr_max_bpm: Option<f64>,
}

/// Friel's bike zones: each zone's lowest whole percent of LTHR, highest first; below the last is Z1.
const FRIEL: [(f64, HrZoneLabel); 6] = [
    (107., HrZoneLabel::Z5c),
    (103., HrZoneLabel::Z5b),
    (100., HrZoneLabel::Z5a),
    (94., HrZoneLabel::Z4),
    (90., HrZoneLabel::Z3),
    (81., HrZoneLabel::Z2),
];
/// The five zones of % HR max: each zone's lowest whole percent, highest first; below the last is Z1.
const PCT_MAX: [(f64, HrZoneLabel); 4] = [
    (90., HrZoneLabel::Z5),
    (80., HrZoneLabel::Z4),
    (70., HrZoneLabel::Z3),
    (60., HrZoneLabel::Z2),
];

fn known(v: Option<f64>) -> Option<f64> {
    v.filter(|v| v.is_finite() && *v > 0.)
}

fn pick(hr: f64, reference: f64, table: &[(f64, HrZoneLabel)]) -> HrZoneLabel {
    let pct = js_round(100. * hr / reference);
    table
        .iter()
        .find(|(lo, _)| pct >= *lo)
        .map_or(HrZoneLabel::Z1, |&(_, label)| label)
}

/// Friel's bike zone of `hr` for this LTHR (both bpm, finite and positive).
#[must_use]
pub fn friel_zone(hr: f64, lthr_bpm: f64) -> HrZoneLabel {
    pick(hr, lthr_bpm, &FRIEL)
}

/// The five-zone % HR max zone of `hr` for this HR max (both bpm, finite and positive).
#[must_use]
pub fn pct_max_zone(hr: f64, hr_max_bpm: f64) -> HrZoneLabel {
    pick(hr, hr_max_bpm, &PCT_MAX)
}

/// The zone of an HR value: Friel's with an LTHR, else % of HR max, else none. A reference that is not finite or
/// not positive counts as unknown. No HR (`None`, not finite, not positive) is no zone too: the readout says why
/// (DESIGN §5.2 "HR lost").
#[must_use]
pub fn hr_zone(hr: Option<f64>, refs: HrRefs) -> Option<HrZone> {
    let hr = known(hr)?;
    if let Some(lthr) = known(refs.lthr_bpm) {
        return Some(HrZone {
            label: friel_zone(hr, lthr),
            model: HrZoneModel::Lthr,
        });
    }
    known(refs.hr_max_bpm).map(|max| HrZone {
        label: pct_max_zone(hr, max),
        model: HrZoneModel::HrMax,
    })
}

#[cfg(test)]
mod tests {
    // Ported from app/src/ride/hrZone.test.ts (#645). With a reference of 100 bpm a bpm is a whole percent, so each
    // case is a table edge as the source writes it.
    use super::HrZoneLabel::{Z1, Z2, Z3, Z4, Z5, Z5a, Z5b, Z5c};
    use super::*;

    #[test]
    fn friel_edges_as_written() {
        let cases = [
            (1., Z1),
            (80., Z1), // "Less than 81%"
            (81., Z2), // "81% to 89%"
            (89., Z2),
            (90., Z3), // "90% to 93%"
            (93., Z3),
            (94., Z4), // "94% to 99%"
            (99., Z4),
            (100., Z5a), // "100% to 102%"
            (102., Z5a),
            (103., Z5b), // "103% to 106%"
            (106., Z5b),
            (107., Z5c), // "More than 106%"
            (150., Z5c),
        ];
        for (pct, zone) in cases {
            assert_eq!(friel_zone(pct, 100.), zone, "{pct} %");
        }
    }

    #[test]
    fn friel_rounds_the_percent_half_up_first() {
        // LTHR 170: 137 bpm is 80.59 % → 81 → Z2; 136 bpm is 80.0 % → Z1.
        assert_eq!(friel_zone(136., 170.), Z1);
        assert_eq!(friel_zone(137., 170.), Z2);
        assert_eq!(friel_zone(80.5, 100.), Z2);
        assert_eq!(friel_zone(80.49, 100.), Z1);
        assert_eq!(friel_zone(106.49, 100.), Z5b);
        assert_eq!(friel_zone(106.5, 100.), Z5c);
    }

    #[test]
    fn pct_max_edges_as_written() {
        let cases = [
            (30., Z1), // below 50 % stays Z1 (DESIGN §18.3)
            (50., Z1), // "50–60%"
            (59., Z1),
            (60., Z2), // a shared edge belongs to the higher zone
            (69., Z2),
            (70., Z3),
            (79., Z3),
            (80., Z4),
            (89., Z4),
            (90., Z5), // "90–100%"
            (100., Z5),
            (104., Z5), // above the estimate stays Z5
        ];
        for (pct, zone) in cases {
            assert_eq!(pct_max_zone(pct, 100.), zone, "{pct} %");
        }
    }

    #[test]
    fn pct_max_rounds_like_friel() {
        assert_eq!(pct_max_zone(59.5, 100.), Z2);
        assert_eq!(pct_max_zone(59.49, 100.), Z1);
    }

    #[test]
    fn lthr_wins_over_hr_max() {
        // 150 / 160 = 93.75 % of LTHR → 94 → Z4; of HR max it would be 79 % → Z3.
        let refs = HrRefs {
            lthr_bpm: Some(160.),
            hr_max_bpm: Some(190.),
        };
        assert_eq!(
            hr_zone(Some(150.), refs),
            Some(HrZone {
                label: Z4,
                model: HrZoneModel::Lthr
            })
        );
    }

    #[test]
    fn hr_max_only_reads_the_five_zones() {
        // 150 / 190 = 78.9 % → 79 → Z3.
        let refs = HrRefs {
            lthr_bpm: None,
            hr_max_bpm: Some(190.),
        };
        assert_eq!(
            hr_zone(Some(150.), refs),
            Some(HrZone {
                label: Z3,
                model: HrZoneModel::HrMax
            })
        );
    }

    #[test]
    fn neither_reference_known_is_no_zone() {
        assert_eq!(hr_zone(Some(150.), HrRefs::default()), None);
    }

    #[test]
    fn a_reference_that_is_no_heart_rate_counts_as_unknown() {
        for bad in [0., -170., f64::NAN, f64::INFINITY] {
            assert_eq!(
                hr_zone(
                    Some(150.),
                    HrRefs {
                        lthr_bpm: Some(bad),
                        hr_max_bpm: None
                    }
                ),
                None,
                "{bad}"
            );
        }
        let refs = HrRefs {
            lthr_bpm: Some(0.),
            hr_max_bpm: Some(190.),
        };
        assert_eq!(
            hr_zone(Some(150.), refs).map(|z| z.model),
            Some(HrZoneModel::HrMax)
        );
    }

    #[test]
    fn no_hr_value_is_no_zone_whatever_the_references() {
        let refs = HrRefs {
            lthr_bpm: Some(160.),
            hr_max_bpm: Some(190.),
        };
        for hr in [None, Some(0.), Some(f64::NAN)] {
            assert_eq!(hr_zone(hr, refs), None, "{hr:?}");
        }
    }

    #[test]
    fn serde_names_match_the_app() {
        let z = HrZone {
            label: Z5a,
            model: HrZoneModel::HrMax,
        };
        assert_eq!(
            serde_json::to_string(&z).ok().as_deref(),
            Some(r#"{"label":"Z5a","model":"hrMax"}"#)
        );
    }
}
