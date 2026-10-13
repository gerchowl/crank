//! HRV from RR intervals in milliseconds, ported from `shared/src/hrv.ts`.
//! Work bounds and integer parameter domains versus TS are documented in
//! <https://github.com/gerchowl/crank/issues/169>.
use crate::metric_error::MetricError;

/// Root mean square successive difference, in milliseconds. Skips only pairs
/// with `abs(b-a) > 0.2*a`; five accepted pairs are required. No other artefact
/// filtering occurs: zero, negative, and IEEE nonfinite samples follow TS arithmetic.
/// Returns `None` for fewer than five accepted pairs, including empty input.
#[must_use]
pub fn rmssd(rr: &[f64]) -> Option<f64> {
    let mut s = 0.;
    // Count as f64 to avoid overflow on arbitrarily long slices.
    let mut count = 0.;
    for pair in rr.windows(2) {
        let d = pair[1] - pair[0];
        if d.abs() > 0.2 * pair[0] {
            continue;
        }
        s += d * d;
        count += 1.;
    }
    (count >= 5.).then(|| (s / count).sqrt())
}

/// Remove a centred moving average from RR milliseconds, truncating at edges.
/// `window` is in beats; even windows use `2*(window/2)+1` beats as in TS.
/// Zero uses a one-beat window. IEEE samples propagate without filtering.
/// # Errors
/// Windows above the signed JS shift range are rejected rather than indexing
/// outside the series (see <https://github.com/gerchowl/crank/issues/169>).
pub fn detrend_rr(rr: &[f64], window: u32) -> Result<Vec<f64>, MetricError> {
    if window > i32::MAX.unsigned_abs() {
        return Err(MetricError::InvalidMetricParameter("detrendWindow"));
    }
    let h = window as usize / 2;
    let mut pre = Vec::with_capacity(rr.len().saturating_add(1));
    pre.push(0.);
    let mut sum = 0.;
    for &x in rr {
        sum += x;
        pre.push(sum);
    }
    Ok(rr
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let a = i.saturating_sub(h);
            let b = i.saturating_add(h).saturating_add(1).min(rr.len());
            x - (pre[b] - pre[a]) / size(b - a)
        })
        .collect())
}
fn size(n: usize) -> f64 {
    // Assemble exact 32-bit limbs before the single rounded double addition.
    let mut limbs = [0_u32; 2];
    for (i, byte) in n.to_le_bytes().into_iter().enumerate() {
        limbs[i / 4] |= u32::from(byte) << ((i % 4) * 8);
    }
    f64::from(limbs[1]) * 4_294_967_296. + f64::from(limbs[0])
}

/// Short-term DFA exponent (dimensionless), with box sizes in beats and optional
/// centred detrending in beats (`0` disables). Defaults in the mock are 4, 16, 25.
/// Requires at least `3*max_box` beats and three positive fluctuation scales.
/// Zero fluctuation or fewer than three positive scales returns `None`. Constant
/// RR values can retain floating-point detrending residuals, as in TS. No RR
/// artefacts are removed.
/// Nonfinite arithmetic follows TS (a nonfinite slope may be `Some(NaN)`).
/// # Errors
/// A zero minimum box size, computational box sizes over 4096, or windows outside JS's signed
/// shift range are rejected to keep index arithmetic and work bounded.
pub fn dfa_alpha1(
    rr_raw: &[f64],
    min_box: u32,
    max_box: u32,
    detrend_window: u32,
) -> Result<Option<f64>, MetricError> {
    if min_box == 0 {
        return Err(MetricError::InvalidMetricParameter("boxSize"));
    }
    if max_box < min_box {
        return Ok(None);
    }
    if u64::try_from(rr_raw.len()).unwrap_or(u64::MAX) < u64::from(max_box) * 3 {
        return Ok(None);
    }
    if max_box > 4096 {
        return Err(MetricError::ComputationLimit);
    }
    let detrended;
    let rr = if detrend_window > 0 {
        detrended = detrend_rr(rr_raw, detrend_window)?;
        &detrended[..]
    } else {
        rr_raw
    };
    let mean = rr.iter().sum::<f64>() / size(rr.len());
    let mut c = 0.;
    let y: Vec<_> = rr
        .iter()
        .map(|&v| {
            c += v - mean;
            c
        })
        .collect();
    let mut lx = Vec::new();
    let mut ly = Vec::new();
    for n in min_box.max(2)..=max_box {
        let width = n as usize;
        let boxes = rr.len() / width;
        if boxes < 2 {
            break;
        }
        let nf = f64::from(n);
        let sx = nf * (nf - 1.) / 2.;
        let sxx = (nf - 1.) * nf * (2. * nf - 1.) / 6.;
        let den = nf * sxx - sx * sx;
        let mut f2 = 0.;
        for values in y.chunks_exact(width) {
            let mut sy = 0.;
            let mut sxy = 0.;
            for (i, &v) in values.iter().enumerate() {
                sy += v;
                sxy += size(i) * v;
            }
            let slope = (nf * sxy - sx * sy) / den;
            let icpt = (sy - slope * sx) / nf;
            for (i, &v) in values.iter().enumerate() {
                let r = v - (icpt + slope * size(i));
                f2 += r * r;
            }
        }
        let f = (f2 / size(boxes * width)).sqrt();
        if f > 0. {
            lx.push(nf.ln());
            ly.push(f.ln());
        }
    }
    if lx.len() < 3 {
        return Ok(None);
    }
    let k = size(lx.len());
    let ax = lx.iter().sum::<f64>() / k;
    let ay = ly.iter().sum::<f64>() / k;
    let mut num = 0.;
    let mut den = 0.;
    for (&x, &y) in lx.iter().zip(&ly) {
        num += (x - ax) * (y - ay);
        den += (x - ax).powi(2);
    }
    Ok((den > 0.).then(|| num / den))
}
