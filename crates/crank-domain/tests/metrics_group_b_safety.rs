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
