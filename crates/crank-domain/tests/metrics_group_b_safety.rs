//! Bounded arbitrary IEEE-754 requests, typed errors, and finite TS edge cases.
use crank_domain::{hr_recovery::*, hrv::*, metric_error::MetricError, wbal::*};
use proptest::prelude::*;
fn ieee() -> impl Strategy<Value = f64> {
    prop_oneof![4=>any::<f64>(),1=>proptest::sample::select(vec![f64::NAN,f64::INFINITY,f64::NEG_INFINITY,1e308,-1e308,f64::from_bits(1),-f64::from_bits(1),0.,-0.])]
}
fn prior(p: f64) -> RecoveryPrior {
    RecoveryPrior {
        tau_s: p,
        tau_log_sd: 0.35,
        hr_inf: 110.,
        hr_inf_sd: 8.,
    }
}
fn input(p: f64, observed: Vec<Sample>) -> RecoveryInput {
    RecoveryInput {
        started_t: 0.,
        min_s: 60.,
        max_s: p,
        target_bpm: 120.,
        observed,
        now: 20.,
        usual_tau_s: 50.,
        prior_hr_inf: 105.,
        prior_hr_inf_sd: None,
        tau_log_sd: None,
        peak_search_s: None,
    }
}
fn fit_error(e: &MetricError) -> bool {
    matches!(
        e,
        MetricError::InvalidMetricParameter(_)
            | MetricError::NonFiniteCalculation
            | MetricError::ComputationLimit
    )
}
proptest! {
 #![proptest_config(ProptestConfig{cases:64,max_shrink_iters:1024,..ProptestConfig::default()})]
 #[test]
 fn wbal_ieee(w in ieee(),p in ieee(),dt in ieee(),cp in ieee(),w0 in ieee(),xs in prop::collection::vec(ieee(),0..64)) {
  let result=wbal_step(w,p,dt,cp,w0);let series=wbal_series(&xs,dt,cp,w0);
  if [dt,cp,w0].iter().all(|v|v.is_finite()) {
   prop_assert!(result.is_ok());prop_assert_eq!(series.unwrap().len(),xs.len());
  } else {prop_assert!(matches!(result,Err(MetricError::InvalidMetricParameter(_))));prop_assert!(matches!(series,Err(MetricError::InvalidMetricParameter(_))));}
 }
 #[test]
 fn hrv_ieee(xs in prop::collection::vec(ieee(),0..128),lo in any::<u32>(),hi in any::<u32>(),window in any::<u32>()) {
  let _=rmssd(&xs);
  let d=detrend_rr(&xs,window);
  if window<=i32::MAX.unsigned_abs() {prop_assert_eq!(d.unwrap().len(),xs.len());}
  else {prop_assert!(matches!(d,Err(MetricError::InvalidMetricParameter("detrendWindow"))));}
  let a=dfa_alpha1(&xs,lo,hi,window);
  match a {Ok(_)=>{},Err(e)=>prop_assert!(matches!(e,MetricError::InvalidMetricParameter(_)|MetricError::ComputationLimit))}
  let _=dfa_alpha1(&xs,4,16,25).unwrap();
 }
 #[test]
 fn recovery_ieee(p in ieee(),ts in prop::collection::vec(ieee(),0..32),ys in prop::collection::vec(ieee(),0..32),g in any::<u32>()) {
  for config in [FitOptions::default(),FitOptions{tau_min:Some(p),tau_max:Some(p),grid_n:Some(g),sigma:Some(p)}] {
   let result=fit_mono_exp(&ts,&ys,p,&prior(p),&config);
   match result {
    Ok(None)=>prop_assert!(ts.len()<3 || ts.len()!=ys.len()),
    Ok(Some(f))=>{
     prop_assert!(f.tau_s.is_finite() && f.hr_inf.is_finite() && f.amp.is_finite());
     prop_assert!(f.grid.len()<=4096);
     match predict_at(&f,p) {Ok(b)=>prop_assert!(b.mean.is_finite() && b.lo<=b.hi),Err(e)=>prop_assert!(fit_error(&e))}
    }
    Err(e)=>prop_assert!(fit_error(&e)),
   }
  }
 }
 #[test]
 fn recovery_state_ieee(p in ieee(),samples in prop::collection::vec((ieee(),prop::option::of(ieee())),0..32)) {
  let observed=samples.into_iter().map(|(t,v)|Sample{t,v}).collect();
  match build_hr_recovery_state(&input(p,observed)) {
   Ok(s)=>{if let Some(ps)=s.predicted {prop_assert!(ps.len()<=92);prop_assert!(ps.iter().all(|x|x.t.is_finite() && x.band.mean.is_finite()));}}
   Err(e)=>prop_assert!(fit_error(&e)),
  }
 }
 #[test]
 fn prediction_ieee(t in ieee(),a in ieee(),b in ieee(),tau in ieee(),weight in ieee(),cov in ieee()) {
  let fit=MonoExpFit{hr_inf:0.,amp:0.,tau_s:50.,sigma:1.5,n:4,t0:0.,grid:vec![TauGridPoint{tau,w:weight,a,b,c00:cov,c01:cov,c11:cov}]};
  match predict_at(&fit,t) {Ok(p)=>prop_assert!(p.mean.is_finite() && p.lo.is_finite() && p.hi.is_finite()),Err(e)=>prop_assert!(fit_error(&e))}
 }
 #[test]
 fn every_recovery_parameter_ieee(p in ieee(),which in 0..9_u8) {
  let mut pri=prior(50.);let mut opts=FitOptions::default();let mut t0=0.;
  match which {0=>pri.tau_s=p,1=>pri.tau_log_sd=p,2=>pri.hr_inf=p,3=>pri.hr_inf_sd=p,4=>opts.sigma=Some(p),5=>opts.tau_min=Some(p),6=>opts.tau_max=Some(p),_=>t0=p}
  let result=fit_mono_exp(&[0.,1.,2.,3.],&[160.,158.,156.,154.],t0,&pri,&opts);
  if p.is_finite() {match result {Ok(Some(f))=>prop_assert!(f.tau_s.is_finite()),Err(e)=>prop_assert!(fit_error(&e)),Ok(None)=>prop_assert!(false)}}
  else {prop_assert!(matches!(result,Err(MetricError::InvalidMetricParameter(_))));}
 }
}
#[test]
fn finite_invalid_parameters_keep_ts_results() {
    assert!((wbal_step(100., 330., 10., 0., 20000.).unwrap() + 3200.).abs() < 1e-12);
    assert!((wbal_step(100., 330., -1., -5., -20000.).unwrap() - 435.).abs() < 1e-12);
    assert!(wbal_step(0., 0., 0., 0., 0.).unwrap().is_nan());
    let ts = [0., 1., 2., 3.];
    let ys = [150., 149., 148., 147.];
    let mut negative = prior(50.);
    negative.hr_inf_sd = -8.;
    negative.tau_log_sd = -0.35;
    let a = fit_mono_exp(
        &ts,
        &ys,
        0.,
        &prior(50.),
        &FitOptions {
            sigma: Some(1.5),
            ..FitOptions::default()
        },
    )
    .unwrap()
    .unwrap();
    let b = fit_mono_exp(
        &ts,
        &ys,
        0.,
        &negative,
        &FitOptions {
            sigma: Some(-1.5),
            ..FitOptions::default()
        },
    )
    .unwrap()
    .unwrap();
    assert!((a.tau_s - b.tau_s).abs() < 1e-12);
    assert!((a.hr_inf - b.hr_inf).abs() < 1e-12);
    assert!(dfa_alpha1(&[800.; 120], 16, 4, 25).unwrap().is_none());
    assert!(dfa_alpha1(&[800.; 120], 1, 16, 25).unwrap().is_none());
    assert_eq!(detrend_rr(&[800., 810.], 0).unwrap(), vec![0., 0.]);
}
#[test]
fn bounds_and_overflow_are_typed_and_zero_is_canonical_in_messages() {
    let ts = [0., 1., 2., 3.];
    let ys = [150., 149., 148., 147.];
    for g in [0, 1, u32::MAX] {
        assert!(matches!(
            fit_mono_exp(
                &ts,
                &ys,
                0.,
                &prior(50.),
                &FitOptions {
                    grid_n: Some(g),
                    ..FitOptions::default()
                }
            ),
            Err(MetricError::ComputationLimit)
        ));
    }
    assert!(matches!(
        fit_mono_exp(&ts, &[1e308; 4], 0., &prior(50.), &FitOptions::default()),
        Err(MetricError::NonFiniteCalculation)
    ));
    let observed = (0..10)
        .map(|i| Sample {
            t: f64::from(i),
            v: Some(160. - f64::from(i)),
        })
        .collect();
    assert!(matches!(
        build_hr_recovery_state(&input(1e308, observed)),
        Err(MetricError::ComputationLimit)
    ));
    let error = fit_mono_exp(&ts, &ys, 0., &prior(-0.), &FitOptions::default()).unwrap_err();
    assert_eq!(error.to_string(), "invalid metric parameter: tauS");
    assert!(!error.to_string().contains("-0"));
    assert!(detrend_rr(&[], u32::MAX).is_err());
    assert!(dfa_alpha1(&[0.; 120], 0, 16, 25).is_err());
}
#[test]
fn large_series_and_equal_peaks_are_bounded() {
    let rr = vec![800.; 100_000];
    assert!(rmssd(&rr).unwrap().abs() < 1e-12);
    assert!(dfa_alpha1(&rr, 4, 16, 25).unwrap().is_none());
    assert_eq!(
        wbal_series(&vec![280.; 100_000], 1., 280., 20000.)
            .unwrap()
            .len(),
        100_000
    );
    assert!(matches!(
        dfa_alpha1(&rr, 4, 5000, 25),
        Err(MetricError::ComputationLimit)
    ));
    let obs = (0..10)
        .map(|i| Sample {
            t: f64::from(i),
            v: Some(150.),
        })
        .collect();
    let state = build_hr_recovery_state(&input(240., obs)).unwrap();
    assert!((state.peak_t - 9.).abs() < 1e-12);
    assert!(state.fit.is_none());
}

#[test]
fn nonfinite_configuration_is_rejected_before_insufficient_data() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let priors = [
            prior(value),
            RecoveryPrior {
                tau_log_sd: value,
                ..prior(50.)
            },
            RecoveryPrior {
                hr_inf: value,
                ..prior(50.)
            },
            RecoveryPrior {
                hr_inf_sd: value,
                ..prior(50.)
            },
        ];
        for p in priors {
            assert!(matches!(
                fit_mono_exp(&[], &[], 0., &p, &FitOptions::default()),
                Err(MetricError::InvalidMetricParameter(_))
            ));
        }
        for options in [
            FitOptions {
                tau_min: Some(value),
                ..FitOptions::default()
            },
            FitOptions {
                tau_max: Some(value),
                ..FitOptions::default()
            },
            FitOptions {
                sigma: Some(value),
                ..FitOptions::default()
            },
        ] {
            assert!(matches!(
                fit_mono_exp(&[0., 1.], &[150., 149.], 0., &prior(50.), &options),
                Err(MetricError::InvalidMetricParameter(_))
            ));
        }
        for field in 0..4 {
            let mut inp = input(240., Vec::new());
            match field {
                0 => inp.usual_tau_s = value,
                1 => inp.prior_hr_inf = value,
                2 => inp.prior_hr_inf_sd = Some(value),
                _ => inp.tau_log_sd = Some(value),
            }
            assert!(matches!(
                build_hr_recovery_state(&inp),
                Err(MetricError::InvalidMetricParameter(_))
            ));
        }
    }
}

// ---- #169: the error KIND per input class -------------------------------------------------
// The IEEE tests above accept any of the three recovery error kinds for any input. These pin the
// kind to the class the input belongs to, so a finite, well-conditioned request can never be
// reported as `NonFiniteCalculation` and a bad parameter is never reported as a work bound.

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1e-9 * expected.abs().max(1.),
        "{actual} != {expected}"
    );
}
fn sampled(len: usize, gap: f64, spread: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let ts: Vec<f64> = (0..len)
        .map(|i| f64::from(u32::try_from(i).expect("small index")) * gap)
        .collect();
    let ys = spread.iter().cycle().take(len).copied().collect();
    (ts, ys)
}
fn bounded_prior() -> impl Strategy<Value = RecoveryPrior> {
    (
        5.0..300.0_f64,
        0.05..2.0_f64,
        40.0..200.0_f64,
        1.0..30.0_f64,
        any::<bool>(),
    )
        .prop_map(
            |(tau_s, tau_log_sd, hr_inf, hr_inf_sd, negative)| RecoveryPrior {
                tau_s,
                // Negative nonzero deviations keep TS's squared-prior semantics.
                tau_log_sd: if negative { -tau_log_sd } else { tau_log_sd },
                hr_inf,
                hr_inf_sd,
            },
        )
}
fn bounded_options() -> impl Strategy<Value = FitOptions> {
    (
        2.0..20.0_f64,
        21.0..600.0_f64,
        2..200_u32,
        proptest::option::of(0.5..10.0_f64),
    )
        .prop_map(|(tau_min, tau_max, grid_n, sigma)| FitOptions {
            tau_min: Some(tau_min),
            tau_max: Some(tau_max),
            grid_n: Some(grid_n),
            sigma,
        })
}
proptest! {
 #![proptest_config(ProptestConfig{cases:64,max_shrink_iters:1024,..ProptestConfig::default()})]
 /// Class: finite, bounded, well-conditioned. Never an error, and the fit/prediction are finite.
 #[test]
 fn bounded_finite_fit_is_never_an_error(
  p in bounded_prior(), o in bounded_options(),
  len in 3..40_usize, gap in 0.5..5.0_f64, ys in prop::collection::vec(60.0..200.0_f64, 3..40)
 ) {
  let (ts, _) = sampled(len, gap, &ys);
  let ys: Vec<f64> = ys.iter().cycle().take(len).copied().collect();
  let fit = fit_mono_exp(&ts, &ys, ts[0], &p, &o).unwrap().unwrap();
  prop_assert!(fit.tau_s.is_finite() && fit.hr_inf.is_finite() && fit.amp.is_finite());
  prop_assert!(!fit.grid.is_empty() && fit.grid.iter().all(|g| g.w.is_finite()));
  let band = predict_at(&fit, ts[len - 1] + 30.).unwrap();
  prop_assert!(band.mean.is_finite() && band.lo <= band.hi);
 }
 /// Class: too few or mismatched observations with a valid configuration: `Ok(None)`.
 #[test]
 fn insufficient_data_with_valid_configuration_is_none(
  p in bounded_prior(), o in bounded_options(), len in 0..3_usize, extra in 1..4_usize
 ) {
  let (ts, ys) = sampled(len, 1., &[150.]);
  prop_assert!(fit_mono_exp(&ts, &ys, 0., &p, &o).unwrap().is_none());
  let (ts, ys) = sampled(len + 3, 1., &[150.]);
  prop_assert!(fit_mono_exp(&ts, &ys[..ys.len() - extra.min(ys.len())], 0., &p, &o).unwrap().is_none());
 }
 /// Class: a finite parameter outside its domain is `InvalidMetricParameter` naming it,
 /// checked before the data, so also with no observations.
 #[test]
 fn out_of_domain_parameter_is_named(
  p in bounded_prior(), bad in prop_oneof![Just(0.), Just(-0.), -1e6..0.0_f64], which in 0..6_u8, empty in any::<bool>()
 ) {
  let mut prior = p; let mut o = FitOptions::default();
  let name = match which {
   0 => { prior.tau_s = bad; "tauS" }
   1 => { o.tau_min = Some(bad); "tauMin" }
   2 => { o.tau_max = Some(bad); "tauMax" }
   3 => { prior.hr_inf_sd = 0.; "hrInfSd" }
   4 => { prior.tau_log_sd = 0.; "tauLogSd" }
   _ => { o.sigma = Some(0.); "sigma" }
  };
  let (ts, ys) = if empty { (vec![], vec![]) } else { sampled(8, 1., &[150., 140.]) };
  prop_assert_eq!(fit_mono_exp(&ts, &ys, 0., &prior, &o).unwrap_err(), MetricError::InvalidMetricParameter(name));
 }
 /// Class: grid count outside 2..=4096 is `ComputationLimit`, never a parameter error.
 #[test]
 fn grid_outside_bounds_is_a_computation_limit(
  p in bounded_prior(), grid_n in prop_oneof![0..2_u32, 4097..=u32::MAX]
 ) {
  let (ts, ys) = sampled(8, 1., &[150., 140.]);
  let o = FitOptions { grid_n: Some(grid_n), ..FitOptions::default() };
  prop_assert_eq!(fit_mono_exp(&ts, &ys, 0., &p, &o).unwrap_err(), MetricError::ComputationLimit);
 }
 /// Class: a nonfinite observation with a valid configuration is `NonFiniteCalculation`.
 #[test]
 fn nonfinite_observation_is_a_nonfinite_calculation(
  p in bounded_prior(), at in 0..8_usize, in_ts in any::<bool>(),
  bad in proptest::sample::select(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY])
 ) {
  let (mut ts, mut ys) = sampled(8, 1., &[150., 140., 130.]);
  if in_ts { ts[at] = bad; } else { ys[at] = bad; }
  prop_assert_eq!(
   fit_mono_exp(&ts, &ys, 0., &p, &FitOptions::default()).unwrap_err(),
   MetricError::NonFiniteCalculation
  );
 }
 /// Class: finite observations whose squares overflow `f64` are `NonFiniteCalculation`
 /// (documented finite-input divergence from TS's nonfinite fit), never a parameter error.
 #[test]
 fn squares_that_overflow_are_a_nonfinite_calculation(
  p in bounded_prior(), magnitude in 1e155..1e300_f64, negative in any::<bool>()
 ) {
  let y = if negative { -magnitude } else { magnitude };
  let (ts, ys) = sampled(6, 1., &[y]);
  prop_assert_eq!(
   fit_mono_exp(&ts, &ys, 0., &p, &FitOptions::default()).unwrap_err(),
   MetricError::NonFiniteCalculation
  );
 }
 /// Class: bounded recovery block. Finite configuration and observations: `Ok`, with a finite
 /// prediction whenever a fit exists.
 #[test]
 fn bounded_recovery_state_is_never_an_error(
  peak in 120.0..200.0_f64, drop in 0.5..4.0_f64, len in 1..60_usize, max_s in 60.0..3600.0_f64,
  min_s in 0.0..120.0_f64, target in 80.0..150.0_f64, now in 0.0..400.0_f64
 ) {
  let observed: Vec<Sample> = (0..u32::try_from(len).unwrap()).map(|i| Sample { t: f64::from(i), v: Some((peak - drop * f64::from(i)).max(50.)) }).collect();
  let mut inp = input(max_s, observed);
  inp.min_s = min_s; inp.target_bpm = target; inp.now = now;
  let state = build_hr_recovery_state(&inp).unwrap();
  if let Some(predicted) = &state.predicted {
   prop_assert!(!predicted.is_empty() && predicted.len() <= 92);
   prop_assert!(state.fit.is_some() && state.ready_in_s.is_some());
  }
 }
 /// Class: nonfinite recovery configuration is `InvalidMetricParameter`, before the data.
 #[test]
 fn nonfinite_recovery_configuration_is_a_parameter_error(
  bad in proptest::sample::select(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY]), which in 0..6_u8
 ) {
  let mut inp = input(240., Vec::new());
  match which { 0 => inp.started_t = bad, 1 => inp.min_s = bad, 2 => inp.max_s = bad,
   3 => inp.target_bpm = bad, 4 => inp.now = bad, _ => inp.peak_search_s = Some(bad) }
  prop_assert!(matches!(build_hr_recovery_state(&inp), Err(MetricError::InvalidMetricParameter(_))));
 }
 /// Class: a nonfinite observation with valid configuration is `NonFiniteCalculation`.
 #[test]
 fn nonfinite_recovery_observation_is_a_nonfinite_calculation(
  at in 0..6_usize, in_time in any::<bool>(),
  bad in proptest::sample::select(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY])
 ) {
  let mut observed: Vec<Sample> = (0..6).map(|i| Sample { t: f64::from(i), v: Some(160. - f64::from(i)) }).collect();
  if in_time { observed[at].t = bad; } else { observed[at].v = Some(bad); }
  prop_assert_eq!(build_hr_recovery_state(&input(240., observed)).unwrap_err(), MetricError::NonFiniteCalculation);
 }
 /// Class: a finite horizon above one day is `ComputationLimit` once a fit exists.
 #[test]
 fn horizon_above_one_day_is_a_computation_limit(max_s in 90_000.0..1e9_f64) {
  let observed = (0..10).map(|i| Sample { t: f64::from(i), v: Some(160. - f64::from(i)) }).collect();
  prop_assert_eq!(build_hr_recovery_state(&input(max_s, observed)).unwrap_err(), MetricError::ComputationLimit);
 }
 /// Class: prediction. A finite fit and time give a finite band; a nonfinite time is a
 /// parameter error and a nonfinite grid component a nonfinite calculation.
 #[test]
 fn prediction_error_kind_per_class(
  t in -1e4..1e4_f64, tau in 1.0..500.0_f64, a in 40.0..200.0_f64, b in -100.0..100.0_f64,
  bad in proptest::sample::select(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY])
 ) {
  let point = TauGridPoint { tau, w: 1., a, b, c00: 1., c01: 0., c11: 1. };
  let fit = |grid| MonoExpFit { hr_inf: a, amp: b, tau_s: tau, sigma: 1.5, n: 4, t0: 0., grid };
  let band = predict_at(&fit(vec![point.clone()]), t.max(0.)).unwrap();
  prop_assert!(band.mean.is_finite() && band.lo.is_finite() && band.hi.is_finite());
  prop_assert_eq!(predict_at(&fit(vec![point.clone()]), bad).unwrap_err(), MetricError::InvalidMetricParameter("t"));
  let broken = TauGridPoint { b: bad, ..point };
  prop_assert_eq!(predict_at(&fit(vec![broken]), 0.).unwrap_err(), MetricError::NonFiniteCalculation);
 }
 /// Class: DFA parameters. Too little data is `Ok(None)` for any box size (TS order); with
 /// enough data each out-of-domain parameter has its own kind.
 #[test]
 fn dfa_error_kind_per_class(
  len in 0..64_usize, max_box in 1..16_u32, min_box in 0..16_u32, window in any::<u32>()
 ) {
  let rr: Vec<f64> = (0..len).map(|i| 800. + f64::from(u8::try_from(i % 7).unwrap())).collect();
  let result = dfa_alpha1(&rr, min_box, max_box, window);
  if len < 3 * max_box as usize {
   prop_assert_eq!(result.unwrap(), None);
  } else if min_box == 0 {
   prop_assert_eq!(result.unwrap_err(), MetricError::InvalidMetricParameter("boxSize"));
  } else if max_box < min_box {
   prop_assert_eq!(result.unwrap(), None);
  } else if window > i32::MAX.unsigned_abs() {
   prop_assert_eq!(result.unwrap_err(), MetricError::InvalidMetricParameter("detrendWindow"));
  } else {
   prop_assert!(result.is_ok());
  }
 }
 /// Class: a box size above 4096 is `ComputationLimit` once there is data to run on.
 #[test]
 fn oversized_box_is_a_computation_limit(max_box in 4097..20_000_u32, min_box in 1..4097_u32) {
  let rr = vec![800.; 3 * max_box as usize];
  prop_assert_eq!(dfa_alpha1(&rr, min_box, max_box, 0).unwrap_err(), MetricError::ComputationLimit);
 }
}

/// #169 item 1: TS gives a grid point whose cost overflows to +inf zero weight, so with
/// `tau_s` = 10 and `tau_log_sd` = 1e-300 only the first grid point (ln τ = ln `tau_s`) survives.
/// Reference captured from the frozen `fitMonoExp` with Vitest, 2026-10-13.
#[test]
fn overflowed_cost_grid_points_get_zero_weight_as_in_ts() {
    let prior = RecoveryPrior {
        tau_s: 10.,
        tau_log_sd: 1e-300,
        hr_inf: 110.,
        hr_inf_sd: 8.,
    };
    let ts = [0., 1., 2., 3.];
    let ys = [150., 149., 148., 147.];
    let fit = fit_mono_exp(&ts, &ys, 0., &prior, &FitOptions::default())
        .unwrap()
        .unwrap();
    assert_close(fit.hr_inf, 125.577_073_235_678_11);
    assert_close(fit.amp, 26.308_865_486_884_024);
    assert_close(fit.tau_s, 10.);
    assert_eq!(fit.grid.len(), 1);
    let g = &fit.grid[0];
    assert_close(g.tau, 10.);
    assert_close(g.w, 1.);
    assert_close(g.c00, 24.546_516_864_378_94);
    assert_close(g.c01, -27.941_176_299_002_908);
    assert_close(g.c11, 32.544_583_491_691_434);
    let band = predict_at(&fit, 10.).unwrap();
    assert_close(band.mean, 135.255_563_968_847_67);
    assert_close(band.lo, 130.489_894_022_766_8);
    assert_close(band.hi, 140.021_233_914_928_53);
}

/// When every grid point overflows TS keeps finite scalars but NaN weights; a NaN weight is
/// nonfinite output, so this is the typed error (documented divergence, #169).
#[test]
fn all_overflowed_costs_are_a_nonfinite_calculation() {
    let prior = RecoveryPrior {
        tau_s: 1e5,
        tau_log_sd: 1e-300,
        hr_inf: 110.,
        hr_inf_sd: 8.,
    };
    assert_eq!(
        fit_mono_exp(
            &[0., 1., 2., 3.],
            &[150., 149., 148., 147.],
            0.,
            &prior,
            &FitOptions::default()
        )
        .unwrap_err(),
        MetricError::NonFiniteCalculation
    );
}

/// #169 item 2: TS returns null for too little data before looking at the box size.
#[test]
fn dfa_length_guard_precedes_box_size_validation() {
    assert_eq!(dfa_alpha1(&[1., 2.], 0, 16, 25).unwrap(), None);
    assert_eq!(dfa_alpha1(&[], 0, 0, 25).unwrap(), None);
    // Enough data: a zero box size is where TS does not terminate.
    assert_eq!(
        dfa_alpha1(&[800.; 48], 0, 16, 25).unwrap_err(),
        MetricError::InvalidMetricParameter("boxSize")
    );
}
