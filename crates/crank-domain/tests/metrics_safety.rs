//! Arbitrary IEEE-754 inputs and typed-error contracts from the round-two decision.
use crank_domain::{
    distributions::{heatmap_from, histogram_from},
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
