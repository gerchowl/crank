//! The eased ERG target (ADR-0002 §7.10, #585; built by #620, wired by #621).
//!
//! Every engine tick the easer hands Ruckig (through its Rust port [`rsruckig`]) the commanded
//! target's state, a target state on the plan line and the rider's limits, and sends the position
//! Ruckig's trajectory reaches one tick later. crank's own part is the rest of ADR-0002 §7.10.2:
//! which target state and in which frame, when to re-plan, three cuts that only lower or stop the
//! load (X2–X4), and what happens on a refusal (the twin X5a, then degraded mode X5, round 13).
//!
//! * [`line`]: the plan line on the tick grid as canonical pieces ([`segments`]).
//! * [`profile`]: the closed-form jerk-limited slope change ([`slope_change`]) behind the planner's
//!   probes and degraded mode's stop, and the landing bounds ([`landing_bound`], [`ramp_bound`]).
//! * [`inputs`]: the input layer for several events on one tick ([`InputLayer`], round 16).
//! * [`easer`]: [`Easer`], the per-tick rules.
//!
//! The executable design model is `docs/adr/models/erg_slew.py` (upstream Ruckig in the loop);
//! this module is its port, and the golden vectors in `tests/fixtures/slew_golden.json` hold the two
//! together within 1e-6 (ADR-0002 §7.10.2 "Determinism").
pub mod easer;
pub mod inputs;
pub mod line;
pub mod profile;

pub use easer::{Cut, Diagnostics, Easer, TickInput};
pub use inputs::{Cap, InputLayer};
pub use line::{Block, Piece, line_at, merge_collinear, plan_finite, segments};
pub use profile::{Profile, landing_bound, profile_at, profile_extrema, ramp_bound, slope_change};

use crate::tuning::SlewTuning;

/// The engine tick the easer steps on, s (ADR-0001 §7.2's engine constants, round 13).
pub const DT: f64 = 0.25;
/// "On the line" is `|R − y| ≤ EPS`, W: half the 1 W resolution; X2's threshold.
pub const EPS: f64 = 0.5;
/// Settle lasts this many ticks (5 s) after landing following a discontinuity or an engagement
/// (ADR-0002 §7.10.5; used by #621's compliance).
pub const SETTLE_AFTER_TICKS: u32 = 20;
/// Two slopes are one slope within this, relative to `max(1, |slope|)`; a slope within it of 0 is 0.
pub const SLOPE_TOL: f64 = 1e-9;
/// After the clamp, a piece continues the previous one if it starts within this of its value, W.
pub const VALUE_TOL: f64 = 1e-6;
/// Before the clamp, a block that starts within this of the previous block's end is joined, W.
pub const RAW_JOIN: f64 = 1e-9;
/// Frame and lead keys are the same input within this, relative to `max(1, |key|)`; also the tie
/// margin of the lead's timing, s.
pub const KEY_TOL: f64 = 1e-9;
/// Safe braking: a stop may pass the line ahead by at most this (`EPS/2`, strictly inside X2's
/// `EPS`), W.
pub const BRAKE_SLACK: f64 = EPS / 2.0;
/// Safe braking: a stop may pass the cap `M` by at most this (floating point), W.
pub const CAP_SLACK: f64 = 1e-6;
/// A landing this close outside `[0, M]` is floating-point rounding: snapped to the bound, not
/// cut, W (round 12).
pub const ROUND_SNAP: f64 = 1e-9;
/// A cut is named only when it changes the position by more than this, W.
pub const CUT_TOL_W: f64 = 1e-6;
/// … or the slope by more than this, W/s.
pub const CUT_TOL_V: f64 = 1e-5;
/// … or the curvature by more than this, W/s².
pub const CUT_TOL_A: f64 = 1e-4;
/// X5a: the twin calculation tightens every limit by `TWIN·max(1, |limit|)` (round 13).
pub const TWIN: f64 = 1.0 / 1_048_576.0;
/// A target velocity is kept `TARGET_MARGIN·max(1, |limit|)` strictly inside its limits (Ruckig
/// 0.19.4 segfaults on a target velocity exactly at `max_velocity` from some states).
pub const TARGET_MARGIN: f64 = 1.0 / 1_048_576.0;

/// The commanded target's state: position `y` (W), slope `v` (W/s), curvature `a` (W/s²).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct State {
    /// The commanded target, W (sent rounded as `Erg { target_w }`).
    pub y: f64,
    /// Its slope, W/s: Ruckig's velocity.
    pub v: f64,
    /// Its curvature, W/s²: Ruckig's acceleration.
    pub a: f64,
}

impl State {
    /// At rest at `y`: an engagement's start (ADR-0002 §7.10.3).
    #[must_use]
    pub fn at_rest(y: f64) -> Self {
        Self { y, v: 0.0, a: 0.0 }
    }
}

/// The rider's limits as Ruckig takes them: the mapping of ADR-0002 §7.10.2, one pair of limits
/// for every move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// The rising rate `r↑ = rise`, W/s: `max_velocity`.
    pub r_up: f64,
    /// The falling rate `r↓ = rise·factor`, W/s: `min_velocity = −r↓`.
    pub r_dn: f64,
    /// `a_max = 2·rise/τ`, W/s², both directions.
    pub a_max: f64,
    /// `j_max = 4·rise/τ²`, W/s³, both directions.
    pub j_max: f64,
    /// The ease time `τ`, s.
    pub ease: f64,
    /// The fall factor `f`.
    pub factor: f64,
}

impl From<&SlewTuning> for Limits {
    fn from(t: &SlewTuning) -> Self {
        let r_up = f64::from(t.rise_w_per_s);
        Self {
            r_up,
            r_dn: r_up * t.fall_factor,
            a_max: 2.0 * r_up / t.ease_s,
            j_max: 4.0 * r_up / (t.ease_s * t.ease_s),
            ease: t.ease_s,
            factor: t.fall_factor,
        }
    }
}

/// `x` clamped to `[0, m]`.
#[must_use]
pub fn clamp(x: f64, m: f64) -> f64 {
    x.max(0.0).min(m)
}

/// A tick index as seconds' worth of ticks (`k · 1.0`): tick counts stay far below 2⁵³.
#[expect(
    clippy::cast_precision_loss,
    reason = "tick indices are far below 2^53"
)]
pub(crate) fn tick_f(k: i64) -> f64 {
    k as f64
}

#[cfg(test)]
mod tests;
