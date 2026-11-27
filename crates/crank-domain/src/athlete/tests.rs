//! ADR-0009 §2.8 "Domain": priors and bounds; each bound refuses; resolution picks the newest row and reports unset
//! parameters as unset, never 0; the snapshot round-trips through `ride.athlete_json`.
use super::prior::{self, Prior, SelfReportOrigin};
use super::*;
use crate::hr_zone::{HrZoneLabel, HrZoneModel};
use proptest::prelude::*;

fn row(id: i64, param: AthleteParam, value: f64, source: Source, updated_at: i64) -> Estimate {
    Estimate {
        id,
        param,
        value,
        sd: value * 0.08,
        source,
        method: METHOD_MANUAL.to_owned(),
        ride_id: None,
        proposal_id: None,
        n: 1,
        updated_at,
    }
}

fn entry(id: i64, param: AthleteParam, value: f64, source: Source, updated_at: i64) -> Entry {
    Entry::Estimate(row(id, param, value, source, updated_at))
}

fn cleared(id: i64, param: AthleteParam, updated_at: i64) -> Entry {
    Entry::Cleared(Cleared {
        id,
        param,
        updated_at,
    })
}

fn measured(value: f64) -> Resolved {
    Resolved {
        value,
        sd: value * 0.08,
        source: Source::SelfReport,
        estimate_id: Some(1),
    }
}

#[test]
fn priors_are_the_adr_table() {
    use AthleteParam as P;
    assert_eq!(
        prior::prior(P::Ftp, None),
        Some(Prior {
            value: 250.,
            sd: 62.5
        })
    );
    assert_eq!(
        prior::prior(P::Cp, None),
        Some(Prior {
            value: 250.,
            sd: 62.5
        })
    );
    assert_eq!(
        prior::prior(P::WPrime, None),
        Some(Prior {
            value: 20_000.,
            sd: 6_000.
        })
    );
    // Tanaka: 208 − 0.7 × 40 = 180 bpm ± 10; no age, no prior.
    assert_eq!(
        prior::prior(P::HrMax, Some(40)),
        Some(Prior {
            value: 180.,
            sd: 10.
        })
    );
    assert_eq!(prior::prior(P::HrMax, None), None);
    for p in [P::Lthr, P::HrRest, P::Pmax, P::Weight] {
        assert_eq!(prior::prior(p, Some(40)), None, "{p}");
    }
    // Every prior is a value its own bounds accept, with an allowed source.
    for p in AthleteParam::ALL {
        if let Some(Prior { value, sd }) = prior::prior(p, Some(40)) {
            assert_eq!(check_value(p, value, sd), Ok(()), "{p}");
            assert!(p.allows(Source::Prior), "{p}");
        } else {
            assert!(!p.allows(Source::Prior), "{p}");
        }
    }
}

#[test]
fn tanaka_has_no_prior_past_the_hr_max_bounds() {
    // 208 − 0.7 × 125 = 120.5 is inside; 126 gives 119.8, outside.
    assert_eq!(prior::tanaka_hr_max(125), Some(208. - 0.7 * 125.));
    assert_eq!(prior::tanaka_hr_max(126), None);
    assert_eq!(prior::tanaka_hr_max(0), Some(208.));
}

#[test]
fn self_report_sd_by_origin() {
    assert!((SelfReportOrigin::Test.sd(250.) - 20.).abs() < 1e-9);
    assert!((SelfReportOrigin::AppEstimate.sd(250.) - 25.).abs() < 1e-9);
    assert!((SelfReportOrigin::Guess.sd(250.) - 50.).abs() < 1e-9);
}

#[test]
fn bounds_are_inclusive_and_refuse_outside() {
    let cases = [
        (AthleteParam::Ftp, 50., 600.),
        (AthleteParam::Cp, 50., 600.),
        (AthleteParam::WPrime, 2_000., 60_000.),
        (AthleteParam::Pmax, 200., 2_500.),
        (AthleteParam::HrMax, 120., 230.),
        (AthleteParam::Lthr, 100., 220.),
        (AthleteParam::HrRest, 30., 110.),
        (AthleteParam::Weight, 30., 200.),
    ];
    for (p, min, max) in cases {
        assert_eq!(p.bounds(), (min, max), "{p}");
        assert_eq!(check_value(p, min, min * 0.1), Ok(()), "{p} min");
        assert_eq!(check_value(p, max, max * 0.1), Ok(()), "{p} max");
        for v in [min - 1e-9, max + 1e-9, 0., -min] {
            assert_eq!(
                check_value(p, v, 1.),
                Err(AthleteError::OutOfRange { param: p, min, max }),
                "{p} {v}"
            );
        }
    }
}

#[test]
fn non_finite_values_and_sds_are_refused() {
    for p in AthleteParam::ALL {
        let (min, _) = p.bounds();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(
                check_value(p, bad, 1.),
                Err(AthleteError::NonFinite { param: p }),
                "{p}"
            );
            assert_eq!(
                check_value(p, min, bad),
                Err(AthleteError::NonFinite { param: p }),
                "{p}"
            );
        }
    }
}

#[test]
fn sd_must_be_positive_and_at_most_half_the_value() {
    let p = AthleteParam::Ftp;
    assert_eq!(check_value(p, 250., 125.), Ok(()));
    for sd in [0., -1., 125.000_001] {
        assert_eq!(
            check_value(p, 250., sd),
            Err(AthleteError::SdOutOfRange { param: p }),
            "{sd}"
        );
    }
}

#[test]
fn sources_allowed_per_param() {
    use AthleteParam as P;
    use Source as S;
    let refused = [
        (P::Pmax, S::Prior),
        (P::Pmax, S::Test),
        (P::Lthr, S::Prior),
        (P::HrRest, S::Prior),
        (P::HrRest, S::Test),
        (P::HrRest, S::Rides),
        (P::Weight, S::Prior),
        (P::Weight, S::Test),
        (P::Weight, S::Rides),
    ];
    for p in AthleteParam::ALL {
        for s in Source::ALL {
            assert_eq!(p.allows(s), !refused.contains(&(p, s)), "{p} {s}");
        }
    }
    let none = AthleteSnapshot::default();
    assert_eq!(
        check_estimate(P::Weight, 70., 2., S::Test, &none),
        Err(AthleteError::SourceNotAllowed {
            param: P::Weight,
            source: S::Test
        })
    );
}

#[test]
fn lthr_at_or_above_hr_max_is_refused_both_ways() {
    let with_max = AthleteSnapshot {
        hr_max: Some(measured(185.)),
        ..AthleteSnapshot::default()
    };
    let s = Source::SelfReport;
    assert_eq!(
        check_estimate(AthleteParam::Lthr, 184., 10., s, &with_max),
        Ok(())
    );
    for lthr in [185., 190.] {
        assert_eq!(
            check_estimate(AthleteParam::Lthr, lthr, 10., s, &with_max),
            Err(AthleteError::LthrNotBelowHrMax { lthr, hr_max: 185. })
        );
    }
    let with_lthr = AthleteSnapshot {
        lthr: Some(measured(170.)),
        ..AthleteSnapshot::default()
    };
    assert_eq!(
        check_estimate(AthleteParam::HrMax, 171., 10., s, &with_lthr),
        Ok(())
    );
    assert_eq!(
        check_estimate(AthleteParam::HrMax, 170., 10., s, &with_lthr),
        Err(AthleteError::LthrNotBelowHrMax {
            lthr: 170.,
            hr_max: 170.
        })
    );
}

#[test]
fn a_prior_hr_max_does_not_refuse_a_measured_lthr() {
    // Age 60: the Tanaka prior is 166 bpm; a typed LTHR of 170 is the rider's measurement and the prior is no
    // reference (ADR-0009 decision 5).
    let snap = AthleteSnapshot::resolve(&[], Some(60));
    assert_eq!(snap.hr_max.map(|r| r.source), Some(Source::Prior));
    assert_eq!(
        check_estimate(AthleteParam::Lthr, 170., 10., Source::SelfReport, &snap),
        Ok(())
    );
}

#[test]
fn a_measured_lthr_does_not_refuse_a_prior_hr_max_entry() {
    // Clear on HR max for a 60-year-old with a typed LTHR of 170: the new row is the Tanaka prior of 166 bpm.
    let snap = AthleteSnapshot::resolve(
        &[entry(1, AthleteParam::Lthr, 170., Source::SelfReport, 1)],
        Some(60),
    );
    let clear = Estimate::prior_row(2, AthleteParam::HrMax, Some(60), 2);
    let clear = clear.map(|e| (e.value, e.sd, e.source));
    assert_eq!(clear, Some((166., 10., Source::Prior)));
    assert_eq!(
        check_estimate(AthleteParam::HrMax, 166., 10., Source::Prior, &snap),
        Ok(())
    );
    // A measured HR max at the same value is still refused.
    assert_eq!(
        check_estimate(AthleteParam::HrMax, 166., 10., Source::SelfReport, &snap),
        Err(AthleteError::LthrNotBelowHrMax {
            lthr: 170.,
            hr_max: 166.
        })
    );
}

#[test]
fn clear_appends_the_prior_or_a_valueless_cleared_entry() {
    // ADR-0009 §2.1.3: Clear appends the prior's row when the param has one, else marks it unset; exactly one.
    for p in AthleteParam::ALL {
        for age in [None, Some(40), Some(130)] {
            let e = Entry::clear(9, p, age, 50);
            assert_eq!(
                (e.id(), e.param(), e.updated_at()),
                (9, p, 50),
                "{p} {age:?}"
            );
            match (&e, prior::prior(p, age)) {
                (Entry::Estimate(est), Some(pr)) => {
                    assert_eq!(
                        (est.value, est.sd, est.source),
                        (pr.value, pr.sd, Source::Prior)
                    );
                    assert_eq!(est.method, METHOD_PRIOR);
                }
                (Entry::Cleared(_), None) => {}
                other => panic!("{p} {age:?}: {other:?}"),
            }
        }
    }
}

#[test]
fn a_cleared_entry_resolves_unset_or_to_the_prior() {
    let rows = [
        entry(1, AthleteParam::Lthr, 165., Source::SelfReport, 10),
        cleared(2, AthleteParam::Lthr, 20),
        entry(3, AthleteParam::Weight, 70., Source::SelfReport, 10),
        cleared(4, AthleteParam::Weight, 20),
        entry(5, AthleteParam::Weight, 68., Source::SelfReport, 30),
        entry(6, AthleteParam::HrMax, 190., Source::Test, 10),
        cleared(7, AthleteParam::HrMax, 20),
    ];
    let snap = AthleteSnapshot::resolve(&rows, None);
    // Cleared and no prior: unset.
    assert_eq!(snap.lthr, None);
    // A later value after the clear is the current value again.
    assert_eq!(
        snap.weight.map(|r| (r.value, r.estimate_id)),
        Some((68., Some(5)))
    );
    // HR max cleared with no age: unset; once the age is known, the prior with no estimate id.
    assert_eq!(snap.hr_max, None);
    let aged = AthleteSnapshot::resolve(&rows, Some(40)).hr_max;
    assert_eq!(
        aged.map(|r| (r.value, r.source, r.estimate_id)),
        Some((180., Source::Prior, None))
    );
    let json = serde_json::to_value(snap).unwrap_or_default();
    assert_eq!(json.get("lthr"), Some(&serde_json::Value::Null));
}

#[test]
fn every_appendable_entry_passes_its_check() {
    // The store contract (#714): what Clear and a measurement append passes check_entry, so "trusted as stored"
    // holds for the rows resolve reads.
    let lthr_170 = AthleteSnapshot::resolve(
        &[entry(1, AthleteParam::Lthr, 170., Source::SelfReport, 1)],
        Some(60),
    );
    for current in [AthleteSnapshot::default(), lthr_170] {
        for p in AthleteParam::ALL {
            for age in [None, Some(40), Some(60), Some(130)] {
                let e = Entry::clear(2, p, age, 2);
                assert_eq!(check_entry(&e, &current), Ok(()), "{p} {age:?}");
            }
            // A Cleared only where Clear can produce one: FTP, CP and W′ always have a prior (#714 review).
            let expected = if matches!(
                p,
                AthleteParam::Ftp | AthleteParam::Cp | AthleteParam::WPrime
            ) {
                Err(AthleteError::ClearedWithPrior { param: p })
            } else {
                Ok(())
            };
            assert_eq!(check_entry(&cleared(3, p, 3), &current), expected, "{p}");
        }
    }
    let bad = entry(4, AthleteParam::Weight, 0., Source::SelfReport, 4);
    assert!(matches!(
        check_entry(&bad, &AthleteSnapshot::default()),
        Err(AthleteError::OutOfRange { .. })
    ));
}

#[test]
fn entries_round_trip_with_their_kind() {
    let c = cleared(5, AthleteParam::HrRest, 1_700_000_000);
    let json = serde_json::to_string(&c).unwrap_or_default();
    assert_eq!(
        json,
        r#"{"kind":"cleared","id":5,"param":"hr_rest","updatedAt":1700000000}"#
    );
    assert_eq!(serde_json::from_str::<Entry>(&json).ok(), Some(c));
    let e = entry(6, AthleteParam::Ftp, 280., Source::Test, 7);
    let json = serde_json::to_string(&e).unwrap_or_default();
    assert!(json.starts_with(r#"{"kind":"estimate","id":6,"#), "{json}");
    assert_eq!(serde_json::from_str::<Entry>(&json).ok(), Some(e));
}

#[test]
fn resolution_picks_the_newest_row_per_param() {
    let rows = [
        entry(1, AthleteParam::Ftp, 240., Source::SelfReport, 100),
        entry(4, AthleteParam::Ftp, 260., Source::Test, 300),
        entry(3, AthleteParam::Ftp, 250., Source::Rides, 200),
        entry(2, AthleteParam::Weight, 72., Source::SelfReport, 150),
        // Same second: the later append (larger id) wins.
        entry(5, AthleteParam::Lthr, 165., Source::SelfReport, 400),
        entry(6, AthleteParam::Lthr, 168., Source::Test, 400),
    ];
    let snap = AthleteSnapshot::resolve(&rows, None);
    let ftp = snap.ftp.map(|r| (r.value, r.source, r.estimate_id));
    assert_eq!(ftp, Some((260., Source::Test, Some(4))));
    assert_eq!(
        snap.weight.map(|r| (r.value, r.estimate_id)),
        Some((72., Some(2)))
    );
    assert_eq!(
        snap.lthr.map(|r| (r.value, r.estimate_id)),
        Some((168., Some(6)))
    );
    assert_eq!(snap.get(AthleteParam::Lthr), snap.lthr);
}

#[test]
fn a_newer_prior_row_wins_over_an_older_measurement() {
    // "Clear" appends a prior row (ADR-0009 §2.1.3): it is the newest, so it is the current value.
    let rows = [
        entry(1, AthleteParam::Ftp, 300., Source::Test, 100),
        entry(2, AthleteParam::Ftp, 250., Source::Prior, 200),
    ];
    let ftp = AthleteSnapshot::resolve(&rows, None).ftp;
    assert_eq!(
        ftp.map(|r| (r.value, r.source, r.estimate_id)),
        Some((250., Source::Prior, Some(2)))
    );
}

#[test]
fn unset_params_are_unset_never_zero() {
    let snap = AthleteSnapshot::resolve(&[], None);
    // Priors without a row: value from the prior, no estimate id.
    assert_eq!(
        snap.ftp,
        Some(Resolved {
            value: 250.,
            sd: 62.5,
            source: Source::Prior,
            estimate_id: None
        })
    );
    assert_eq!(snap.cp.map(|r| r.value), Some(250.));
    assert_eq!(snap.w_prime.map(|r| r.value), Some(20_000.));
    // No prior and no row: unset.
    for p in [
        AthleteParam::Pmax,
        AthleteParam::HrMax,
        AthleteParam::Lthr,
        AthleteParam::HrRest,
        AthleteParam::Weight,
    ] {
        assert_eq!(snap.get(p), None, "{p}");
    }
    let json = serde_json::to_value(snap).unwrap_or_default();
    for key in ["pmax", "hrMax", "lthr", "hrRest", "weight"] {
        assert_eq!(json.get(key), Some(&serde_json::Value::Null), "{key}");
    }
}

#[test]
fn hr_zones_never_come_from_a_prior_hr_max() {
    let new_rider = AthleteSnapshot::resolve(&[], Some(30));
    assert!(new_rider.hr_max.is_some());
    assert_eq!(new_rider.hr_refs(), crate::hr_zone::HrRefs::default());
    assert_eq!(new_rider.hr_zone(Some(150.)), None);

    let typed = AthleteSnapshot::resolve(
        &[entry(1, AthleteParam::HrMax, 190., Source::SelfReport, 1)],
        Some(30),
    );
    let zone = typed.hr_zone(Some(150.)).map(|z| (z.label, z.model));
    assert_eq!(zone, Some((HrZoneLabel::Z3, HrZoneModel::HrMax)));

    let both = AthleteSnapshot::resolve(
        &[
            entry(1, AthleteParam::HrMax, 190., Source::SelfReport, 1),
            entry(2, AthleteParam::Lthr, 160., Source::Test, 1),
        ],
        None,
    );
    assert_eq!(
        both.hr_zone(Some(150.)).map(|z| (z.label, z.model)),
        Some((HrZoneLabel::Z4, HrZoneModel::Lthr))
    );
}

#[test]
fn snapshot_round_trips_through_athlete_json() {
    let rows = [
        entry(7, AthleteParam::Ftp, 287., Source::Test, 10),
        entry(8, AthleteParam::WPrime, 18_500., Source::Rides, 11),
        entry(9, AthleteParam::Weight, 71.5, Source::Imported, 12),
    ];
    let snap = AthleteSnapshot::resolve(&rows, Some(41));
    let json = serde_json::to_string(&snap).unwrap_or_default();
    assert!(
        json.contains(r#""ftp":{"value":287.0,"sd":"#)
            && json.contains(r#""source":"test","estimateId":7}"#),
        "{json}"
    );
    assert!(json.contains(r#""lthr":null"#), "{json}");
    let back: AthleteSnapshot = serde_json::from_str(&json).unwrap_or_default();
    assert_eq!(back, snap);
}

#[test]
fn stored_names_parse_and_print() {
    for p in AthleteParam::ALL {
        assert_eq!(p.as_str().parse::<AthleteParam>(), Ok(p));
        assert_eq!(
            serde_json::to_value(p).ok(),
            Some(serde_json::Value::from(p.as_str()))
        );
    }
    for s in Source::ALL {
        assert_eq!(s.as_str().parse::<Source>(), Ok(s));
        assert_eq!(
            serde_json::to_value(s).ok(),
            Some(serde_json::Value::from(s.as_str()))
        );
    }
    assert_eq!(
        "vo2max".parse::<AthleteParam>(),
        Err(AthleteError::UnknownParam)
    );
    assert_eq!("guess".parse::<Source>(), Err(AthleteError::UnknownSource));
}

#[test]
fn estimate_round_trips() {
    let mut e = row(3, AthleteParam::Cp, 270., Source::Rides, 1_700_000_000);
    e.method = "cp_ols_v1".to_owned();
    e.ride_id = Some("r-1".to_owned());
    e.proposal_id = Some(12);
    e.n = 4;
    let json = serde_json::to_string(&e).unwrap_or_default();
    assert!(
        json.contains(r#""param":"cp""#) && json.contains(r#""rideId":"r-1""#),
        "{json}"
    );
    assert_eq!(serde_json::from_str::<Estimate>(&json).ok(), Some(e));
}

fn any_param() -> impl Strategy<Value = AthleteParam> {
    prop::sample::select(AthleteParam::ALL.to_vec())
}

proptest! {
    #[test]
    fn check_value_accepts_exactly_the_bounds(p in any_param(), value in prop::num::f64::ANY, frac in 0.0..1.0_f64) {
        let sd = value.abs() * frac;
        let (min, max) = p.bounds();
        let ok = value.is_finite() && sd.is_finite() && (min..=max).contains(&value) && sd > 0. && sd <= 0.5 * value;
        prop_assert_eq!(check_value(p, value, sd).is_ok(), ok);
    }

    #[test]
    fn resolution_equals_the_max_by_time_then_id(
        rows in prop::collection::vec((any_param(), 0..5_i64, 50.0..200.0_f64, prop::bool::weighted(0.2)), 0..24),
    ) {
        let rows: Vec<Entry> = rows
            .into_iter()
            .enumerate()
            .map(|(i, (p, t, v, clear))| {
                let id = i64::try_from(i).unwrap_or(i64::MAX);
                if clear { cleared(id, p, t) } else { entry(id, p, v, Source::SelfReport, t) }
            })
            .collect();
        let snap = AthleteSnapshot::resolve(&rows, None);
        for p in AthleteParam::ALL {
            let mut newest: Option<&Entry> = None;
            for e in rows.iter().filter(|e| e.param() == p) {
                if newest.is_none_or(|n| (e.updated_at(), e.id()) > (n.updated_at(), n.id())) {
                    newest = Some(e);
                }
            }
            let from_prior = || prior::prior(p, None).map(|_| None);
            let expected = match newest {
                Some(Entry::Estimate(e)) => Some(Some(e.id)),
                Some(Entry::Cleared(_)) | None => from_prior(),
            };
            prop_assert_eq!(snap.get(p).map(|r| r.estimate_id), expected);
        }
    }
}
