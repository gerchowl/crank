//! Tick arithmetic, the two time axes and sample holding (ADR-0001 §5).
use crank_devices::MonoTime;

/// One tick is 250 ms (FE-C's 4 Hz), in microseconds.
pub const TICK_US: u64 = 250_000;
/// Ticks per second.
pub const TICKS_PER_SECOND: u64 = 4;
/// A trainer value is held for up to 1.0 s (4 ticks) after its sample's tick.
pub const TRAINER_HOLD_TICKS: u64 = 4;
/// A heart-rate value is held for up to 3.0 s (12 ticks) after its sample's tick.
pub const HR_HOLD_TICKS: u64 = 12;
/// Consecutive ticks without a trainer sample that mean the trainer is lost (2.0 s).
pub const LOSS_TICKS: u32 = 8;
/// Consecutive ticks of cadence = 0 and power = 0 that mean stopped pedalling (3 s).
pub const STOPPED_TICKS: u32 = 12;

/// Seconds in `ticks` ticks, as the contract carries time (ADR-0001 §4.1).
#[must_use]
pub fn ticks_to_seconds(ticks: u64) -> f64 {
    #[expect(clippy::cast_precision_loss, reason = "ride lengths fit f64 exactly")]
    let t = ticks as f64;
    t / 4.0
}

/// Session time: ticks since the ride started, running while riding and paused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionTicks(pub u64);
/// Workout time: ticks spent in `riding`; drives the block timer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkoutTicks(pub u64);

/// Tick `k` covers the monotonic window `[start + k·250 ms, start + (k+1)·250 ms)`.
#[derive(Clone, Copy, Debug)]
pub struct TickWindow {
    start: MonoTime,
}
impl TickWindow {
    /// Windows anchored at the ride's start instant.
    #[must_use]
    pub fn new(start: MonoTime) -> Self {
        Self { start }
    }
    /// The tick whose window contains `at`; `None` before the ride started.
    #[must_use]
    pub fn tick_of(&self, at: MonoTime) -> Option<u64> {
        at.0.checked_sub(self.start.0).map(|d| d / TICK_US)
    }
    /// First instant after tick `k`'s window: the tick may be processed from here on.
    #[must_use]
    pub fn end_of(&self, k: u64) -> MonoTime {
        MonoTime(self.start.0.saturating_add((k + 1).saturating_mul(TICK_US)))
    }
}

/// The last value seen, usable for `ticks` ticks after the tick it arrived in.
#[derive(Clone, Debug)]
pub struct Hold<T> {
    last: Option<(u64, T)>,
    ticks: u64,
}
impl<T: Copy> Hold<T> {
    /// A hold that lasts `ticks` ticks past the sample's own tick.
    #[must_use]
    pub fn new(ticks: u64) -> Self {
        Self { last: None, ticks }
    }
    /// Record a value that arrived in tick `k`.
    pub fn push(&mut self, k: u64, value: T) {
        self.last = Some((k, value));
    }
    /// The value for tick `k`: fresh, or held for at most the hold length, else `None`.
    #[must_use]
    pub fn get(&self, k: u64) -> Option<T> {
        self.last
            .filter(|(at, _)| k >= *at && k - at <= self.ticks)
            .map(|(_, v)| v)
    }
    /// Forget the held value (the source is gone; ADR-0001 §5).
    pub fn clear(&mut self) {
        self.last = None;
    }
}
