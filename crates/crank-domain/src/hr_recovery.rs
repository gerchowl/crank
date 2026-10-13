//! Pure variable-projection HR recovery fit, matching `shared/src/hrRecovery.ts`.
//! Times are absolute ride seconds, heart rates bpm, covariances bpm².
//! Unsupported work bounds and nonfinite/singular finite-input divergences from
//! TS are recorded in <https://github.com/gerchowl/crank/issues/169>.
//! Recovery exponentials use pure Rust `libm` to avoid platform-libc rounding
//! differences amplified by the subtractive SSE and posterior weights.
use crate::metric_error::{MetricError, finite_parameters};
use serde::{Deserialize, Serialize};

/// Gaussian priors for the asymptote and log recovery time.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryPrior {
    /// Usual recovery time constant, seconds (positive).
    pub tau_s: f64,
    /// Standard deviation of ln τ (dimensionless, nonzero).
    pub tau_log_sd: f64,
    /// Expected steady heart rate, bpm.
    pub hr_inf: f64,
    /// Standard deviation of steady heart rate, bpm (nonzero).
    pub hr_inf_sd: f64,
}
/// One retained posterior grid component; weights below 1e-6 are omitted,
/// without renormalising retained weights, as in TypeScript.
#[derive(Clone, Debug, Serialize)]
pub struct TauGridPoint {
    /// Time constant, seconds.
    pub tau: f64,
    /// Normalised posterior weight before omission.
    pub w: f64,
    /// Asymptotic heart rate, bpm.
    pub a: f64,
    /// Exponential amplitude, bpm.
    pub b: f64,
    /// Asymptote variance, bpm².
    pub c00: f64,
    /// Asymptote/amplitude covariance, bpm².
    pub c01: f64,
    /// Amplitude variance, bpm².
    pub c11: f64,
}
/// Fitted exponential and posterior mixture, including its sample count.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonoExpFit {
    /// MAP asymptote, bpm.
    pub hr_inf: f64,
    /// MAP amplitude, bpm.
    pub amp: f64,
    /// MAP recovery constant, seconds.
    pub tau_s: f64,
    /// Observation noise standard deviation, bpm.
    pub sigma: f64,
    /// Number of observations.
    pub n: usize,
    /// Absolute time of the post-interval HR peak, seconds.
    pub t0: f64,
    /// Posterior components used for prediction.
    pub grid: Vec<TauGridPoint>,
}
/// Optional fitting configuration; `None` fields use the TS defaults.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FitOptions {
    /// Minimum grid time constant, seconds (default 10).
    pub tau_min: Option<f64>,
    /// Maximum grid time constant, seconds (default 200).
    pub tau_max: Option<f64>,
    /// Log-spaced grid point count (default 96, supported 2..=4096).
    pub grid_n: Option<u32>,
    /// Fixed noise standard deviation in bpm; otherwise estimated after 12 points.
    pub sigma: Option<f64>,
}
/// Mean and 90% curve uncertainty interval; all fields are in bpm for
/// predictions and seconds for readiness. Excludes observation noise.
#[derive(Clone, Debug, Serialize)]
pub struct Band {
    /// Posterior mean.
    pub mean: f64,
    /// Lower 90% bound.
    pub lo: f64,
    /// Upper 90% bound.
    pub hi: f64,
}
#[derive(Clone, Debug)]
struct Solution {
    a: f64,
    b: f64,
    sse: f64,
    j: f64,
    c00: f64,
    c01: f64,
    c11: f64,
}
struct FitContext<'a> {
    ts: &'a [f64],
    ys: &'a [f64],
    t0: f64,
    n: f64,
    sy: f64,
    syy: f64,
    prior: &'a RecoveryPrior,
    amp0: f64,
}
impl FitContext<'_> {
    fn solve(&self, tau: f64, sigma: f64) -> Result<Solution, MetricError> {
        let mut sx = 0.;
        let mut sxx = 0.;
        let mut sxy = 0.;
        for (&t, &y) in self.ts.iter().zip(self.ys) {
            let x = libm::exp(-(t - self.t0) / tau);
            sx += x;
            sxx += x * x;
            sxy += x * y;
        }
        if [sx, sxx, sxy, self.sy, self.syy]
            .iter()
            .any(|v| !v.is_finite())
        {
            return Err(MetricError::NonFiniteCalculation);
        }
        let is2 = 1. / (sigma * sigma);
        let ia2 = 1. / (self.prior.hr_inf_sd * self.prior.hr_inf_sd);
        let ib2 = 1. / 400.;
        let a00 = self.n * is2 + ia2;
        let a01 = sx * is2;
        let a11 = sxx * is2 + ib2;
        let r0 = self.sy * is2 + self.prior.hr_inf * ia2;
        let r1 = sxy * is2 + self.amp0 * ib2;
        let det = a00 * a11 - a01 * a01;
        if det <= 0. || !det.is_finite() {
            return Err(MetricError::NonFiniteCalculation);
        }
        let a = (r0 * a11 - a01 * r1) / det;
        let b = (a00 * r1 - a01 * r0) / det;
        let sse = (self.syy - 2. * a * self.sy - 2. * b * sxy
            + self.n * a * a
            + 2. * a * b * sx
            + b * b * sxx)
            .max(0.);
        let lt = (tau.ln() - self.prior.tau_s.ln()) / self.prior.tau_log_sd;
        let j = sse * is2
            + (a - self.prior.hr_inf).powi(2) * ia2
            + (b - self.amp0).powi(2) * ib2
            + lt * lt
            + det.ln();
        let result = Solution {
            a,
            b,
            sse,
            j,
            c00: a11 / det,
            c01: -a01 / det,
            c11: a00 / det,
        };
        if [a, b, sse, j, result.c00, result.c01, result.c11]
            .iter()
            .any(|v| !v.is_finite())
        {
            return Err(MetricError::NonFiniteCalculation);
        }
        Ok(result)
    }
    fn run(&self, taus: &[f64], sigma: f64) -> Result<(Vec<Solution>, usize), MetricError> {
        let sol: Vec<_> = taus
            .iter()
            .map(|&t| self.solve(t, sigma))
            .collect::<Result<_, _>>()?;
        let mut best = 0;
        for j in 1..sol.len() {
            if sol[j].j < sol[best].j {
                best = j;
            }
        }
        Ok((sol, best))
    }
}
fn fit_parameters(
    t0: f64,
    prior: &RecoveryPrior,
    opts: &FitOptions,
) -> Result<(f64, f64, u32, f64), MetricError> {
    let tau_min = opts.tau_min.unwrap_or(10.);
    let tau_max = opts.tau_max.unwrap_or(200.);
    let g = opts.grid_n.unwrap_or(96);
    let sigma = opts.sigma.unwrap_or(1.5);
    finite_parameters(&[
        ("t0", t0),
        ("tauMin", tau_min),
        ("tauMax", tau_max),
        ("tauS", prior.tau_s),
        ("tauLogSd", prior.tau_log_sd),
        ("hrInf", prior.hr_inf),
        ("hrInfSd", prior.hr_inf_sd),
        ("sigma", sigma),
    ])?;
    for (name, value) in [
        ("tauMin", tau_min),
        ("tauMax", tau_max),
        ("tauS", prior.tau_s),
    ] {
        if value <= 0. {
            return Err(MetricError::InvalidMetricParameter(name));
        }
    }
    for (name, value) in [
        ("sigma", sigma),
        ("hrInfSd", prior.hr_inf_sd),
        ("tauLogSd", prior.tau_log_sd),
    ] {
        if value == 0. {
            return Err(MetricError::InvalidMetricParameter(name));
        }
    }
    if !(2..=4096).contains(&g) {
        return Err(MetricError::ComputationLimit);
    }
    Ok((tau_min, tau_max, g, sigma))
}

/// Fit `hr(t)=hr_inf+amp*exp(-(t-t0)/tau)` from timestamps in seconds and HR in bpm.
/// No sample filtering occurs. Fewer than three points or mismatched arrays yield
/// `None` after configuration validation. Constant times/HR remain regularised
/// by the amplitude and HR priors.
/// Negative nonzero standard deviations retain TS's squared-prior semantics.
/// # Errors
/// Rejects nonfinite inputs, nonpositive τ, zero noise/prior deviations, grids
/// outside 2..=4096, and nonfinite or singular intermediate calculations.
/// Configuration errors are checked even on empty/insufficient input (#169).
pub fn fit_mono_exp(
    ts: &[f64],
    ys: &[f64],
    t0: f64,
    prior: &RecoveryPrior,
    opts: &FitOptions,
) -> Result<Option<MonoExpFit>, MetricError> {
    let (tau_min, tau_max, g, mut sigma) = fit_parameters(t0, prior, opts)?;
    if ts.len() < 3 || ts.len() != ys.len() {
        return Ok(None);
    }
    if ts.iter().chain(ys).any(|x| !x.is_finite()) {
        return Err(MetricError::NonFiniteCalculation);
    }
    let mut sy = 0.;
    let mut syy = 0.;
    let mut n = 0.;
    for &y in ys {
        sy += y;
        syy += y * y;
        n += 1.;
    }
    let ctx = FitContext {
        ts,
        ys,
        t0,
        n,
        sy,
        syy,
        prior,
        amp0: (ys[0] - prior.hr_inf).max(0.),
    };
    let taus: Vec<_> = (0..g)
        .map(|j| tau_min * (tau_max / tau_min).powf(f64::from(j) / f64::from(g - 1)))
        .collect();
    let (mut sol, mut jmin) = ctx.run(&taus, sigma)?;
    if opts.sigma.is_none() && ts.len() >= 12 {
        sigma = (sol[jmin].sse / (n - 2.)).sqrt().clamp(0.8, 6.);
        (sol, jmin) = ctx.run(&taus, sigma)?;
    }
    let minimum_cost = sol[jmin].j;
    let mut wsum = 0.;
    let ws: Vec<_> = sol
        .iter()
        .map(|s| {
            let w = (-(s.j - minimum_cost) / 2.).exp();
            wsum += w;
            w
        })
        .collect();
    let grid = sol
        .iter()
        .zip(&taus)
        .zip(&ws)
        .filter_map(|((s, &tau), &weight)| {
            let w = weight / wsum;
            (w >= 1e-6).then_some(TauGridPoint {
                tau,
                w,
                a: s.a,
                b: s.b,
                c00: s.c00,
                c01: s.c01,
                c11: s.c11,
            })
        })
        .collect();
    let mut tau_hat = taus[jmin];
    if jmin > 0 && jmin < taus.len() - 1 {
        let y0 = sol[jmin - 1].j;
        let y1 = sol[jmin].j;
        let y2 = sol[jmin + 1].j;
        let den = y0 - 2. * y1 + y2;
        if den > 0. {
            let off = (0.5 * (y0 - y2)) / den;
            let step = (tau_max / tau_min).ln() / f64::from(g - 1);
            tau_hat = (tau_hat.ln() + off.clamp(-0.5, 0.5) * step).exp();
        }
    }
    let best = ctx.solve(tau_hat, sigma)?;
    Ok(Some(MonoExpFit {
        hr_inf: best.a,
        amp: best.b,
        tau_s: tau_hat,
        sigma,
        n: ts.len(),
        t0,
        grid,
    }))
}
/// Predict at absolute ride time `t` in seconds using the posterior mixture.
/// An empty grid returns a zero band, as in TS.
/// # Errors
/// Rejects nonfinite time/components and nonfinite prediction arithmetic.
pub fn predict_at(fit: &MonoExpFit, t: f64) -> Result<Band, MetricError> {
    finite_parameters(&[("t", t), ("t0", fit.t0)])?;
    let mut m = 0.;
    let mut m2 = 0.;
    for g in &fit.grid {
        if [g.tau, g.w, g.a, g.b, g.c00, g.c01, g.c11]
            .iter()
            .any(|x| !x.is_finite())
        {
            return Err(MetricError::NonFiniteCalculation);
        }
        let x = libm::exp(-(t - fit.t0) / g.tau);
        let mu = g.a + g.b * x;
        let variance = g.c00 + 2. * g.c01 * x + g.c11 * x * x;
        if !variance.is_finite() {
            return Err(MetricError::NonFiniteCalculation);
        }
        let v = variance.max(0.);
        m += g.w * mu;
        m2 += g.w * (v + mu * mu);
    }
    let sd = (m2 - m * m).max(0.).sqrt();
    let band = Band {
        mean: m,
        lo: m - 1.645 * sd,
        hi: m + 1.645 * sd,
    };
    if [band.mean, band.lo, band.hi, m2]
        .iter()
        .any(|x| !x.is_finite())
    {
        return Err(MetricError::NonFiniteCalculation);
    }
    Ok(band)
}

/// One HR observation: absolute ride seconds and optional bpm (`None` is dropout).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sample {
    /// Absolute ride time, seconds.
    pub t: f64,
    /// Heart rate, bpm; missing measurements are retained in state output.
    pub v: Option<f64>,
}
/// Raw recovery-block input, with 1 Hz HR observations.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryInput {
    /// Absolute block start, seconds.
    pub started_t: f64,
    /// Minimum recovery duration, seconds.
    pub min_s: f64,
    /// Maximum recovery duration, seconds.
    pub max_s: f64,
    /// Heart rate readiness threshold, bpm.
    pub target_bpm: f64,
    /// Chronologically ordered observations; only dropouts are removed for fitting.
    pub observed: Vec<Sample>,
    /// Current absolute ride time, seconds.
    pub now: f64,
    /// Prior usual recovery time constant, seconds.
    pub usual_tau_s: f64,
    /// Prior steady HR, bpm.
    pub prior_hr_inf: f64,
    /// Prior steady HR standard deviation, bpm (default 8).
    pub prior_hr_inf_sd: Option<f64>,
    /// Prior ln τ standard deviation (default 0.35).
    pub tau_log_sd: Option<f64>,
    /// Peak search duration from block start, seconds (default 60).
    pub peak_search_s: Option<f64>,
}
/// Compact fit reported to recovery consumers.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryFit {
    /// Fitted steady HR, bpm.
    pub hr_inf: f64,
    /// Fitted amplitude, bpm.
    pub amp: f64,
    /// Fitted recovery time constant, seconds.
    pub tau_s: f64,
    /// Athlete's prior usual time constant, seconds.
    pub usual_tau_s: f64,
}
/// One forecast point in absolute seconds and bpm.
#[derive(Clone, Debug, Serialize)]
pub struct Prediction {
    /// Absolute forecast time, seconds.
    pub t: f64,
    /// HR mean and uncertainty, bpm.
    #[serde(flatten)]
    pub band: Band,
}
/// Recovery state; fit/prediction/readiness are absent before four post-peak points.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HrRecoveryState {
    /// Whether recovery is active.
    pub active: bool,
    /// Block start, absolute seconds.
    pub started_t: f64,
    /// Latest equal-valued HR peak in the search window, absolute seconds.
    pub peak_t: f64,
    /// Readiness threshold, bpm.
    pub target_bpm: f64,
    /// Original observations, including dropouts.
    pub observed: Vec<Sample>,
    /// Whether the fitted steady HR is at or above the target.
    pub unreachable: bool,
    /// Minimum recovery duration, seconds.
    pub min_s: f64,
    /// Maximum recovery duration, seconds.
    pub max_s: f64,
    /// Compact fitted parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<RecoveryFit>,
    /// Forecast curve, sampled at the TS cadence (at most about 91 points).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicted: Option<Vec<Prediction>>,
    /// Seconds from now until ready, bounded by block minimum and maximum.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_in_s: Option<Band>,
}
/// Inactive recovery with zero times, no observations, and absent predictions.
#[must_use]
pub fn empty_recovery_state() -> HrRecoveryState {
    HrRecoveryState {
        active: false,
        started_t: 0.,
        peak_t: 0.,
        target_bpm: 0.,
        observed: Vec::new(),
        unreachable: false,
        min_s: 0.,
        max_s: 0.,
        fit: None,
        predicted: None,
        ready_in_s: None,
    }
}
/// Find the post-interval peak, fit from there, and forecast readiness on a 1 s grid.
/// Drops only `None` observations. Equal peaks choose the latest; input order is
/// preserved. Minimum duration applies even when measured HR is already ready.
/// Fractional horizons retain the TS loop's integer steps and endpoint behaviour.
/// # Errors
/// Rejects nonfinite inputs/calculations and horizons over 86400 s (one day),
/// preventing unbounded loops. Nonfinite configuration is rejected even before
/// a peak or fit is available. See [`fit_mono_exp`] for fitting errors.
pub fn build_hr_recovery_state(inp: &RecoveryInput) -> Result<HrRecoveryState, MetricError> {
    let peak_search = inp.peak_search_s.unwrap_or(60.);
    finite_parameters(&[
        ("startedT", inp.started_t),
        ("minS", inp.min_s),
        ("maxS", inp.max_s),
        ("targetBpm", inp.target_bpm),
        ("now", inp.now),
        ("peakSearchS", peak_search),
        ("usualTauS", inp.usual_tau_s),
        ("priorHrInf", inp.prior_hr_inf),
        ("priorHrInfSd", inp.prior_hr_inf_sd.unwrap_or(8.)),
        ("tauLogSd", inp.tau_log_sd.unwrap_or(0.35)),
    ])?;
    if inp
        .observed
        .iter()
        .any(|s| !s.t.is_finite() || s.v.is_some_and(|v| !v.is_finite()))
    {
        return Err(MetricError::NonFiniteCalculation);
    }
    let valid: Vec<_> = inp
        .observed
        .iter()
        .filter_map(|s| s.v.map(|v| (s.t, v)))
        .collect();
    let mut peak_idx = None;
    for (i, &(t, v)) in valid.iter().enumerate() {
        if t > inp.started_t + peak_search {
            break;
        }
        if peak_idx.is_none_or(|p: usize| v >= valid[p].1) {
            peak_idx = Some(i);
        }
    }
    let peak_t = peak_idx.map_or(inp.started_t, |i| valid[i].0);
    let mut base = HrRecoveryState {
        active: true,
        started_t: inp.started_t,
        peak_t,
        target_bpm: inp.target_bpm,
        observed: inp.observed.clone(),
        unreachable: false,
        min_s: inp.min_s,
        max_s: inp.max_s,
        fit: None,
        predicted: None,
        ready_in_s: None,
    };
    let Some(idx) = peak_idx else {
        return Ok(base);
    };
    let pts = &valid[idx..];
    if pts.len() < 4 {
        return Ok(base);
    }
    let prior = RecoveryPrior {
        tau_s: inp.usual_tau_s,
        tau_log_sd: inp.tau_log_sd.unwrap_or(0.35),
        hr_inf: inp.prior_hr_inf,
        hr_inf_sd: inp.prior_hr_inf_sd.unwrap_or(8.),
    };
    let ts: Vec<_> = pts.iter().map(|p| p.0).collect();
    let ys: Vec<_> = pts.iter().map(|p| p.1).collect();
    let Some(fit) = fit_mono_exp(&ts, &ys, peak_t, &prior, &FitOptions::default())? else {
        return Ok(base);
    };
    let end_t = inp.started_t + inp.max_s;
    let last = pts[pts.len() - 1];
    let now = inp.now.max(last.0);
    let earliest = (inp.started_t + inp.min_s - now).max(0.);
    let latest = (end_t - now).max(0.);
    if !end_t.is_finite() || !earliest.is_finite() || !latest.is_finite() {
        return Err(MetricError::NonFiniteCalculation);
    }
    if latest > 86400. {
        return Err(MetricError::ComputationLimit);
    }
    let (predicted, ready) = forecast(&fit, now, earliest, latest, inp.target_bpm, last.1)?;
    base.ready_in_s = Some(ready);
    base.unreachable = fit.hr_inf >= inp.target_bpm;
    base.fit = Some(RecoveryFit {
        hr_inf: fit.hr_inf,
        amp: fit.amp,
        tau_s: fit.tau_s,
        usual_tau_s: inp.usual_tau_s,
    });
    base.predicted = Some(predicted);
    Ok(base)
}

fn forecast(
    fit: &MonoExpFit,
    now: f64,
    earliest: f64,
    latest: f64,
    target: f64,
    last_hr: f64,
) -> Result<(Vec<Prediction>, Band), MetricError> {
    let step = (latest / 90.).ceil().max(1.);
    let mut c_mean = None;
    let mut c_lo = None;
    let mut c_hi = None;
    let mut predicted = Vec::new();
    for seconds in 0..=86400_u32 {
        let dt = f64::from(seconds);
        if dt > latest {
            break;
        }
        let p = predict_at(fit, now + dt)?;
        if c_lo.is_none() && p.lo <= target {
            c_lo = Some(dt);
        }
        if c_mean.is_none() && p.mean <= target {
            c_mean = Some(dt);
        }
        if c_hi.is_none() && p.hi <= target {
            c_hi = Some(dt);
        }
        if dt % step == 0. || dt.to_bits() == latest.to_bits() {
            predicted.push(Prediction {
                t: now + dt,
                band: p,
            });
        }
    }
    let clamp = |x: Option<f64>| latest.min(earliest.max(x.unwrap_or(latest)));
    let ready = if last_hr <= target {
        Band {
            mean: earliest,
            lo: earliest,
            hi: earliest,
        }
    } else {
        Band {
            mean: clamp(c_mean),
            lo: clamp(c_lo),
            hi: clamp(c_hi),
        }
    };
    Ok((predicted, ready))
}
