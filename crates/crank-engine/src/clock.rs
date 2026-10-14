//! Engine clocks (ADR-0001 §5): monotonic, never wall-clock UTC.
use crank_devices::{Clock, MonoTime};
use futures_util::future::BoxFuture;
use std::time::Duration;
use tokio::time::Instant;

fn micros_since(epoch: Instant) -> u64 {
    u64::try_from(epoch.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Real monotonic time: microseconds since construction on `tokio::time::Instant`.
///
/// Under a paused tokio runtime (`start_paused`) the same type is deterministic, which
/// is what the simulator tests use together with [`VirtualClock`] semantics.
#[derive(Clone, Copy, Debug)]
pub struct MonotonicClock {
    epoch: Instant,
}
impl MonotonicClock {
    /// A clock whose epoch is now.
    #[must_use]
    pub fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}
impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock for MonotonicClock {
    fn now(&self) -> MonoTime {
        MonoTime(micros_since(self.epoch))
    }
    fn sleep_until(&self, at: MonoTime) -> BoxFuture<'static, ()> {
        let deadline = self.epoch + Duration::from_micros(at.0);
        Box::pin(tokio::time::sleep_until(deadline))
    }
}

/// Simulated time running `speed`× faster than the tokio timer it sits on
/// (ADR-0001 §5: tests, `--clock virtual`, 10×/60× simulation).
///
/// Run under `#[tokio::test(start_paused = true)]` the tokio timer itself is virtual and
/// auto-advances when every task is idle, so a whole ride takes no wall time and the
/// ride is bit-for-bit repeatable.
#[derive(Clone, Copy, Debug)]
pub struct VirtualClock {
    epoch: Instant,
    speed: u32,
}
impl VirtualClock {
    /// A clock at `speed`× (clamped to at least 1) whose epoch is now.
    #[must_use]
    pub fn new(speed: u32) -> Self {
        Self {
            epoch: Instant::now(),
            speed: speed.max(1),
        }
    }
}
impl Clock for VirtualClock {
    fn now(&self) -> MonoTime {
        MonoTime(micros_since(self.epoch).saturating_mul(u64::from(self.speed)))
    }
    fn sleep_until(&self, at: MonoTime) -> BoxFuture<'static, ()> {
        // Round up so that `now()` is at or past `at` when the sleep returns.
        let real_us = at.0.div_ceil(u64::from(self.speed));
        Box::pin(tokio::time::sleep_until(
            self.epoch + Duration::from_micros(real_us),
        ))
    }
}
