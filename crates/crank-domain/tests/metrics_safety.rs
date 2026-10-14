//! Arbitrary IEEE-754 inputs and typed-error contracts from the round-two decision.
use crank_domain::{
    distributions::{heatmap_from, histogram_from, typical_distribution},
    filters::{Filter, FilterChain},
    history::MultiResSeries,
    metric_error::MetricError,
    scales::TimeScale,
};
use proptest::prelude::*;

fn ieee() -> impl Strategy<Value = f64> {
    prop_oneof![4 => any::<f64>(), 1 => proptest::sample::select(vec![f64::NAN,f64::INFINITY,f64::NEG_INFINITY,1e308,-1e308,f64::from_bits(1),-f64::from_bits(1),0.,-0.])]
}
fn descriptions(p: f64) -> Vec<Filter> {
    vec![
        Filter::Sma { window_s: p },
        Filter::Ema { tau_s: p },
        Filter::Median { n: p },
        Filter::OneEuro {
            min_cutoff: p,
            beta: 0.01,
        },
        Filter::OneEuro {
            min_cutoff: 0.3,
            beta: p,
        },
        Filter::Hold,
    ]
}
proptest! {
    #![proptest_config(ProptestConfig { cases: 64, max_shrink_iters: 1024, ..ProptestConfig::default() })]
    #[test]
    fn median_arbitrary_f64(p in ieee(), samples in prop::collection::vec(ieee(),0..64)) {
        let mut probe=FilterChain::new([Filter::Median{n:500.}]).unwrap();
        prop_assert!(probe.push(0.,Some(f64::NAN)).unwrap().is_nan());
        prop_assert_eq!(probe.push(1.,Some(7.)),Some(7.));
        let built=FilterChain::new([Filter::Median{n:p}]);
        if p.is_finite() {
            let mut chain=built.unwrap();
            prop_assert!(chain.push(0.,Some(f64::NAN)).unwrap().is_nan());
            prop_assert_eq!(chain.push(1.,Some(7.)),Some(7.));
            for (t,v) in (2_u32..).zip(samples) { prop_assert!(chain.push(f64::from(t),Some(v)).is_some()); }
            chain.reset();
            prop_assert!(chain.push(0.,Some(f64::NAN)).unwrap().is_nan());
            prop_assert_eq!(chain.push(1.,Some(7.)),Some(7.));
        } else { prop_assert!(matches!(built,Err(MetricError::InvalidFilterParameter("n")))); }
    }
    #[test]
    fn every_filter_arbitrary_f64(p in ieee(), samples in prop::collection::vec((ieee(),prop::option::of(ieee())),0..32)) {
        prop_assert!(FilterChain::new([Filter::Sma{window_s:f64::NAN}]).is_err(), "non-finite filter parameter must be rejected");
        for filter in descriptions(p) {
            let hold=matches!(filter,Filter::Hold);
            let built=FilterChain::new([filter]);
            if !p.is_finite() && !hold { prop_assert!(matches!(built,Err(MetricError::InvalidFilterParameter(_)))); continue; }
            let mut chain=built.unwrap();
            let mut previous=None;
            let mut last_t=f64::NEG_INFINITY;
            for &(t,v) in &samples {
                if t<last_t { previous=None; }
                last_t=t;
                if v.is_some() { previous=v; }
                let result=chain.push(t,v);
                if hold { prop_assert_eq!(result.map(f64::to_bits),previous.map(f64::to_bits)); }
                else { prop_assert_eq!(result.is_some(),v.is_some()); }
            }
        }
    }
    #[test]
    fn histogram_arbitrary_f64(w in ieee(), xs in prop::collection::vec(ieee(),0..32)) {
        prop_assert!(histogram_from("power",&[1.],f64::INFINITY,None).is_err());
        let result=histogram_from("power",&xs,w,None);
        if !w.is_finite() || w<=0. { prop_assert!(matches!(result,Err(MetricError::InvalidBinWidth))); }
        else {
            match result {
                Ok(h) => {
                    prop_assert!(h.bin_width.is_finite() && h.bin_width>0.);
                    prop_assert!(h.bins.len()<=401);
                    prop_assert!(h.bins.iter().map(|b|b.seconds).sum::<usize>()<=xs.iter().filter(|v|v.is_finite()).count());
                    if !xs.iter().any(|v|v.is_finite()) { prop_assert!(h.bins.is_empty() && h.typical.is_none()); }
                }
                Err(e) => prop_assert!(matches!(e,MetricError::NonFiniteRange|MetricError::TooManyBins)),
            }
        }
    }
    #[test]
    fn heatmap_arbitrary_f64(xw in ieee(),yw in ieee(),xs in prop::collection::vec(ieee(),0..32),ys in prop::collection::vec(ieee(),0..32)) {
        prop_assert!(heatmap_from("power","hr",&[1.],&[1.],f64::INFINITY,1.).is_err());
        let result=heatmap_from("power","hr",&xs,&ys,xw,yw);
        if !xw.is_finite() || xw<=0. || !yw.is_finite() || yw<=0. { prop_assert!(matches!(result,Err(MetricError::InvalidBinWidth))); }
        else {
            match result {
                Ok(h) => {
                    prop_assert!(h.cols<=400 && h.rows<=400);
                    prop_assert_eq!(h.seconds.len(),h.cols*h.rows);
                    prop_assert!(h.seconds.iter().sum::<usize>()<=xs.iter().zip(&ys).filter(|(a,b)|a.is_finite() && b.is_finite()).count());
                }
                Err(e) => prop_assert!(matches!(e,MetricError::NonFiniteRange|MetricError::TooManyBins)),
            }
        }
    }
    #[test]
    fn history_query_arbitrary_f64(now in ieee(),span in ieee(),n in any::<u32>(),samples in prop::collection::vec((ieee(),prop::option::of(ieee())),0..32)) {
        let mut series=MultiResSeries::default();series.add(0.,Some(1.));
        prop_assert!(series.query(1.,-2.,10,TimeScale::Log).is_empty());
        series.add(5e307,Some(2.));
        prop_assert!(series.query(1e308,1e308,10,TimeScale::Log).len()<=10);
        for (t,v) in samples { series.add(t,v); }
        for scale in [TimeScale::Log,TimeScale::Fisheye] {
            let buckets=series.query(now,span,n,scale);
            if !now.is_finite() || !span.is_finite() || now<0. || span<=0. { prop_assert!(buckets.is_empty()); }
            prop_assert!(buckets.len()<=2000);
            for b in buckets {
                prop_assert!(b.dur.is_finite() && b.dur>0.);
                prop_assert!(b.t_ago.is_finite() && b.t_ago>=0.);
                prop_assert!(b.min.is_finite() && b.max.is_finite() && b.min<=b.max);
            }
        }
    }
}
#[test]
fn median_nan_stress_and_window_eviction() {
    let mut chain = FilterChain::new([Filter::Median { n: 500. }]).unwrap();
    let mut results = Vec::new();
    for t in 0..4000 {
        let v = if t % 7 == 0 {
            f64::NAN
        } else {
            f64::from((t * 31) % 97)
        };
        results.push(chain.push(f64::from(t), Some(v)).unwrap());
    }
    assert!(results.iter().skip(1).all(|v| v.is_finite()));
    for t in 4000..4500 {
        chain.push(f64::from(t), Some(f64::NAN));
    }
    assert!(chain.push(4500., Some(f64::NAN)).unwrap().is_nan());
}
#[test]
fn histogram_overflow_range_is_typed_error() {
    assert!(matches!(
        histogram_from("power", &[-1e308, 1e308], 1., None),
        Err(MetricError::NonFiniteRange)
    ));
}
#[test]
fn rounded_negative_indices_match_ts() {
    let v = 975.331_168_680_309;
    let width = 2.316_701_113_254_891;
    assert!(
        histogram_from("power", &[v], width, None)
            .unwrap()
            .bins
            .is_empty()
    );
    assert!(
        heatmap_from("power", "hr", &[v], &[1.], width, 1.)
            .unwrap()
            .seconds
            .is_empty()
    );
    // When only the first sample rounds to -1, TS omits it rather than using bin 0.
    let h = histogram_from("power", &[v, v + width * 2.], width, None).unwrap();
    assert_eq!(h.bins.iter().map(|b| b.seconds).sum::<usize>(), 1);
}

// ---- #148, #150: intentional divergences from frozen `shared/` (see DIVERGENCES.md) -----

/// #148: TS normalises finite zero/negative widths to one; Rust returns the typed error, also
/// on empty input. Frozen width-zero vectors are checked for the same error in
/// `distribution_vectors`.
#[test]
fn nonpositive_bin_widths_are_typed_errors_not_normalised() {
    for width in [0., -0., -1., f64::MIN, f64::NAN, f64::INFINITY] {
        for series in [&[][..], &[1., 2., 3.][..]] {
            assert!(
                matches!(
                    histogram_from("power", series, width, None),
                    Err(MetricError::InvalidBinWidth)
                ),
                "histogram {width} {series:?}"
            );
            assert!(
                matches!(
                    heatmap_from("power", "hr", series, series, width, 1.),
                    Err(MetricError::InvalidBinWidth)
                ),
                "heatmap x {width} {series:?}"
            );
            assert!(
                matches!(
                    heatmap_from("power", "hr", series, series, 1., width),
                    Err(MetricError::InvalidBinWidth)
                ),
                "heatmap y {width} {series:?}"
            );
        }
    }
    // The boundary stays valid: the smallest positive width is accepted or fails on range,
    // never as a bad width.
    assert!(!matches!(
        histogram_from("power", &[], f64::from_bits(1), None),
        Err(MetricError::InvalidBinWidth)
    ));
}

/// #150: the reference loops with `s++` once a slot index is too large for `+ 1` to advance
/// it. Rust walks the retained slots instead, so a query at an extreme finite timestamp
/// terminates, is bounded by the bucket cap and only yields well-formed buckets.
#[test]
fn history_query_terminates_at_extreme_finite_timestamps() {
    for t in [1e308, f64::MAX, 9.007_199_254_740_993e15] {
        let mut series = MultiResSeries::default();
        series.add(0., Some(1.));
        series.add(t, Some(2.));
        for (now, span) in [(t, t), (t, 1e300), (t, 60.), (f64::MAX, f64::MAX)] {
            for scale in [TimeScale::Log, TimeScale::Fisheye] {
                let buckets = series.query(now, span, 50, scale);
                assert!(buckets.len() <= 50, "{t} {now} {span}");
                for b in buckets {
                    assert!(b.dur.is_finite() && b.dur > 0., "{t} {now} {span}");
                    assert!(b.min <= b.max);
                }
            }
        }
    }
}

// ---- #155 -------------------------------------------------------------------------------

const TYPICAL_METRICS: [&str; 5] = ["power", "power_30s", "pct_ftp", "hr", "cadence"];
const POWER_METRICS: [&str; 4] = ["power", "power_3s", "power_10s", "power_30s"];
const FTP_FREE_METRICS: [&str; 3] = ["pct_ftp", "hr", "cadence"];
const BAD_FTPS: [f64; 10] = [
    f64::NAN,
    f64::INFINITY,
    f64::NEG_INFINITY,
    0.,
    -0.,
    -1.,
    -280.,
    f64::MIN,
    f64::MAX,
    f64::from_bits(1),
];

/// A nonfinite or nonpositive FTP is a typed error for every power metric (TS propagated NaN
/// into the distribution).
#[test]
fn typical_distribution_rejects_invalid_ftp_for_power_metrics() {
    for ftp in BAD_FTPS {
        for metric in POWER_METRICS {
            assert!(
                matches!(
                    typical_distribution(metric, ftp),
                    Err(MetricError::InvalidMetricParameter("ftp"))
                ),
                "{metric} at {ftp}"
            );
        }
    }
}

/// `hr`, `cadence` and `pct_ftp` do not depend on FTP, so a bad FTP must not reject them, and
/// their result must not depend on it (#186 item 3; TS returns the same finite result).
#[test]
fn typical_distribution_ignores_ftp_for_ftp_free_metrics() {
    for metric in FTP_FREE_METRICS {
        let reference = typical_distribution(metric, 280.).unwrap().unwrap();
        for ftp in BAD_FTPS {
            let f = typical_distribution(metric, ftp)
                .unwrap_or_else(|e| panic!("{metric} at {ftp}: {e:?}"))
                .expect("known metric");
            for (lo, hi) in [(-1000., 1000.), (60., 90.), (100., 140.)] {
                assert_eq!(f(lo, hi).to_bits(), reference(lo, hi).to_bits(), "{metric}");
            }
        }
    }
    // An unsupported metric is `Ok(None)` regardless of FTP, since no FTP is consulted.
    for ftp in BAD_FTPS {
        assert!(typical_distribution("unknown", ftp).unwrap().is_none());
    }
}

#[test]
fn typical_distribution_valid_ftp_covers_known_metrics_only() {
    for metric in TYPICAL_METRICS {
        let f = typical_distribution(metric, 280.).unwrap().unwrap();
        let mass = f(-1000., 1000.);
        assert!((mass - 1.).abs() < 1e-6, "{metric}: {mass}");
    }
    assert!(typical_distribution("unknown", 280.).unwrap().is_none());
}

proptest! {
 #![proptest_config(ProptestConfig{cases:64,max_shrink_iters:1024,..ProptestConfig::default()})]
 #[test]
 fn typical_distribution_error_kind_per_ftp_class(ftp in ieee(), index in 0..TYPICAL_METRICS.len(), lo in -1e3..1e3_f64, width in 0.0..1e3_f64) {
  let result = typical_distribution(TYPICAL_METRICS[index], ftp);
  // Every power component must be a finite mean with a nonzero finite deviation.
  let usable = ftp.is_finite() && ftp > 0. && 1.05 * ftp < f64::MAX && 0.07 * ftp > 0.;
  let uses_ftp = POWER_METRICS.contains(&TYPICAL_METRICS[index]);
  if usable || !uses_ftp {
   let share = result.unwrap().unwrap()(lo, lo + width);
   prop_assert!((0. ..=1. + 1e-9).contains(&share), "{share}");
  } else {
   prop_assert!(matches!(result, Err(MetricError::InvalidMetricParameter("ftp"))));
  }
 }
 /// The incrementally sorted window gives exactly the result of sorting the window afresh,
 /// including NaN slots, signed zeros and infinities.
 #[test]
 fn median_matches_a_fresh_sort_of_the_window(
  n in 1..9_u32,
  samples in prop::collection::vec(
   prop_oneof![
    4 => prop::option::of(-50.0..50.0_f64),
    1 => prop::option::of(proptest::sample::select(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0., -0., 1e308, -1e308])),
   ],
   0..80,
  )
 ) {
  let mut chain = FilterChain::new([Filter::Median { n: f64::from(n) }]).unwrap();
  let mut window: Vec<f64> = Vec::new();
  for (t, sample) in samples.into_iter().enumerate() {
   let got = chain.push(f64::from(u32::try_from(t).unwrap()), sample);
   let Some(v) = sample else { prop_assert!(got.is_none()); continue; };
   window.push(v);
   if window.len() > n as usize { window.remove(0); }
   let mut sorted: Vec<f64> = window.iter().copied().filter(|x| !x.is_nan()).collect();
   sorted.sort_by(f64::total_cmp);
   let expected = match sorted.len() {
    0 => f64::NAN,
    len if len % 2 == 1 => sorted[len / 2],
    len => {
     // TS sums then halves, so a pair of huge finite samples overflows to infinity.
     let sum = sorted[len / 2 - 1] + sorted[len / 2];
     sum / 2.
    }
   };
   let got = got.unwrap();
   prop_assert!(got.to_bits() == expected.to_bits() || (got.is_nan() && expected.is_nan()), "{got} != {expected}");
  }
 }
}

/// A heatmap clamps an over-wide axis to 400 bins, with the last bin as the overflow bin, where
/// a histogram widens and then errors. Both are TS contracts, so the two intentionally differ.
#[test]
fn heatmap_axis_cap_is_a_clamp_not_an_error() {
    let h = heatmap_from("power", "hr", &[0., 1e6, 5.], &[0., 0., 0.], 1., 1.).unwrap();
    assert_eq!((h.cols, h.rows), (400, 1));
    assert_eq!(h.seconds.len(), 400);
    assert_eq!(h.seconds.iter().sum::<usize>(), 3);
    assert_eq!(h.seconds[0], 1);
    assert_eq!(h.seconds[5], 1);
    assert_eq!(h.seconds[399], 1);
    // The same range on a histogram is widened to fit (still no error).
    let hist = histogram_from("power", &[0., 1e6], 1., None).unwrap();
    assert!(hist.bins.len() <= 401 && hist.bin_width > 1.);
}

/// DIVERGENCES.md claims finite negative filter parameters are clamped as in TS. Expected values
/// are the outputs of `shared/src/filters.ts` (`createFilterChain`) for the same samples, so the
/// clamp is pinned by value, not just by "produces something".
#[test]
fn finite_negative_filter_parameters_are_clamped_with_ts_values() {
    const T: [f64; 6] = [0., 1., 2.5, 3., 7., 7.5];
    const V: [f64; 6] = [10., 20., 15., 40., 5., 30.];
    let run = |f: Filter| -> Vec<f64> {
        let mut chain = FilterChain::new([f]).unwrap();
        T.iter()
            .zip(V)
            .map(|(&t, v)| chain.push(t, Some(v)).unwrap())
            .collect()
    };
    // The SMA `window_s.max(0.)` clamp is an equivalent mutant (#222): `FilterChain::push` resets
    // the stages whenever time steps backwards, so every earlier sample has `t_old <= t` and is
    // evicted for any window <= 0. The SMA case below therefore pins the passthrough, not the clamp.
    // window 0, tau 0 and n 1 all pass the input through; negatives clamp to exactly those.
    for f in [
        Filter::Sma { window_s: -5. },
        Filter::Ema { tau_s: -5. },
        Filter::Median { n: -3. },
        Filter::Median { n: 0.4 },
    ] {
        assert_eq!(run(f.clone()), V, "{f:?}");
    }
    // minCutoff -1 clamps to 1e-3 and beta -2 to 0 (not to a passthrough, so the value is a real check).
    let expected = [
        10.,
        10.062_439_533_909_748,
        10.108_540_454_034_557,
        10.202_153_150_788_833,
        10.074_614_185_625_913,
        10.137_015_591_521_957,
    ];
    let got = run(Filter::OneEuro {
        min_cutoff: -1.,
        beta: -2.,
    });
    for (g, e) in got.iter().zip(expected) {
        assert!((g - e).abs() <= 1e-12, "{got:?}");
    }
    let clamped = run(Filter::OneEuro {
        min_cutoff: 1e-3,
        beta: 0.,
    });
    assert_eq!(got, clamped);
}
