//! Rust equivalents and golden parity for the frozen metric reference.
use crank_domain::{
    filters::{Filter, FilterChain},
    history::MultiResSeries,
    scales::{TimeScale, time_ago_to_u, u_to_time_ago},
};
use proptest::prelude::*;

fn exact(a: f64, b: f64) {
    assert_eq!(a.to_bits(), b.to_bits());
}

fn filters() -> Vec<Filter> {
    vec![
        Filter::Sma { window_s: 3. },
        Filter::Ema { tau_s: 10. },
        Filter::Median { n: 3. },
        Filter::OneEuro {
            min_cutoff: 0.3,
            beta: 0.01,
        },
        Filter::Hold,
    ]
}
proptest! {
    #[test]
    fn filters_stay_in_input_range(xs in prop::collection::vec(-10000_i32..10000, 1..200), dt in 1_u32..100) {
        for filter in filters() {
            let mut c = FilterChain::new([filter.clone()]);
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for (k, v) in (0_u32..).zip(&xs) {
                let v = f64::from(*v);
                lo = lo.min(v); hi = hi.max(v);
                let t = f64::from(k) * f64::from(dt) / 100.;
                let y = c.push(t, Some(v)).unwrap();
                prop_assert!(y >= lo - 1e-9 && y <= hi + 1e-9, "{filter:?}: {y} outside [{lo},{hi}]");
            }
        }
    }
    #[test]
    fn reset_and_backwards_time_are_fresh(xs in prop::collection::vec(-10000_i32..10000, 1..100)) {
        for filter in filters() {
            let mut c = FilterChain::new([filter.clone()]);
            for (k,v) in (1_u32..).zip(&xs) { c.push(f64::from(k),Some(f64::from(*v))); }
            prop_assert_eq!(c.push(0.,Some(123.)),Some(123.));
            c.reset();
            prop_assert_eq!(c.push(0.,None),None);
            prop_assert_eq!(c.push(1.,Some(456.)),Some(456.));
        }
    }
}
#[test]
fn ema_time_constant_and_nulls() {
    for dt in [0.25, 1.] {
        let mut c = FilterChain::new([Filter::Ema { tau_s: 10. }]);
        c.push(0., Some(0.));
        let mut t = dt;
        let mut y = 0.;
        while t <= 10. + 1e-9 {
            y = c.push(t, Some(100.)).unwrap();
            t += dt;
        }
        assert!((y - 100. * (1. - (-1_f64).exp())).abs() < 1e-6);
        assert_eq!(c.push(11., None), None);
    }
}
#[test]
fn one_euro_jitter_and_step_response() {
    let mut c = FilterChain::new([Filter::OneEuro {
        min_cutoff: 0.3,
        beta: 0.01,
    }]);
    for i in 0..200 {
        let y = c
            .push(
                f64::from(i) * 0.25,
                Some(200. + if i % 2 == 1 { 10. } else { -10. }),
            )
            .unwrap();
        if i > 40 {
            assert!((y - 200.).abs() < 5.);
        }
    }
    let mut y = 0.;
    for i in 200..240 {
        y = c.push(f64::from(i) * 0.25, Some(400.)).unwrap();
    }
    assert!(y > 390.);
}
#[test]
fn history_tiles_and_matches_brute_force() {
    for scale in [TimeScale::Log, TimeScale::Fisheye] {
        for n in [1800, 7200] {
            let mut series = MultiResSeries::default();
            let xs: Vec<_> = (0..n)
                .map(|i| 200. + 100. * (f64::from(i) / 50.).sin())
                .collect();
            for (t, v) in (0_u32..).zip(&xs) {
                series.add(f64::from(t), Some(*v));
            }
            let buckets = series.query(
                f64::from(n) - 0.25,
                f64::from(n),
                if n == 7200 { 120 } else { 100 },
                scale,
            );
            assert!(buckets.len() > 40 && buckets.len() <= if n == 7200 { 120 } else { 100 });
            exact(buckets.iter().map(|b| b.dur).sum::<f64>(), f64::from(n));
            exact(buckets.last().unwrap().t_ago, 0.);
            for pair in buckets.windows(2) {
                exact(pair[0].t_ago, pair[1].t_ago + pair[1].dur);
            }
            for b in &buckets {
                let hi = f64::from(n) - b.t_ago;
                let lo = hi - b.dur;
                let values: Vec<_> = (0_u32..)
                    .zip(&xs)
                    .filter(|(i, _)| f64::from(*i) >= lo && f64::from(*i) < hi)
                    .map(|(_, v)| *v)
                    .collect();
                let count = values.iter().fold(0., |c, _| c + 1.);
                assert!((b.mean - values.iter().sum::<f64>() / count).abs() < 1e-6);
                if lo >= f64::from(n) - 1200. {
                    exact(b.min, values.iter().copied().fold(f64::INFINITY, f64::min));
                    exact(
                        b.max,
                        values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    );
                }
                assert!(b.min <= b.mean + 1e-9 && b.max >= b.mean - 1e-9);
            }
            if n == 7200 && matches!(scale, TimeScale::Log) {
                exact(buckets.last().unwrap().dur, 1.);
                exact(buckets[buckets.len() - 2].dur, 1.);
            }
        }
    }
}
#[test]
fn history_envelope_gaps_and_invalid_values() {
    let mut series = MultiResSeries::default();
    for i in 0..40 {
        series.add(
            f64::from(i) * 0.25,
            Some(if i % 4 == 0 { 1000. } else { 100. }),
        );
    }
    let b = series.query(9.9, 10., 10, TimeScale::Log);
    let last = b.last().unwrap();
    assert_eq!((last.min, last.mean, last.max), (100., 325., 1000.));
    series.reset();
    series.add(-1., Some(2.));
    series.add(0., None);
    series.add(1., Some(f64::NAN));
    assert!(series.query(10., 10., 10, TimeScale::Log).is_empty());
    for t in 0..100 {
        if !(40..60).contains(&t) {
            series.add(f64::from(t), Some(1.));
        }
    }
    let b = series.query(99.5, 3600., 200, TimeScale::Log);
    let covered = b.iter().map(|b| b.dur).sum::<f64>();
    assert!((75. ..=100.).contains(&covered));
    assert!(b.iter().all(|b| b.mean.to_bits() == 1_f64.to_bits()));
}
#[test]
fn scale_inverse_and_monotonicity() {
    for sc in [TimeScale::Log, TimeScale::Fisheye] {
        let mut prev = -1.;
        for i in 0..=20 {
            let u = f64::from(i) / 20.;
            let t = u_to_time_ago(u, 7200., sc);
            assert!((time_ago_to_u(t, 7200., sc) - u).abs() < 1e-9);
            assert!(t > prev);
            prev = t;
        }
    }
}

#[test]
fn median_rounds_and_clamps_sample_count() {
    for n in [-1., 0., 1.49] {
        let mut chain = FilterChain::new([Filter::Median { n }]);
        chain.push(0., Some(1.));
        assert_eq!(chain.push(1., Some(9.)), Some(9.));
    }
    let mut chain = FilterChain::new([Filter::Median { n: 1.5 }]);
    chain.push(0., Some(1.));
    assert_eq!(chain.push(1., Some(9.)), Some(5.));
}
