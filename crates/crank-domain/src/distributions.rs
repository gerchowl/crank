//! Distributions over 1 Hz samples. Nonfinite samples represent missing seconds.
use serde::{Deserialize, Serialize};

/// One histogram bin, including empty bins between populated ones.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct HistogramBin {
    /// Inclusive lower boundary in the metric's units, rounded to six decimals.
    pub lo: f64,
    /// Number of valid one-second samples in this bin.
    pub seconds: usize,
}
/// Time-in-bin distribution for one metric.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Histogram {
    /// Metric identifier provided by the caller.
    pub metric: String,
    /// Effective bin width; large ranges automatically widen it.
    pub bin_width: f64,
    /// Contiguous bins, or empty if no finite samples exist.
    pub bins: Vec<HistogramBin>,
    /// Expected seconds per bin when a typical distribution was supplied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typical: Option<Vec<f64>>,
}
fn round6(x: f64) -> f64 {
    crate::js_round(x * 1e6) / 1e6
}
fn width(x: f64) -> f64 {
    if x > 0. { x } else { 1. }
}
/// Histogram in metric units. Nonpositive widths become one; nonfinite samples
/// are omitted. Empty input omits `typical`, even when a model was supplied.
/// The optional callback returns a fraction of time in the half-open bin.
#[must_use]
pub fn histogram_from(
    metric: &str,
    series: &[f64],
    bin_width: f64,
    typical: Option<&dyn Fn(f64, f64) -> f64>,
) -> Histogram {
    let mut bw = width(bin_width);
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut total = 0.;
    for &v in series.iter().filter(|v| v.is_finite()) {
        lo = lo.min(v);
        hi = hi.max(v);
        total += 1.;
    }
    let mut h = Histogram {
        metric: metric.into(),
        bin_width: bw,
        bins: vec![],
        typical: None,
    };
    if total == 0. {
        return h;
    }
    let mut lo0 = (lo / bw).floor() * bw;
    let mut nb = ((hi - lo0) / bw).floor() + 1.;
    if nb > 400. {
        bw *= (((hi - lo0) / 400.) / bw).ceil();
        lo0 = (lo / bw).floor() * bw;
        nb = ((hi - lo0) / bw).floor() + 1.;
    }
    // One-based integer grid also avoids narrowing floating-point bin indices.
    let grid: Vec<_> = (0_u32..=u32::MAX)
        .take_while(|&i| f64::from(i) < nb)
        .map(f64::from)
        .collect();
    h.bins = grid
        .iter()
        .map(|i| HistogramBin {
            lo: round6(lo0 + i * bw),
            seconds: 0,
        })
        .collect();
    for &v in series.iter().filter(|v| v.is_finite()) {
        let index = ((v - lo0) / bw).floor();
        let i = grid.partition_point(|&g| g < index).min(grid.len() - 1);
        h.bins[i].seconds += 1;
    }
    h.bin_width = bw;
    h.typical = typical.map(|f| {
        grid.iter()
            .map(|i| f(lo0 + i * bw, lo0 + (i + 1.) * bw) * total)
            .collect()
    });
    h
}
fn norm_cdf(z: f64) -> f64 {
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1. / (1. + 0.327_591_1 * x);
    let y = 1.
        - ((((1.061_405_429 * t - 1.453_152_027) * t + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t
            * (-x * x).exp();
    if z >= 0. {
        0.5 * (1. + y)
    } else {
        0.5 * (1. - y)
    }
}
/// Synthetic mixture-of-normals reference distribution. FTP is in watts.
/// Supported metrics are power (including smoothed variants), `pct_ftp`, HR and
/// cadence. Other metrics have no typical model. FTP must be finite and positive.
#[must_use]
pub fn typical_distribution(metric: &str, ftp: f64) -> Option<impl Fn(f64, f64) -> f64> {
    let components = match metric {
        "power" | "power_3s" | "power_10s" | "power_30s" => vec![
            (0.12, 0.25 * ftp, 0.12 * ftp),
            (0.45, 0.62 * ftp, 0.07 * ftp),
            (0.28, 0.82 * ftp, 0.08 * ftp),
            (0.15, 1.05 * ftp, 0.1 * ftp),
        ],
        "pct_ftp" => vec![
            (0.12, 25., 12.),
            (0.45, 62., 7.),
            (0.28, 82., 8.),
            (0.15, 105., 10.),
        ],
        "hr" => vec![(0.2, 115., 10.), (0.5, 138., 9.), (0.3, 160., 9.)],
        "cadence" => vec![(0.1, 75., 8.), (0.9, 89., 5.)],
        _ => return None,
    };
    Some(move |lo, hi| {
        components.iter().fold(0., |s, &(w, m, sd)| {
            s + w * (norm_cdf((hi - m) / sd) - norm_cdf((lo - m) / sd))
        })
    })
}
/// Joint time-in-bin distribution, in row-major order.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Heatmap2D {
    /// Horizontal metric identifier.
    pub x: String,
    /// Vertical metric identifier.
    pub y: String,
    /// Horizontal bin width in metric units.
    pub x_bin: f64,
    /// Vertical bin width in metric units.
    pub y_bin: f64,
    /// Lowest horizontal boundary, rounded to six decimals.
    pub x_lo: f64,
    /// Lowest vertical boundary, rounded to six decimals.
    pub y_lo: f64,
    /// Number of columns, capped at 400.
    pub cols: usize,
    /// Number of rows, capped at 400.
    pub rows: usize,
    /// One-second counts; missing pairs are omitted, overflow goes in the last bin.
    pub seconds: Vec<usize>,
}
/// Paired 1 Hz heatmap. Uses the shorter series and ignores pairs with a
/// nonfinite member. Nonpositive bin widths become one; empty axes start at zero.
#[must_use]
pub fn heatmap_from(x: &str, y: &str, xs: &[f64], ys: &[f64], x_bin: f64, y_bin: f64) -> Heatmap2D {
    let xb = width(x_bin);
    let yb = width(y_bin);
    let pairs = || {
        xs.iter()
            .zip(ys)
            .filter(|(a, b)| a.is_finite() && b.is_finite())
    };
    let (mut xl, mut xh, mut yl, mut yh) = (
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    );
    for (&a, &b) in pairs() {
        xl = xl.min(a);
        xh = xh.max(a);
        yl = yl.min(b);
        yh = yh.max(b);
    }
    let mut heatmap = Heatmap2D {
        x: x.into(),
        y: y.into(),
        x_bin: xb,
        y_bin: yb,
        x_lo: 0.,
        y_lo: 0.,
        cols: 0,
        rows: 0,
        seconds: vec![],
    };
    if !xl.is_finite() {
        return heatmap;
    }
    let x_lo = (xl / xb).floor() * xb;
    let y_lo = (yl / yb).floor() * yb;
    let grid = |hi: f64, lo: f64, bin: f64| {
        (0..400)
            .map(f64::from)
            .take_while(|&i| i < ((hi - lo) / bin).floor() + 1.)
            .collect::<Vec<_>>()
    };
    let cols = grid(xh, x_lo, xb);
    let rows = grid(yh, y_lo, yb);
    heatmap.cols = cols.len();
    heatmap.rows = rows.len();
    heatmap.x_lo = round6(x_lo);
    heatmap.y_lo = round6(y_lo);
    heatmap.seconds.resize(heatmap.cols * heatmap.rows, 0);
    for (&a, &b) in pairs() {
        let c = cols
            .partition_point(|&g| g < ((a - x_lo) / xb).floor())
            .min(heatmap.cols - 1);
        let r = rows
            .partition_point(|&g| g < ((b - y_lo) / yb).floor())
            .min(heatmap.rows - 1);
        heatmap.seconds[r * heatmap.cols + c] += 1;
    }
    heatmap
}
/// Reference power-duration sampling grid, in seconds.
pub const MMP_DURATIONS_S: [u16; 23] = [
    1, 2, 3, 5, 7, 10, 15, 20, 30, 45, 60, 90, 120, 180, 240, 300, 420, 600, 900, 1200, 1800, 2400,
    3600,
];
/// Best mean watts at each integer duration from 1 Hz data, rounded to 0.1 W.
/// Missing samples contribute zero; negative means cannot beat the initial zero.
/// Empty input, zero durations, and durations longer than the ride return zero.
#[must_use]
pub fn mmp_from(power: &[f64], durations: &[u16]) -> Vec<f64> {
    let mut pre = vec![0.];
    let mut sum = 0.;
    for &v in power {
        sum += if v.is_finite() { v } else { 0. };
        pre.push(sum);
    }
    durations
        .iter()
        .map(|&d| {
            if d == 0 || usize::from(d) > power.len() {
                return 0.;
            }
            let best = (usize::from(d)..=power.len()).fold(0_f64, |best, i| {
                best.max((pre[i] - pre[i - usize::from(d)]) / f64::from(d))
            });
            crate::js_round(best * 10.) / 10.
        })
        .collect()
}
/// Synthetic CP/W′/peak-power curve in watts, scaled and rounded to whole watts.
/// FTP is watts; durations are seconds. Beyond 20 min it declines gently.
/// The output is forced nonincreasing in the supplied order, including empty input.
#[must_use]
pub fn synthetic_mmp(ftp: f64, scale: f64, durations: &[u16]) -> Vec<f64> {
    let cp = ftp * 1.04;
    let pmax = ftp * 3.9;
    let mut prev = f64::INFINITY;
    durations
        .iter()
        .map(|&d| {
            let d = f64::from(d);
            let mut p = 20000. / (d + 20000. / (pmax - cp)) + cp;
            if d > 1200. {
                p *= 1. - 0.07 * (d / 1200.).log10() / 3_f64.log10();
            }
            p *= scale;
            p = prev.min(p);
            prev = p;
            crate::js_round(p)
        })
        .collect()
}
/// Empty power-duration response using the reference duration grid.
#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct MmpCurve {
    /// Durations in seconds.
    pub durations_s: Vec<f64>,
    /// Today's mean power in watts.
    pub today: Vec<f64>,
    /// Best power in the preceding 90 days, initially absent.
    pub best90d: Vec<f64>,
    /// Best lifetime power, initially absent.
    pub all_time: Vec<f64>,
}
/// Construct an empty MMP result: today is zero; historical curves are empty.
#[must_use]
pub fn empty_mmp() -> MmpCurve {
    MmpCurve {
        durations_s: MMP_DURATIONS_S.iter().copied().map(f64::from).collect(),
        today: vec![0.; MMP_DURATIONS_S.len()],
        best90d: vec![],
        all_time: vec![],
    }
}
