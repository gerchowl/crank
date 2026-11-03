//! Time-ago bucket edges; the presentation mapping remains in TS (ADR-0001 §3).
use serde::{Deserialize, Serialize};

/// Axis geometry used for history bucket placement.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum TimeScale {
    /// Logarithmic in one plus elapsed seconds.
    Log,
    /// Linear recent focus followed by a logarithmic tail.
    Fisheye,
}
/// Width of the fisheye focus region in seconds, for a positive finite span.
#[must_use]
pub fn fisheye_focus_s(span_s: f64) -> f64 {
    120_f64.min(span_s / 4.)
}
/// Map seconds ago to normalized axis position, clamping to the span.
/// A zero span produces NaN, matching TS; use positive finite spans for queries.
#[must_use]
pub fn time_ago_to_u(t_ago: f64, span_s: f64, scale: TimeScale) -> f64 {
    let a = t_ago.min(span_s).max(0.);
    match scale {
        TimeScale::Log => a.ln_1p() / span_s.ln_1p(),
        TimeScale::Fisheye => {
            let f = fisheye_focus_s(span_s);
            if a <= f {
                0.5 * a / f
            } else {
                0.5 + 0.5 * (a / f).ln() / (span_s / f).ln()
            }
        }
    }
}
/// Inverse mapping, in seconds ago. `u` is clamped to [0, 1].
#[must_use]
pub fn u_to_time_ago(u: f64, span_s: f64, scale: TimeScale) -> f64 {
    let x = u.clamp(0., 1.);
    match scale {
        TimeScale::Log => (x * span_s.ln_1p()).exp_m1(),
        TimeScale::Fisheye => {
            let f = fisheye_focus_s(span_s);
            if x <= 0.5 {
                x / 0.5 * f
            } else {
                f * (span_s / f).powf((x - 0.5) / 0.5)
            }
        }
    }
}
/// `n + 1` ascending edges in seconds ago, uniform on the chosen axis.
/// Zero buckets yields one NaN edge, matching the TS division by zero.
#[must_use]
pub fn time_ago_edges(span_s: f64, n: u32, scale: TimeScale) -> Vec<f64> {
    (0..=n)
        .map(|k| u_to_time_ago(f64::from(k) / f64::from(n), span_s, scale))
        .collect()
}
