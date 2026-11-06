//! Connect attempts of the paired devices and their phase log (ADR-0002 §5.1.6).
//!
//! Every phase change of an attempt is logged at INFO under this module's target,
//! `crank_core::links`, with `device`, `attempt` (per device, from 1), `from`, `to` and `ms`,
//! the time spent in the phase being left. The phases are `resolve` (left with
//! `via=lookup|scan`), `gatt`, `identity` and, for the trainer, `handshake`; an attempt ends in
//! `ready` (then a `ready_total` line with the attempt's whole `ms`) or `not_found` (with the
//! error). The driver reports the phases it enters ([`crank_devices::phase`]); the times come
//! from the core's clock. The trainer's identity check (`Pairing::verify`) runs after its
//! handshake and is counted in `handshake`.
use crank_devices::{
    Clock, MonoTime,
    phase::{self, ConnectPhase, PhaseSink, Via},
};
use std::{
    fmt::Display,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

/// Which paired device an attempt connects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkDevice {
    /// The trainer.
    Trainer,
    /// The heart-rate strap.
    Strap,
}
impl LinkDevice {
    /// The log spelling: `trainer` or `strap`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Trainer => "trainer",
            Self::Strap => "strap",
        }
    }
}

/// The attempt counters of one core, per device.
#[derive(Default)]
pub(crate) struct Attempts([AtomicU32; 2]);
impl Attempts {
    /// The number of `device`'s next attempt, from 1.
    pub(crate) fn next(&self, device: LinkDevice) -> u32 {
        self.0[device as usize].fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// One phase an attempt left, as logged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    /// The phase left.
    pub(crate) from: &'static str,
    /// The phase entered.
    pub(crate) to: &'static str,
    /// Time spent in `from`.
    pub(crate) ms: u64,
    /// How the peripheral was found, on leaving `resolve`.
    pub(crate) via: Option<Via>,
}

/// The phases of one attempt so far.
pub(crate) struct Timeline {
    device: LinkDevice,
    attempt: u32,
    clock: Arc<dyn Clock>,
    start: MonoTime,
    state: Mutex<(&'static str, MonoTime, Vec<Step>)>,
    /// Told each phase entered (the keeper's phases, §5.1.1).
    watch: Option<Box<dyn Fn(&'static str) + Send + Sync>>,
}
impl Timeline {
    /// Attempt number `attempt` of `device`, starting now in `resolve`.
    #[cfg(test)]
    pub(crate) fn new(device: LinkDevice, attempt: u32, clock: Arc<dyn Clock>) -> Arc<Self> {
        Self::watched(device, attempt, clock, None)
    }
    /// As [`Timeline::new`], telling `watch` each phase entered.
    pub(crate) fn watched(
        device: LinkDevice,
        attempt: u32,
        clock: Arc<dyn Clock>,
        watch: Option<Box<dyn Fn(&'static str) + Send + Sync>>,
    ) -> Arc<Self> {
        let start = clock.now();
        Arc::new(Self {
            device,
            attempt,
            clock,
            start,
            state: Mutex::new(("resolve", start, Vec::new())),
            watch,
        })
    }
    /// The phases left so far.
    #[cfg(test)]
    pub(crate) fn steps(&self) -> Vec<Step> {
        self.lock().2.clone()
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, (&'static str, MonoTime, Vec<Step>)> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn ms(&self, since: MonoTime) -> u64 {
        self.clock.now().0.saturating_sub(since.0) / 1000
    }
    /// Leave the current phase for `to`, and log it.
    fn enter(&self, to: &'static str, via: Option<Via>, error: Option<&dyn Display>) {
        let mut state = self.lock();
        let step = Step {
            from: state.0,
            to,
            ms: self.ms(state.1),
            via,
        };
        tracing::info!(
            device = self.device.as_str(),
            attempt = self.attempt,
            from = step.from,
            to,
            ms = step.ms,
            via = via.map(Via::as_str),
            error = error.map(tracing::field::display),
            "link phase"
        );
        state.0 = to;
        state.1 = self.clock.now();
        state.2.push(step);
        drop(state);
        if let Some(watch) = &self.watch {
            watch(to);
        }
    }
    fn sink(self: &Arc<Self>) -> PhaseSink {
        let timeline = self.clone();
        Arc::new(move |p: ConnectPhase| {
            let via = match p {
                ConnectPhase::Gatt(via) => Some(via),
                ConnectPhase::Identity | ConnectPhase::Handshake => None,
            };
            timeline.enter(p.as_str(), via, None);
        })
    }
}

/// Run `connect` as the attempt of `timeline`, bounded by `limit`, logging each phase. `None`
/// when it timed out.
pub(crate) async fn attempt<T, E: Display>(
    timeline: Arc<Timeline>,
    limit: Duration,
    connect: impl Future<Output = Result<T, E>>,
) -> Option<Result<T, E>> {
    let result = tokio::time::timeout(limit, phase::observe(timeline.sink(), connect)).await;
    match &result {
        Ok(Ok(_)) => {
            timeline.enter("ready", None, None);
            tracing::info!(
                device = timeline.device.as_str(),
                attempt = timeline.attempt,
                ms = timeline.ms(timeline.start),
                "ready_total"
            );
        }
        Ok(Err(error)) => timeline.enter("not_found", None, Some(error)),
        Err(_) => timeline.enter("not_found", None, Some(&"connect timed out")),
    }
    result.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crank_devices::{
        DeviceRef, HeartRateDriver, RiderConfig, TrainerDriver,
        sim::{SimConfig, SimLatency, SimRig},
    };
    use crank_engine::VirtualClock;

    fn step(from: &'static str, to: &'static str, ms: u64, via: Option<Via>) -> Step {
        Step { from, to, ms, via }
    }

    /// Each phase of a connect is timed from the driver's reports, and a hung one ends
    /// `not_found` at the limit.
    #[tokio::test(start_paused = true)]
    async fn each_phase_is_timed_from_the_drivers_reports() {
        let clock: Arc<dyn Clock> = Arc::new(VirtualClock::new(1));
        let s = Duration::from_secs;
        let cfg = SimConfig {
            trainer_latency: SimLatency {
                resolve: s(1),
                gatt: s(2),
                handshake: s(3),
                never_answers: false,
                not_advertising: false,
            },
            heart_latency: SimLatency {
                never_answers: true,
                ..SimLatency::default()
            },
            ..SimConfig::default()
        };
        let rig = SimRig::new(cfg, clock.clone());
        let rider = RiderConfig {
            rider_kg: 72.0,
            bike_kg: 9.0,
            wheel_m: 2.096,
        };
        let trainer = Timeline::new(LinkDevice::Trainer, 1, clock.clone());
        let connect = rig.trainer().connect(&DeviceRef("t".into()), rider);
        let link = attempt(trainer.clone(), s(15), connect).await;
        assert!(matches!(link, Some(Ok(_))));
        assert_eq!(
            trainer.steps(),
            [
                step("resolve", "gatt", 1000, Some(Via::Scan)),
                step("gatt", "identity", 2000, None),
                step("identity", "handshake", 0, None),
                step("handshake", "ready", 3000, None),
            ]
        );
        let strap = Timeline::new(LinkDevice::Strap, 2, clock);
        let connect = rig.heart().connect(&DeviceRef("h".into()));
        assert!(attempt(strap.clone(), s(15), connect).await.is_none());
        assert_eq!(
            strap.steps(),
            [
                step("resolve", "gatt", 0, Some(Via::Lookup)),
                step("gatt", "not_found", 15_000, None),
            ]
        );
    }

    /// A connect that fails ends `not_found`, never `ready`.
    #[tokio::test(start_paused = true)]
    async fn a_failed_connect_ends_not_found() {
        let clock: Arc<dyn Clock> = Arc::new(VirtualClock::new(1));
        let trainer = Timeline::new(LinkDevice::Trainer, 1, clock);
        let failed = attempt(trainer.clone(), Duration::from_secs(15), async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Err::<(), _>("no such device")
        })
        .await;
        assert_eq!(failed, Some(Err("no such device")));
        assert_eq!(trainer.steps(), [step("resolve", "not_found", 2000, None)]);
    }

    #[test]
    fn attempts_count_per_device_from_one() {
        let attempts = Attempts::default();
        assert_eq!(attempts.next(LinkDevice::Trainer), 1);
        assert_eq!(attempts.next(LinkDevice::Strap), 1);
        assert_eq!(attempts.next(LinkDevice::Trainer), 2);
    }
}
