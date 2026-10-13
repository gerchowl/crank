//! Behavioral tests ported from shared/test/{wbal,hrRecovery}.test.ts.
use crank_domain::{hr_recovery::*, hrv::*, wbal::*};

// Test-only copy of the mock's seeded mulberry32 and Box-Muller generator.
struct Rng {
    state: u32,
    spare: Option<f64>,
}
impl Rng {
    fn new(seed: u32) -> Self {
        Self {
            state: seed,
            spare: None,
        }
    }
    fn next(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b_79f5);
        let mut t = self.state;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 0x3d));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.
    }
    fn normal(&mut self, sd: f64) -> f64 {
        if let Some(z) = self.spare.take() {
            return sd * z;
        }
        let mut u = 0.;
        while u <= 1e-12 {
            u = self.next();
        }
        let v = self.next();
        let r = (-2. * u.ln()).sqrt();
        self.spare = Some(r * (2. * std::f64::consts::PI * v).sin());
        sd * r * (2. * std::f64::consts::PI * v).cos()
    }
}
fn prior(tau: f64) -> RecoveryPrior {
    RecoveryPrior {
        tau_s: tau,
        tau_log_sd: 0.35,
        hr_inf: 110.,
        hr_inf_sd: 8.,
    }
}
fn synth(
    hr: f64,
    amp: f64,
    tau: f64,
    n: u32,
    noise: f64,
    seed: u32,
    round: bool,
) -> (Vec<f64>, Vec<f64>) {
    let mut rng = Rng::new(seed);
    let mut ts = Vec::new();
    let mut ys = Vec::new();
    for i in 0..n {
        ts.push(1000. + f64::from(i));
        let y = hr + amp * (-f64::from(i) / tau).exp() + rng.normal(noise);
        ys.push(if round { (y + 0.5).floor() } else { y });
    }
    (ts, ys)
}
fn fitted(ts: &[f64], ys: &[f64], p: &RecoveryPrior) -> MonoExpFit {
    fit_mono_exp(ts, ys, 1000., p, &FitOptions::default())
        .expect("finite synthetic fit")
        .expect("enough synthetic points")
}
#[test]
fn wbal_depletion_refill_step_size_and_thirty_fifteen() {
    let cp = 280.;
    let w0 = 20000.;
    let series = wbal_series(&[380.; 60], 1., cp, w0).unwrap();
    assert!((series[59] - 14000.).abs() < 1e-9);
    let tau = w0 / (cp - 140.);
    assert!(
        (wbal_step(10000., 140., tau, cp, w0).unwrap() - (w0 - 10000. * (-1_f64).exp())).abs()
            < 1e-9
    );
    let mut a = 5000.;
    let mut b = 5000.;
    for _ in 0..240 {
        a = wbal_step(a, 100., 0.25, cp, w0).unwrap();
    }
    for _ in 0..60 {
        b = wbal_step(b, 100., 1., cp, w0).unwrap();
    }
    assert!((a - b).abs() < 5e-10);
    assert_eq!(
        wbal_series(&[100.; 240], 0.25, cp, 30000.).unwrap().len(),
        240
    );
    assert!((wbal_step(5000., cp, 100., cp, w0).unwrap() - 5000.).abs() < 1e-12);
    assert!((wbal_step(100., cp + 50., 10., cp, w0).unwrap() + 400.).abs() < 1e-12);
    let depleted = wbal_step(w0, 350., 30., cp, w0).unwrap();
    assert!((depleted - 17900.).abs() < 1e-12);
    let refilled = wbal_step(depleted, 140., 15., cp, w0).unwrap();
    assert!((refilled - (w0 - 2100. * (-140_f64 * 15. / w0).exp())).abs() < 5e-10);
}
#[test]
fn recovery_parameters_across_time_constants_and_seeds() {
    for (tau, n) in [(20., 60), (45., 135), (90., 270)] {
        let (ts, ys) = synth(105., 60., tau, n, 1., 7, true);
        let f = fitted(&ts, &ys, &prior(60.));
        assert!((f.tau_s - tau).abs() / tau < 0.1);
        assert!((f.hr_inf - 105.).abs() < 1.6);
        assert!(f.amp > 55. && f.amp < 65.);
        assert!(f.sigma > 0.7 && f.sigma < 1.6);
    }
    let mut taus = Vec::new();
    for seed in 1..=40 {
        let (ts, ys) = synth(108., 55., 50., 150, 1.5, seed, false);
        taus.push(fitted(&ts, &ys, &prior(70.)).tau_s);
    }
    let mean = taus.iter().sum::<f64>() / 40.;
    assert!((mean - 50.).abs() < 2.5);
    assert!(taus.iter().all(|&t| t > 42. && t < 60.));
}
fn sd_log(f: &MonoExpFit) -> f64 {
    let m = f.grid.iter().map(|g| g.w * g.tau.ln()).sum::<f64>();
    f.grid
        .iter()
        .map(|g| g.w * (g.tau.ln() - m).powi(2))
        .sum::<f64>()
        .sqrt()
}
#[test]
fn scarce_data_prior_and_prediction_uncertainty() {
    let (ts, ys) = synth(105., 60., 30., 150, 2., 7, false);
    let usual = RecoveryPrior {
        hr_inf_sd: 30.,
        ..prior(70.)
    };
    let flat = RecoveryPrior {
        tau_log_sd: 10.,
        ..usual.clone()
    };
    let early = fitted(&ts[..5], &ys[..5], &usual);
    let early_flat = fitted(&ts[..5], &ys[..5], &flat);
    let late = fitted(&ts, &ys, &usual);
    assert!((early.tau_s / 70.).ln().abs() < (early_flat.tau_s / 70.).ln().abs());
    assert!((late.tau_s - 30.).abs() < 3.);
    assert!(sd_log(&early) > 3. * sd_log(&late));
    let (ts, ys) = synth(105., 60., 45., 200, 1., 7, false);
    let mut widths = Vec::new();
    for n in [10, 30, 60, 120] {
        let f = fitted(&ts[..n], &ys[..n], &prior(45.));
        let p = predict_at(&f, 1180.).unwrap();
        widths.push(p.hi - p.lo);
        if n >= 30 {
            let truth = 105. + 60. * (-180_f64 / 45.).exp();
            assert!(truth > p.lo - 1. && truth < p.hi + 1.);
        }
    }
    assert!(widths.windows(2).all(|w| w[1] < w[0]));
    assert!(
        fit_mono_exp(
            &[0., 1.],
            &[150., 149.],
            0.,
            &prior(50.),
            &FitOptions::default()
        )
        .unwrap()
        .is_none()
    );
}
fn observations(hr: f64, amp: f64, tau: f64, n: u32) -> Vec<Sample> {
    let mut rng = Rng::new(3);
    (0..n)
        .map(|i| {
            let y = if i < 3 {
                hr + amp - f64::from(3 - i)
            } else {
                hr + amp * (-f64::from(i - 3) / tau).exp()
            };
            let value = (y + rng.normal(0.8) + 0.5).floor();
            Sample {
                t: 500. + f64::from(i),
                v: if i == 20 || i == 21 {
                    None
                } else {
                    Some(value)
                },
            }
        })
        .collect()
}
fn input(target: f64, now: f64, observed: Vec<Sample>) -> RecoveryInput {
    RecoveryInput {
        started_t: 500.,
        min_s: 60.,
        max_s: 240.,
        usual_tau_s: 50.,
        prior_hr_inf: 108.,
        target_bpm: target,
        observed,
        now,
        prior_hr_inf_sd: None,
        tau_log_sd: None,
        peak_search_s: None,
    }
}
#[test]
fn recovery_peak_forecast_and_dropout() {
    let st = build_hr_recovery_state(&input(120., 569., observations(105., 65., 45., 70))).unwrap();
    assert!(st.active && (502. ..=504.).contains(&st.peak_t));
    let fit = st.fit.unwrap();
    assert!((fit.tau_s - 45.).abs() < 6.);
    assert!((fit.usual_tau_s - 50.).abs() < 1e-12);
    assert!(!st.unreachable);
    let ready = st.ready_in_s.unwrap();
    let crossing = 503. + 45. * (65_f64 / 15.).ln();
    assert!((ready.mean - (crossing - 569.).max(0.)).abs() < 5.);
    assert!(ready.lo <= ready.mean && ready.hi >= ready.mean);
    assert_eq!(st.observed.len(), 70);
    assert!(st.observed[20].v.is_none());
    let st = build_hr_recovery_state(&input(115., 529., observations(105., 65., 45., 30))).unwrap();
    let p = st.predicted.unwrap();
    assert!(p.len() > 10);
    assert!((p[0].t - 529.).abs() < 1e-12);
    assert!((p.last().unwrap().t - 740.).abs() < 1e-12);
    assert!(
        p.iter()
            .all(|x| x.band.lo <= x.band.mean && x.band.hi >= x.band.mean)
    );
    let crossing = 503. + 45. * (65_f64 / 10.).ln() - 529.;
    let ready = st.ready_in_s.unwrap();
    assert!((ready.mean - crossing).abs() < 25.);
    assert!(ready.hi - ready.lo > 5.);
}
#[test]
fn recovery_minimum_maximum_unreachable_and_still_rising() {
    let st = build_hr_recovery_state(&input(120., 539., observations(95., 40., 15., 40))).unwrap();
    let ready = st.ready_in_s.unwrap();
    assert!(
        [ready.mean, ready.lo, ready.hi]
            .iter()
            .all(|v| (*v - 21.).abs() < 1e-12)
    );
    let mut inp = input(120., 619., observations(126., 40., 30., 120));
    inp.prior_hr_inf = 124.;
    let st = build_hr_recovery_state(&inp).unwrap();
    assert!(st.unreachable);
    assert!(st.fit.unwrap().hr_inf > 120.);
    assert!((st.ready_in_s.unwrap().mean - 121.).abs() < 1e-12);
    let obs = (0..4)
        .map(|i| Sample {
            t: 500. + f64::from(i),
            v: Some(160. + f64::from(i)),
        })
        .collect();
    let st = build_hr_recovery_state(&input(120., 503., obs)).unwrap();
    assert!(st.fit.is_none());
    assert!((st.peak_t - 503.).abs() < 1e-12);
}
#[test]
fn hrv_pair_threshold_trends_and_degenerate_fits() {
    assert!(rmssd(&[]).is_none());
    assert!(rmssd(&[800.; 5]).is_none());
    assert!(rmssd(&[800.; 6]).unwrap().abs() < 1e-15);
    let rr = [800., 810., 790., 1200., 800., 805., 795., 800., 810.];
    assert!((rmssd(&rr).unwrap() - 11.180_339_887_498_949).abs() < 1e-12);
    assert!(dfa_alpha1(&[800.; 120], 4, 16, 25).unwrap().is_none());
    assert!(dfa_alpha1(&[800.; 47], 4, 16, 25).unwrap().is_none());
    let rr: Vec<_> = (0..120).map(|i| 800. + f64::from(i)).collect();
    let detrended = detrend_rr(&rr, 25).unwrap();
    assert!(detrended[12..108].iter().all(|v| v.abs() < 1e-12));
    let ts = [0.; 4];
    let ys = [150.; 4];
    let fit = fit_mono_exp(&ts, &ys, 0., &prior(50.), &FitOptions::default())
        .unwrap()
        .unwrap();
    assert!(fit.tau_s.is_finite());
    assert!(predict_at(&fit, 20.).unwrap().mean.is_finite());
    assert!(!empty_recovery_state().active);
}
