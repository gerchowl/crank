//! The input layer (ADR-0002 §7.10.2, X5, round 16): the cap, the ± offset and an engagement,
//! each with its own validity, for any number of events on one tick.
//!
//! For each input, in the order its events arrive on a tick, the **last valid value wins** and
//! **any invalid value raises the fault** on that tick; the fault stays raised while that input's
//! latest value is invalid. The easer gets the cap in force ([`Cap`]) and the fault. The caller
//! applies a tick's cap events before its other events, so an engagement on the same tick starts
//! at or below the new cap.

/// A valid cap `M`, W: a number ≥ 0 or +∞ (none). Only [`Cap::new`] makes one, so the easer's
/// contract (a cap that is a number ≥ 0) holds by type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cap(f64);

impl Cap {
    /// No cap.
    pub const NONE: Self = Self(f64::INFINITY);
    /// The cap `x` if it is valid (a number, or +∞), floored at 0 W; `None` for NaN or −∞.
    #[must_use]
    pub fn new(x: f64) -> Option<Self> {
        (!(x.is_nan() || x == f64::NEG_INFINITY)).then_some(Self(x.max(0.0)))
    }
    /// The cap's value, W (+∞ for none).
    #[must_use]
    pub fn get(self) -> f64 {
        self.0
    }
}

/// The inputs' state across ticks: the cap in force, the multiplier, and which input's latest
/// value is invalid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputLayer {
    cap: Cap,
    mult: f64,
    cap_bad: bool,
    off_bad: bool,
    tick_bad: bool,
}

impl InputLayer {
    /// Starting with `initial_cap` (the initial cap counts as the first value: an invalid one is a
    /// fault from the first tick, with no cap known, round 17) and the multiplier 1.
    #[must_use]
    pub fn new(initial_cap: f64) -> Self {
        let cap = Cap::new(initial_cap);
        Self {
            cap: cap.unwrap_or(Cap::NONE),
            mult: 1.0,
            cap_bad: cap.is_none(),
            off_bad: false,
            tick_bad: false,
        }
    }
    /// A new tick: no invalid value has arrived on it yet.
    pub fn begin_tick(&mut self) {
        self.tick_bad = false;
    }
    /// A cap event. A valid cap takes effect at once, whatever else is a fault (round 15).
    pub fn cap(&mut self, x: f64) {
        if let Some(c) = Cap::new(x) {
            self.cap = c;
            self.cap_bad = false;
        } else {
            self.cap_bad = true;
            self.tick_bad = true;
        }
    }
    /// An offset event (the multiplier `1 + offset + coach`): applied if it is a finite number.
    /// Returns whether it was applied (a discontinuity of the line).
    pub fn offset(&mut self, mult: f64) -> bool {
        if mult.is_finite() {
            self.mult = mult;
            self.off_bad = false;
            true
        } else {
            self.off_bad = true;
            self.tick_bad = true;
            false
        }
    }
    /// An engagement event with origin `x`: valid if it is a finite number. An invalid one raises
    /// the fault on this tick. Returns whether it is valid.
    pub fn engage(&mut self, x: f64) -> bool {
        let ok = x.is_finite();
        self.tick_bad |= !ok;
        ok
    }
    /// The cap in force: the last valid one.
    #[must_use]
    pub fn cap_in_force(&self) -> Cap {
        self.cap
    }
    /// The multiplier in force: the last valid one.
    #[must_use]
    pub fn multiplier(&self) -> f64 {
        self.mult
    }
    /// The inputs' fault on this tick: an invalid value arrived on it, or the cap's or the
    /// offset's latest value is invalid. (The plan's own validity is the caller's:
    /// [`plan_finite`](super::plan_finite).)
    #[must_use]
    pub fn fault(&self) -> bool {
        self.cap_bad || self.off_bad || self.tick_bad
    }
}
