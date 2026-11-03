//! Compliance (ADR-0002 §7.8): the share of ERG seconds with `power_3s` within ±5 % of
//! the controlled target, excluding settle, released, paused and `suspect` /
//! `not_following` seconds.

/// Settle after a block boundary that is a discontinuity, in ticks (5 s).
pub const BLOCK_SETTLE_TICKS: u64 = 20;
/// Settle after an ERG (re-)engagement: the 5 s soft start plus 5 s, in ticks.
pub const ENGAGE_SETTLE_TICKS: u64 = 40;
const TOLERANCE: f64 = 0.05;
use crate::ticks::TICKS_PER_SECOND;

/// The running tally, in whole seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compliance {
    /// Seconds that count (ERG, riding, settled, followed, with power).
    pub counted_s: u32,
    /// Of those, seconds within ±5 % of the controlled target.
    pub within_s: u32,
    /// Seconds excluded for any rule above.
    pub excluded_s: u32,
}
impl Compliance {
    /// `within / counted`, `None` before any second counted.
    #[must_use]
    pub fn share(&self) -> Option<f64> {
        (self.counted_s > 0).then(|| f64::from(self.within_s) / f64::from(self.counted_s))
    }
}

/// What one tick tells compliance. `power_3w`/`target` are those of the second's last tick.
#[derive(Clone, Copy, Debug)]
pub struct TickFacts {
    /// Whether the tick is excluded by a rule (paused, released, settling, not followed, …).
    pub excluded: bool,
    /// Mean power over the last 3 s, if any.
    pub power_3s_w: Option<f64>,
    /// The controlled target (never smoothed, never derived from measured power).
    pub target_w: f64,
}

/// Accumulates ticks into seconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct Accumulator {
    tally: Compliance,
    excluded: bool,
}
impl Accumulator {
    /// The tally so far.
    #[must_use]
    pub fn tally(&self) -> Compliance {
        self.tally
    }
    /// Feed tick `tick` (4 ticks per second); closes the second on its last tick.
    pub fn tick(&mut self, tick: u64, f: TickFacts) {
        self.excluded |= f.excluded;
        if !(tick + 1).is_multiple_of(TICKS_PER_SECOND) {
            return;
        }
        let excluded = std::mem::take(&mut self.excluded);
        match (excluded, f.power_3s_w) {
            (false, Some(p)) => {
                self.tally.counted_s += 1;
                if (p - f.target_w).abs() <= TOLERANCE * f.target_w {
                    self.tally.within_s += 1;
                }
            }
            _ => self.tally.excluded_s += 1,
        }
    }
}
