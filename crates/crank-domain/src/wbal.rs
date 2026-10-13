//! Skiba differential W′ balance; exact integration for piecewise constant power.
use crate::metric_error::{MetricError, finite_parameters};

/// Advance W′ balance in joules using power and critical power `cp` in watts,
/// elapsed `dt` in seconds, and initial capacity `w0` in joules.
/// Balance may become negative. Finite zero/negative parameters retain the TS
/// formula (including IEEE nonfinite results); no physiological clamps apply.
/// Nonfinite parameters are typed errors; measured power/balance propagate IEEE values.
/// # Errors
/// Returns an error for nonfinite `dt`, `cp`, or `w0`.
pub fn wbal_step(wbal: f64, power: f64, dt: f64, cp: f64, w0: f64) -> Result<f64, MetricError> {
    finite_parameters(&[("dt", dt), ("cp", cp), ("wPrime", w0)])?;
    Ok(step(wbal, power, dt, cp, w0))
}
fn step(wbal: f64, power: f64, dt: f64, cp: f64, w0: f64) -> f64 {
    if power > cp {
        wbal - (power - cp) * dt
    } else {
        w0 - (w0 - wbal) * (-(cp - power) * dt / w0).exp()
    }
}
/// Balance after each power sample (watts), starting full at `w0` joules.
/// Sampling interval `dt` is in seconds. No dropout/artefact filtering is applied;
/// empty input yields an empty trace. Edge behaviour is that of [`wbal_step`].
/// # Errors
/// Returns an error for nonfinite `dt`, `cp`, or `w0`, even for empty input.
pub fn wbal_series(power: &[f64], dt: f64, cp: f64, w0: f64) -> Result<Vec<f64>, MetricError> {
    finite_parameters(&[("dt", dt), ("cp", cp), ("wPrime", w0)])?;
    let mut w = w0;
    Ok(power
        .iter()
        .map(|&p| {
            w = step(w, p, dt, cp, w0);
            w
        })
        .collect())
}
