//! Closed-form jerk-limited motion (ADR-0002 §7.10.2, round 11): the planner's probes (safe
//! braking's stop, the floor, the lead's timing) and degraded mode's stop use these, not Ruckig,
//! so none of them can be refused. And the landing bounds of P4/P7 and D5 (§7.10.6), which the
//! easer also uses to reject Ruckig's slower answers.
//!
//! Hand-rolled on purpose (the owner rule of 2026-11-14): Ruckig's own velocity interface is the
//! library's, but a probe that Ruckig can refuse made the fall to 0 W late (round 11), so these
//! few lines are the textbook double-S slope change of Biagiotti and Melchiorri (2008, §3.4),
//! covered by property tests against dense sampling and by the model's golden vectors.
#![expect(
    clippy::many_single_char_names,
    reason = "the textbook notation: y, v, a, j, s, d"
)]
use super::Limits;

/// A closed-form profile: up to three (duration s, jerk W/s³) segments. Empty when the state
/// already has the target slope and no curvature.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Profile {
    segs: [(f64, f64); 3],
    len: usize,
}

impl Profile {
    /// The (duration, jerk) segments.
    #[must_use]
    pub fn segments(&self) -> &[(f64, f64)] {
        &self.segs[..self.len]
    }
    /// The total duration, s.
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.segments().iter().map(|(d, _)| d).sum()
    }
}

/// The time-optimal jerk-limited change of slope from `(v, a)` to `(target, 0)` under `a_max` and
/// `j_max`: the curvature ramps to a peak `a_p ≤ a_max`, holds, and ramps back to 0.
#[must_use]
pub fn slope_change(v: f64, a: f64, target: f64, lim: &Limits) -> Profile {
    let (j, amax) = (lim.j_max, lim.a_max);
    let vu = v + a * a.abs() / (2.0 * j); // the slope once the curvature is unwound now
    let s = if target - vu > 0.0 { 1.0 } else { -1.0 };
    let (d, a1) = (s * (target - v), s * a); // mirrored: a non-negative change, needing a > 0
    if (target - vu).abs() <= 1e-12 && a.abs() <= 1e-12 {
        return Profile::default();
    }
    let ap2 = j * d + a1 * a1 / 2.0;
    let mut ap = ap2.max(0.0).sqrt();
    let mut t2 = 0.0;
    if ap > amax {
        ap = amax;
        t2 = ((d - (2.0 * amax * amax - a1 * a1) / (2.0 * j)) / amax).max(0.0);
    }
    let t1 = ((ap - a1) / j).max(0.0);
    let t3 = ap / j;
    Profile {
        segs: [(t1, s * j), (t2, 0.0), (t3, -s * j)],
        len: 3,
    }
}

/// One segment of constant jerk `jk` for `h` seconds from `(y, v, a)`.
fn advance(y: f64, v: f64, a: f64, jk: f64, h: f64) -> (f64, f64, f64) {
    (
        y + v * h + a * h * h / 2.0 + jk * h * h * h / 6.0,
        v + a * h + jk * h * h / 2.0,
        a + jk * h,
    )
}

/// The state reached after `tt` seconds of `prof` from `(y, v, a)`, then at constant slope
/// (curvature 0).
#[must_use]
pub fn profile_at(y: f64, v: f64, a: f64, prof: &Profile, tt: f64) -> (f64, f64, f64) {
    let (mut y, mut v, mut a, mut tt) = (y, v, a, tt);
    for &(d, jk) in prof.segments() {
        let h = d.min(tt);
        (y, v, a) = advance(y, v, a, jk, h);
        tt -= h;
        if tt <= 0.0 {
            return (y, v, a);
        }
    }
    (y + v * tt, v, 0.0)
}

/// The lowest and highest position over `prof` from `(y, v, a)`: the slope's roots inside each
/// segment, and the segment ends.
#[must_use]
pub fn profile_extrema(y: f64, v: f64, a: f64, prof: &Profile) -> (f64, f64) {
    let (mut y, mut v, mut a) = (y, v, a);
    let (mut lo, mut hi) = (y, y);
    for &(d, jk) in prof.segments() {
        // roots of v + a·τ + jk·τ²/2 in (0, d)
        let mut cands = [f64::NAN; 2];
        if jk.abs() > 1e-15 {
            let disc = a * a - 2.0 * jk * v;
            if disc >= 0.0 {
                cands = [(-a + disc.sqrt()) / jk, (-a - disc.sqrt()) / jk];
            }
        } else if a.abs() > 1e-15 {
            cands[0] = -v / a;
        }
        for tau in cands {
            if 0.0 < tau && tau < d {
                let yy = advance(y, v, a, jk, tau).0;
                lo = lo.min(yy);
                hi = hi.max(yy);
            }
        }
        (y, v, a) = advance(y, v, a, jk, d);
        lo = lo.min(y);
        hi = hi.max(y);
    }
    (lo, hi)
}

/// P4/P7's bound from any state (ADR-0002 §7.10.6): the time to be on (a rise) or at or below (a
/// fall) a held line `delta` away (signed), starting with slope `v0` and curvature `a0`. From rest
/// it is `|Δ|/r + T_e + 0.75 s`, `T_e` = τ rising and τ(1 + f)/2 falling; a moving state adds easing
/// it out (unwinding the curvature, then stopping the slope) and the distance covered meanwhile.
#[must_use]
pub fn landing_bound(lim: &Limits, delta: f64, v0: f64, a0: f64) -> f64 {
    let up = delta > 0.0;
    let r = if up { lim.r_up } else { lim.r_dn };
    let t_e = if up {
        lim.ease
    } else {
        lim.ease * (1.0 + lim.factor) / 2.0
    };
    if v0 == 0.0 && a0 == 0.0 {
        return delta.abs() / r + t_e + 0.75;
    }
    let v_eff = v0.abs() + a0 * a0 / (2.0 * lim.j_max);
    let stop_t = a0.abs() / lim.j_max + v_eff / lim.a_max + lim.a_max / lim.j_max;
    let stop_d = v_eff * stop_t;
    (delta.abs() + stop_d) / r + t_e + stop_t + 0.75
}

/// P7 (round 14) and D5 onto a line of slope `s` strictly within the rates: [`landing_bound`] in
/// the line's frame, closing at the real rates `r↑ − s` (from below) and `r↓ + s` (from above),
/// finite and tight as `s → 0±`. The catch-up's rate follows the sign of the gap left once the
/// relative slope is stopped (the slower rate where that sign is not known), and where the line
/// comes to a waiting command, or the command meets it at rest, an overshoot of at most
/// `|s|·T_s/2` (`T_s = |s|/a_max + a_max/j_max`) is closed at the opposite relative rate.
#[must_use]
pub fn ramp_bound(lim: &Limits, delta: f64, v0: f64, a0: f64, s: f64) -> f64 {
    let (up, dn) = (lim.r_up - s, lim.r_dn + s);
    let v_rel = v0 - s;
    let moving = v_rel != 0.0 || a0 != 0.0;
    let v_eff = v_rel.abs() + a0 * a0 / (2.0 * lim.j_max);
    let (stop_t, stop_d) = if moving {
        (
            a0.abs() / lim.j_max + v_eff / lim.a_max + lim.a_max / lim.j_max,
            v_eff
                * (a0.abs() / lim.j_max + f64::midpoint(v_eff / lim.a_max, lim.a_max / lim.j_max)),
        )
    } else {
        (0.0, 0.0)
    };
    // The gap left after the stop lies between these (curvature: either way).
    let (lo_g, hi_g) = if a0 != 0.0 {
        (delta - stop_d, delta + stop_d)
    } else if v_rel > 0.0 {
        (delta - stop_d, delta)
    } else if v_rel < 0.0 {
        (delta, delta + stop_d)
    } else {
        (delta, delta)
    };
    let r = if lo_g > 0.0 {
        up
    } else if hi_g < 0.0 {
        dn
    } else {
        up.min(dn)
    };
    let t_s = s.abs() / lim.a_max + lim.a_max / lim.j_max;
    let mut over = 0.0_f64;
    if hi_g > 0.0 && s < 0.0 {
        over = over.max(s.abs() * t_s / 2.0 / dn + t_s);
    }
    if lo_g < 0.0 && s > 0.0 {
        over = over.max(s * t_s / 2.0 / up + t_s);
    }
    let d = delta.abs() + stop_d;
    d / r + over + lim.ease * (1.0 + lim.factor) / 2.0 + stop_t + 0.75
}
