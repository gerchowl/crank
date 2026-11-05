//! The link keeper (ADR-0002 §5.1): who holds the paired trainer and strap before a ride, and
//! when.
//!
//! [`Keeper`] is the state machine, as pure transitions: an [`Input`] and the time it happened
//! at go in, [`Effect`]s come out, and the time is always an input (the clock), never read. The
//! task that runs it ([`run`]) owns the links, the connect attempts, the scan and the timers,
//! and turns each effect into Bluetooth work through the drivers.
//!
//! - **Intent** (§5.1.2): `Likely` warms every paired device that is `cold` up, trainer and
//!   strap in parallel, and a fresh `Likely` (one after another intent) also every device that
//!   is `not_found`. A refresh of a `Likely` that stands, the throttled rider input, retries
//!   nothing: a `not_found` device is retried on its next advertisement in the capped scan,
//!   never on a connect timer. `Retry` retries every device at once, scans continuously again
//!   and clears the latch.
//! - **Cool-down** (§5.1.3): `T_COOL` after the last `Likely { ready: false }` or after `Left`;
//!   `GRACE` after `Background`; at once on `Release`. `Likely { ready: true }` (the Ready sheet
//!   over a load in flight or failed) has no idle timer. `RiderRelease` latches the trainer:
//!   no warm-up of it until a load or `Retry`.
//! - **A ride** (§5.1.4): a load hands the links over once the trainer is `warm` and the strap
//!   has settled (an attempt that is running is awaited, never doubled). From then until the
//!   ride is released the keeper is inactive: every intent is ignored, except that a `Likely`
//!   counts as rider input for `T_ABANDON` and that `Sleep` and `RiderRelease` discard a ride
//!   that has not started. A started ride is never touched.
use crate::dto::{LinkIntent, ReleaseReason};
use crate::links::LinkDevice;
use crank_devices::{MonoTime, RiderConfig};
use std::time::Duration;

pub(crate) mod run;
#[cfg(test)]
mod tests;

/// Idle time without a `Likely` after which the keeper's links are released.
pub(crate) const T_COOL: Duration = Duration::from_mins(5);
/// Time in the background after which the keeper's links are released.
pub(crate) const GRACE: Duration = Duration::from_mins(1);
/// Time without Start and without rider input after which a loaded ride is discarded.
pub(crate) const T_ABANDON: Duration = Duration::from_mins(10);
/// The retry scan runs continuously for this long after a device became `not_found`.
pub(crate) const SCAN_CONTINUOUS: Duration = Duration::from_secs(30);
/// After that, a scan window opens once per period.
pub(crate) const SCAN_PERIOD: Duration = Duration::from_secs(30);
/// The length of a scan window: a connect's own resolve scan.
pub(crate) const SCAN_WINDOW: Duration = crank_ble::RESOLVE_SCAN;

/// The link phase of one paired device (ADR-0002 §5.1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Not connected, no attempt running.
    Cold,
    /// An attempt resolves the peripheral.
    Searching,
    /// An attempt connects (GATT, discovery, subscribe, identity).
    Connecting,
    /// Trainer only: an attempt awaits the page-54 reply.
    Handshake,
    /// The keeper holds the link.
    Warm,
    /// The last attempt failed, or the warm link was lost.
    NotFound,
    /// Handed to a ride's session: the keeper no longer owns it.
    Riding,
}
impl Phase {
    fn attempting(self) -> bool {
        matches!(self, Self::Searching | Self::Connecting | Self::Handshake)
    }
    fn held(self) -> bool {
        self == Self::Warm || self.attempting()
    }
}

/// Why the keeper let a device go: the `reason` of its `cool` log line (§5.1.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cool {
    /// `T_COOL` without a `Likely`.
    Idle,
    /// `GRACE` in the background.
    Background,
    /// A `Release` intent.
    Release(ReleaseReason),
    /// The warm link was lost.
    Lost,
    /// The rider configuration changed (page 55 goes out only at connect).
    Rider,
    /// The pairing changed.
    Unpaired,
    /// The application quits.
    Shutdown,
}

/// What happened, as the keeper's transitions take it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Input<'a> {
    /// The UI's intent (`links_intent`).
    Intent(LinkIntent),
    /// Time passed (a deadline of [`Keeper::next_wake`] is due).
    Tick,
    /// Attempt `era` of a device entered a phase.
    Phase(LinkDevice, u32, Phase),
    /// Attempt `era` of a device ended: linked (`true`) or not.
    Attempted(LinkDevice, u32, bool),
    /// The warm link of attempt `era` was lost.
    Lost(LinkDevice, u32),
    /// The retry scan saw this platform id.
    Advertised(&'a str),
    /// The paired platform ids, trainer then strap.
    Pairings([Option<String>; 2]),
    /// The rider configuration that a trainer link must be connected with.
    Rider(RiderConfig),
    /// A load wants the links (after the [`Input::Rider`] it needs).
    Acquire,
    /// The load that was handed the links was gone before it took them: they are the
    /// keeper's again.
    Refused,
    /// The ride of the handed links was loaded.
    Loaded(String),
    /// The ride started.
    Started(String),
    /// The ride (or the load that was handed the links) let them go.
    Released,
    /// The application quits.
    Shutdown,
}

/// What the keeper's task must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    /// Start attempt `era` of a device.
    Attempt(LinkDevice, u32),
    /// Let a device go: cancel its attempt (`was` attempting) or release its warm link.
    Cool {
        /// The device.
        device: LinkDevice,
        /// Its phase before.
        was: Phase,
        /// Why.
        reason: Cool,
    },
    /// Hand the warm trainer (and the strap, if warm) to the waiting load.
    HandOver,
    /// The waiting load gets no trainer.
    FailLoad,
    /// Discard this loaded, unstarted ride (the path of Cancel).
    Discard(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    /// No intent since launch, a release or a cool-down.
    None,
    Likely {
        ready: bool,
        since: MonoTime,
    },
    Left {
        since: MonoTime,
    },
    Background {
        since: MonoTime,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Ride {
    /// No ride: the keeper is active.
    Free,
    /// The links are handed to a load that has not loaded its ride yet.
    Handed,
    /// A ride is loaded.
    Loaded {
        id: String,
        started: bool,
        /// The load, or the last rider input since.
        input_at: MonoTime,
        /// The discard was asked for.
        discarding: bool,
    },
}

#[derive(Clone, Debug)]
struct Dev {
    /// The paired platform id; no phase without one.
    id: Option<String>,
    phase: Phase,
    /// The current attempt; results of any other are stale.
    era: u32,
}

/// The keeper's state machine (module docs).
#[derive(Clone, Debug)]
pub(crate) struct Keeper {
    devs: [Dev; 2],
    intent: Intent,
    /// `RiderRelease`: no automatic warm-up of the trainer.
    latched: bool,
    ride: Ride,
    /// A load waits for the attempts to settle.
    waiting: bool,
    /// The last time a device became `not_found`: the scan cap counts from it.
    scan_since: Option<MonoTime>,
    rider: RiderConfig,
    shut: bool,
    out: Vec<Effect>,
}

fn after(t: MonoTime, d: Duration) -> MonoTime {
    MonoTime(t.0.saturating_add(u64::try_from(d.as_micros()).unwrap_or(u64::MAX)))
}
fn us(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

impl Keeper {
    /// No pairings, no intent; trainer links connect with `rider`.
    pub(crate) fn new(rider: RiderConfig) -> Self {
        let dev = Dev {
            id: None,
            phase: Phase::Cold,
            era: 0,
        };
        Self {
            devs: [dev.clone(), dev],
            intent: Intent::None,
            latched: false,
            ride: Ride::Free,
            waiting: false,
            scan_since: None,
            rider,
            shut: false,
            out: Vec::new(),
        }
    }
    /// A device's phase; `None` without a pairing.
    pub(crate) fn phase(&self, device: LinkDevice) -> Option<Phase> {
        let dev = &self.devs[device as usize];
        dev.id.as_ref().map(|_| dev.phase)
    }
    /// A device's paired platform id.
    pub(crate) fn id(&self, device: LinkDevice) -> Option<&str> {
        self.devs[device as usize].id.as_deref()
    }
    /// The rider configuration trainer attempts connect with.
    pub(crate) fn rider(&self) -> RiderConfig {
        self.rider
    }
    /// Whether attempt `era` is the device's current one.
    pub(crate) fn current(&self, device: LinkDevice, era: u32) -> bool {
        self.devs[device as usize].era == era
    }
    fn likely(&self) -> bool {
        matches!(self.intent, Intent::Likely { .. })
    }

    /// Apply `input`, which happened at `now`.
    pub(crate) fn step(&mut self, now: MonoTime, input: Input<'_>) -> Vec<Effect> {
        match input {
            Input::Intent(intent) => self.intent(now, intent),
            Input::Tick => self.tick(now),
            Input::Phase(d, era, phase) => {
                let dev = &mut self.devs[d as usize];
                if dev.era == era && dev.phase.attempting() {
                    dev.phase = phase;
                }
            }
            Input::Attempted(d, era, linked) => {
                let dev = &mut self.devs[d as usize];
                if dev.era == era && dev.phase.attempting() {
                    dev.phase = if linked { Phase::Warm } else { Phase::NotFound };
                    if !linked {
                        self.scan_since = Some(now);
                    }
                    self.settle();
                }
            }
            Input::Lost(d, era) => {
                if self.devs[d as usize].era == era && self.devs[d as usize].phase == Phase::Warm {
                    self.cool(d, Cool::Lost);
                    self.devs[d as usize].phase = Phase::NotFound;
                    self.scan_since = Some(now);
                    // a load waiting for the strap still wants this device: connect it cold
                    if self.waiting {
                        self.attempt(d);
                    }
                    self.settle();
                }
            }
            Input::Advertised(id) => {
                if self.ride == Ride::Free && self.likely() && !self.shut {
                    for d in [LinkDevice::Trainer, LinkDevice::Strap] {
                        let dev = &self.devs[d as usize];
                        if dev.id.as_deref() == Some(id)
                            && dev.phase == Phase::NotFound
                            && !self.latched_out(d)
                        {
                            self.attempt(d);
                        }
                    }
                }
            }
            Input::Pairings(ids) => {
                for (d, id) in [LinkDevice::Trainer, LinkDevice::Strap]
                    .into_iter()
                    .zip(ids)
                {
                    if self.devs[d as usize].id != id {
                        self.cool(d, Cool::Unpaired);
                        self.devs[d as usize].id = id;
                    }
                }
                self.settle();
            }
            Input::Rider(rider) => self.rider_changed(rider),
            Input::Acquire => self.acquire(now),
            Input::Refused | Input::Loaded(_) | Input::Started(_) | Input::Released => {
                self.ride_input(now, input);
            }
            Input::Shutdown => {
                self.shut = true;
                self.cool_all(Cool::Shutdown);
            }
        }
        std::mem::take(&mut self.out)
    }
    /// What a load and its ride report.
    fn ride_input(&mut self, now: MonoTime, input: Input<'_>) {
        match input {
            Input::Refused if self.ride == Ride::Handed => {
                self.free(Phase::Warm);
                self.resume_waiting();
            }
            Input::Loaded(id) if self.ride == Ride::Handed => {
                self.ride = Ride::Loaded {
                    id,
                    started: false,
                    input_at: now,
                    discarding: false,
                };
            }
            Input::Started(id) => {
                if let Ride::Loaded {
                    id: loaded,
                    started,
                    ..
                } = &mut self.ride
                    && *loaded == id
                {
                    *started = true;
                }
            }
            Input::Released if self.ride != Ride::Free => {
                self.free(Phase::Cold);
                if self.waiting {
                    self.resume_waiting();
                } else {
                    self.intent = Intent::None;
                }
            }
            _ => {}
        }
    }
    /// A load that came while the links were still handed out (its predecessor's release or
    /// return not yet reported) gets them now.
    fn resume_waiting(&mut self) {
        if self.waiting {
            self.warm_up(true);
            self.settle();
        }
    }
    /// The keeper is active again; the devices handed over are in `phase`.
    fn free(&mut self, phase: Phase) {
        self.ride = Ride::Free;
        for dev in &mut self.devs {
            if dev.phase == Phase::Riding {
                dev.phase = phase;
            }
        }
    }
    fn intent(&mut self, now: MonoTime, intent: LinkIntent) {
        if self.ride != Ride::Free {
            match intent {
                LinkIntent::Likely { .. } | LinkIntent::Retry => {
                    if let Ride::Loaded {
                        started: false,
                        input_at,
                        ..
                    } = &mut self.ride
                    {
                        *input_at = now;
                    }
                }
                LinkIntent::Release {
                    reason: reason @ (ReleaseReason::Sleep | ReleaseReason::RiderRelease),
                } => {
                    if let Ride::Loaded {
                        id,
                        started: false,
                        discarding,
                        ..
                    } = &mut self.ride
                    {
                        if !std::mem::replace(discarding, true) {
                            self.out.push(Effect::Discard(id.clone()));
                        }
                        self.latched |= reason == ReleaseReason::RiderRelease;
                    }
                }
                LinkIntent::Left | LinkIntent::Background | LinkIntent::Release { .. } => {}
            }
            return;
        }
        if self.shut {
            return;
        }
        match intent {
            LinkIntent::Likely { ready } => {
                let fresh = !self.likely();
                self.intent = Intent::Likely { ready, since: now };
                self.warm_up(fresh);
            }
            LinkIntent::Retry => {
                self.latched = false;
                self.refresh(now);
                self.warm_up(true);
            }
            LinkIntent::Left => {
                if !matches!(self.intent, Intent::Left { .. }) {
                    self.intent = Intent::Left { since: now };
                }
            }
            LinkIntent::Background => {
                if !matches!(self.intent, Intent::Background { .. }) {
                    self.intent = Intent::Background { since: now };
                }
            }
            LinkIntent::Release { reason } => {
                self.cool_all(Cool::Release(reason));
                self.intent = Intent::None;
                self.latched |= reason == ReleaseReason::RiderRelease;
            }
        }
    }
    /// A `Likely` now, keeping a standing `ready`.
    fn refresh(&mut self, now: MonoTime) {
        let ready = matches!(self.intent, Intent::Likely { ready: true, .. });
        self.intent = Intent::Likely { ready, since: now };
    }
    fn latched_out(&self, d: LinkDevice) -> bool {
        d == LinkDevice::Trainer && self.latched
    }
    /// Attempt every paired device that is `cold` and, with `not_found`, every one that is.
    fn warm_up(&mut self, not_found: bool) {
        for d in [LinkDevice::Trainer, LinkDevice::Strap] {
            let dev = &self.devs[d as usize];
            let due = match dev.phase {
                Phase::Cold => true,
                Phase::NotFound => not_found,
                _ => false,
            };
            if dev.id.is_some() && due && !self.latched_out(d) {
                self.attempt(d);
            }
        }
    }
    fn attempt(&mut self, d: LinkDevice) {
        let dev = &mut self.devs[d as usize];
        dev.era += 1;
        dev.phase = Phase::Searching;
        self.out.push(Effect::Attempt(d, dev.era));
    }
    /// Let `d` go if the keeper holds it or attempts it; it is `cold` after.
    fn cool(&mut self, d: LinkDevice, reason: Cool) {
        let dev = &mut self.devs[d as usize];
        if dev.phase == Phase::Riding {
            return;
        }
        if dev.phase.held() {
            self.out.push(Effect::Cool {
                device: d,
                was: dev.phase,
                reason,
            });
            // a late result of the attempt let go is stale
            dev.era += 1;
        }
        dev.phase = Phase::Cold;
    }
    fn cool_all(&mut self, reason: Cool) {
        self.cool(LinkDevice::Trainer, reason);
        self.cool(LinkDevice::Strap, reason);
        self.settle();
    }
    fn rider_changed(&mut self, rider: RiderConfig) {
        if rider == self.rider {
            return;
        }
        self.rider = rider;
        if self.devs[0].phase.held() {
            self.cool(LinkDevice::Trainer, Cool::Rider);
            if self.likely() || self.waiting {
                self.attempt(LinkDevice::Trainer);
            }
        }
    }
    /// A load wants the links. Only one load runs at a time (the core's `loading` flag): one
    /// that comes while another waits replaces it, which was cancelled. One that comes while
    /// the links are still handed to a load that has ended (its release or their return is
    /// reported on another channel, so it may come later) waits for them.
    fn acquire(&mut self, now: MonoTime) {
        if matches!(self.ride, Ride::Loaded { .. }) || self.shut {
            self.out.push(Effect::FailLoad);
            return;
        }
        self.latched = false;
        self.refresh(now);
        self.waiting = true;
        if self.ride == Ride::Free {
            self.warm_up(true);
            self.settle();
        }
    }
    /// Answer a waiting load once the trainer has settled: hand over when it is `warm` and the
    /// strap has settled too, fail it when the trainer is not to be had.
    fn settle(&mut self) {
        if !self.waiting {
            return;
        }
        let [trainer, strap] = [&self.devs[0], &self.devs[1]].map(|d| d.phase);
        let strap_settled = self.devs[1].id.is_none() || !strap.attempting();
        if trainer == Phase::Warm && strap_settled {
            self.waiting = false;
            self.ride = Ride::Handed;
            for dev in &mut self.devs {
                if dev.phase == Phase::Warm {
                    dev.phase = Phase::Riding;
                }
            }
            self.out.push(Effect::HandOver);
        } else if self.devs[0].id.is_none() || !trainer.held() {
            self.waiting = false;
            self.out.push(Effect::FailLoad);
        }
    }
    fn tick(&mut self, now: MonoTime) {
        if let Ride::Loaded {
            id,
            started: false,
            input_at,
            discarding: discarding @ false,
        } = &mut self.ride
            && now >= after(*input_at, T_ABANDON)
        {
            *discarding = true;
            self.out.push(Effect::Discard(id.clone()));
        }
        if self.ride != Ride::Free {
            return;
        }
        if let Some((due, reason)) = self.cool_due()
            && now >= due
        {
            self.cool_all(reason);
            self.intent = Intent::None;
        }
    }
    /// When the intent cools the devices down, and why.
    fn cool_due(&self) -> Option<(MonoTime, Cool)> {
        match self.intent {
            Intent::Likely {
                ready: false,
                since,
            }
            | Intent::Left { since } => Some((after(since, T_COOL), Cool::Idle)),
            Intent::Background { since } => Some((after(since, GRACE), Cool::Background)),
            Intent::None | Intent::Likely { ready: true, .. } => None,
        }
    }
    /// The capped retry scan at `now` (§5.1.2): on (`Some(continuous)`) or off (`None`), and
    /// when that changes next; `None` when no scan is wanted at all.
    fn scan_at(&self, now: MonoTime) -> Option<(Option<bool>, MonoTime)> {
        let since = self.scan_since?;
        let wanted = !self.shut
            && self.ride == Ride::Free
            && self.likely()
            && [LinkDevice::Trainer, LinkDevice::Strap]
                .into_iter()
                .any(|d| {
                    let dev = &self.devs[d as usize];
                    dev.id.is_some() && dev.phase == Phase::NotFound && !self.latched_out(d)
                });
        if !wanted {
            return None;
        }
        let elapsed = now.0.saturating_sub(since.0);
        let (cont, period, window) = (us(SCAN_CONTINUOUS), us(SCAN_PERIOD), us(SCAN_WINDOW));
        if elapsed < cont {
            return Some((Some(true), MonoTime(since.0 + cont)));
        }
        // window k (from 1) opens `k` periods after the continuous scan ended
        let k = (elapsed - cont) / period;
        let start = since.0 + cont + k * period;
        Some(if k > 0 && now.0 - start < window {
            (Some(false), MonoTime(start + window))
        } else {
            (None, MonoTime(start + period))
        })
    }
    /// Whether the retry scan is on at `now`: `Some(true)` continuous, `Some(false)` a window.
    pub(crate) fn scanning(&self, now: MonoTime) -> Option<bool> {
        self.scan_at(now).and_then(|(on, _)| on)
    }
    /// The next time something is due without an input: a cool-down, `T_ABANDON` or a change
    /// of the scan.
    pub(crate) fn next_wake(&self, now: MonoTime) -> Option<MonoTime> {
        let abandon = match &self.ride {
            Ride::Loaded {
                started: false,
                input_at,
                discarding: false,
                ..
            } => Some(after(*input_at, T_ABANDON)),
            _ => None,
        };
        let cool = (self.ride == Ride::Free)
            .then(|| self.cool_due().map(|(due, _)| due))
            .flatten();
        let scan = self.scan_at(now).map(|(_, change)| change);
        [abandon, cool, scan].into_iter().flatten().min()
    }
}
