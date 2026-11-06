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
//!   ride is released the keeper is inactive: a `Likely` counts as rider input for
//!   `T_ABANDON`, `Sleep` and `RiderRelease` discard a ride that has not started, and a started
//!   ride is never touched.
//!
//! **Totality.** The ride state is one of three: `Free` (the keeper is active), `Handed` (the
//! links are handed to a load that has not loaded its ride yet) and `Loaded`. Every input has a
//! transition in each of them, written as matches with no catch-all arm, and every intent is
//! answered by exactly one [`Ack`]: applied (possibly with nothing to do, by a rule of
//! §5.1.3), recorded (in `Handed`: applied the moment the load resolves, then acknowledged),
//! awaiting a discard, or refused. No intent is dropped.
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
    /// The discard of this ride ended: it was discarded (`true`), or not (a Start was sent
    /// first, or the ride was gone).
    Discarded(String, bool),
    /// The ride (or the load that was handed the links) let them go.
    Released,
    /// The application quits.
    Shutdown,
}

/// How an intent was taken (module docs, **Totality**).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ack {
    /// Applied now; it is answered once the releases it caused are done.
    Applied,
    /// Recorded while the links are handed out: answered when the load resolves and the
    /// recorded intent is applied (then a second `Ack` settles it).
    Recorded,
    /// It discards the loaded ride (or joins the discard under way): answered with the
    /// discard's outcome.
    AwaitDiscard,
    /// Refused: the ride holds the trainer (`RiderRelease` during a started ride), or the
    /// application quits (a `Likely` or `Retry` after `Shutdown`).
    Refused(&'static str),
    /// Not an answer to this step's intent: the releases recorded in `Handed` so far were
    /// superseded by a rider action (a load, Retry) and are cancelled; their answers are
    /// `Conflict` ([`SUPERSEDED`]). Recorded intents that are no release stay recorded.
    Superseded,
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
    /// Discard this loaded, unstarted ride (the path of Cancel); [`Input::Discarded`] reports
    /// how it ended.
    Discard(String),
    /// The answer to the intent of this step, or, in a step that resolves `Handed`, to every
    /// intent recorded there.
    Ack(Ack),
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

/// Why a discard of an unstarted ride is under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Discard {
    /// `T_ABANDON` passed.
    Abandoned,
    /// `Release(Sleep)`.
    Sleep,
    /// "Release trainer": its success latches the trainer.
    RiderRelease,
}

/// What `Handed` recorded, applied when the load resolves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Recorded {
    /// The strongest release: `RiderRelease`, then `Sleep`, then `Devices`, then `Hidden`. A
    /// later intent that is no release and no rider action (visibility, focus) never
    /// overrides it; a later rider action (a load, Retry) cancels it.
    release: Option<ReleaseReason>,
    /// The latest other intent.
    other: Option<LinkIntent>,
    /// Some intent was recorded: its answer is due at the resolution.
    any: bool,
}
/// A rider action in `Handed` cancels the release recorded so far; `true` if there was one.
fn supersede(recorded: &mut Recorded) -> bool {
    let had = recorded.release.take().is_some();
    recorded.any = recorded.other.is_some();
    had
}
fn strength(reason: ReleaseReason) -> u8 {
    match reason {
        ReleaseReason::Hidden => 0,
        ReleaseReason::Devices => 1,
        ReleaseReason::Sleep => 2,
        ReleaseReason::RiderRelease => 3,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Ride {
    /// No ride: the keeper is active.
    Free,
    /// The links are handed to a load that has not loaded its ride yet.
    Handed {
        recorded: Recorded,
        /// The device's pairing changed since the handover: a link that comes back
        /// ([`Input::Refused`]) is the old device's, and is released.
        stale: [bool; 2],
    },
    /// A ride is loaded.
    Loaded {
        id: String,
        started: bool,
        /// The load, or the last rider input since.
        input_at: MonoTime,
        /// A discard is under way, and why.
        discard: Option<Discard>,
    },
}

#[derive(Clone, Debug)]
struct Dev {
    /// The paired platform id; no phase without one.
    id: Option<String>,
    phase: Phase,
    /// The current attempt; results of any other are stale.
    era: u32,
    /// The current attempt was started by an advertisement in the retry scan: its failure
    /// does not restart the scan cap.
    advertised: bool,
    /// The scan window (0: the continuous scan) of the last attempt an advertisement
    /// started, since the cap began: at most one per window (§5.1.2).
    advert_window: Option<u64>,
}

/// The keeper's state machine (module docs).
#[derive(Clone, Debug)]
pub(crate) struct Keeper {
    devs: [Dev; 2],
    intent: Intent,
    /// `RiderRelease`: no automatic warm-up of the trainer.
    latched: bool,
    ride: Ride,
    /// A load waits for the attempts to settle (or for links still handed out).
    waiting: bool,
    /// When the scan cap began: a device became `not_found` other than by the failure of an
    /// attempt an advertisement started.
    scan_since: Option<MonoTime>,
    rider: RiderConfig,
    /// The rider configuration of the trainer's current attempt or link.
    trainer_rider: RiderConfig,
    shut: bool,
    out: Vec<Effect>,
}

fn after(t: MonoTime, d: Duration) -> MonoTime {
    MonoTime(t.0.saturating_add(u64::try_from(d.as_micros()).unwrap_or(u64::MAX)))
}
fn us(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}
const DEVICES: [LinkDevice; 2] = [LinkDevice::Trainer, LinkDevice::Strap];
const QUITTING: &str = "the application is quitting";
/// The answer to a recorded release that a rider action cancelled.
pub(crate) const SUPERSEDED: &str = "superseded by the rider's request for a ride";
const RIDE_HOLDS_TRAINER: &str = "the ride holds the trainer: end the ride first";

impl Keeper {
    /// No pairings, no intent; trainer links connect with `rider`.
    pub(crate) fn new(rider: RiderConfig) -> Self {
        let dev = Dev {
            id: None,
            phase: Phase::Cold,
            era: 0,
            advertised: false,
            advert_window: None,
        };
        Self {
            devs: [dev.clone(), dev],
            intent: Intent::None,
            latched: false,
            ride: Ride::Free,
            waiting: false,
            scan_since: None,
            rider,
            trainer_rider: rider,
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
            Input::Intent(intent) => {
                let ack = self.intent(now, intent);
                self.emit(Effect::Ack(ack));
            }
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
                    // a failed attempt of an advertisement does not restart the cap (§5.1.2)
                    if !linked && !dev.advertised {
                        self.start_scan(now);
                    }
                    self.settle();
                }
            }
            Input::Lost(d, era) => {
                if self.devs[d as usize].era == era && self.devs[d as usize].phase == Phase::Warm {
                    self.cool(d, Cool::Lost);
                    self.devs[d as usize].phase = Phase::NotFound;
                    self.start_scan(now);
                    // a load waiting for the strap still wants this device: connect it cold
                    if self.waiting {
                        self.attempt(d, false);
                    }
                    self.settle();
                }
            }
            Input::Advertised(id) => self.advertised(now, id),
            Input::Pairings(ids) => self.pairings(ids),
            Input::Rider(rider) => self.rider_changed(rider),
            Input::Acquire => self.acquire(now),
            Input::Refused => self.refused(now),
            Input::Loaded(id) => self.loaded(now, id),
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
            Input::Discarded(id, discarded) => self.discarded(now, &id, discarded),
            Input::Released => self.released(now),
            Input::Shutdown => {
                self.shut = true;
                self.cool_all(Cool::Shutdown);
            }
        }
        // a load still waiting after the quit gets no links (ADR-0002 §5.1.3)
        if self.shut && std::mem::replace(&mut self.waiting, false) {
            self.emit(Effect::FailLoad);
        }
        std::mem::take(&mut self.out)
    }
    /// The one way an effect leaves the keeper, and the one place the quit is enforced: after
    /// `Shutdown` no effect may acquire a link. An attempt is dropped (the device stays
    /// `cold`); a handover is turned back (its links released) into the load's failure. Every
    /// path that would connect or hand over after the quit, now or later, goes through here.
    fn emit(&mut self, effect: Effect) {
        if self.shut {
            match effect {
                Effect::Attempt(d, _) => {
                    self.devs[d as usize].phase = Phase::Cold;
                    return;
                }
                Effect::HandOver => {
                    self.free(Phase::Warm);
                    self.cool_all(Cool::Shutdown);
                    self.out.push(Effect::FailLoad);
                    return;
                }
                Effect::Cool { .. } | Effect::FailLoad | Effect::Discard(_) | Effect::Ack(_) => {}
            }
        }
        self.out.push(effect);
    }

    /// An intent, in each ride state (module docs, **Totality**).
    fn intent(&mut self, now: MonoTime, intent: LinkIntent) -> Ack {
        match &mut self.ride {
            Ride::Handed { recorded, .. } => {
                recorded.any = true;
                match intent {
                    LinkIntent::Release { reason } => {
                        if recorded
                            .release
                            .is_none_or(|r| strength(reason) >= strength(r))
                        {
                            recorded.release = Some(reason);
                        }
                    }
                    LinkIntent::Retry => {
                        let superseded = supersede(recorded);
                        recorded.other = Some(intent);
                        recorded.any = true;
                        if superseded {
                            self.emit(Effect::Ack(Ack::Superseded));
                        }
                    }
                    LinkIntent::Likely { .. } | LinkIntent::Left | LinkIntent::Background => {
                        recorded.other = Some(intent);
                    }
                }
                Ack::Recorded
            }
            Ride::Loaded { .. } => self.loaded_intent(now, intent),
            Ride::Free => self.free_intent(now, intent),
        }
    }
    /// An intent while a ride is loaded: the ride keeps its links (§5.1.3).
    fn loaded_intent(&mut self, now: MonoTime, intent: LinkIntent) -> Ack {
        let Ride::Loaded {
            id,
            started,
            input_at,
            discard,
        } = &mut self.ride
        else {
            return Ack::Applied;
        };
        let cause = match intent {
            // rider input on the Ready sheet
            LinkIntent::Likely { .. } | LinkIntent::Retry => {
                if !*started {
                    *input_at = now;
                }
                return Ack::Applied;
            }
            LinkIntent::Left
            | LinkIntent::Background
            | LinkIntent::Release {
                reason: ReleaseReason::Hidden | ReleaseReason::Devices,
            } => return Ack::Applied,
            LinkIntent::Release {
                reason: ReleaseReason::Sleep,
            } => Discard::Sleep,
            LinkIntent::Release {
                reason: ReleaseReason::RiderRelease,
            } => Discard::RiderRelease,
        };
        if *started {
            return if cause == Discard::RiderRelease {
                Ack::Refused(RIDE_HOLDS_TRAINER)
            } else {
                // sleep during a started ride is #51
                Ack::Applied
            };
        }
        if let Some(under_way) = discard {
            // joins the discard under way; "Release trainer" makes it latch
            if cause == Discard::RiderRelease {
                *under_way = cause;
            }
        } else {
            *discard = Some(cause);
            let id = id.clone();
            self.emit(Effect::Discard(id));
        }
        Ack::AwaitDiscard
    }
    /// An intent while the keeper is active.
    fn free_intent(&mut self, now: MonoTime, intent: LinkIntent) -> Ack {
        if self.shut {
            return match intent {
                LinkIntent::Likely { .. } | LinkIntent::Retry => Ack::Refused(QUITTING),
                // nothing is held after the quit
                LinkIntent::Left | LinkIntent::Background | LinkIntent::Release { .. } => {
                    Ack::Applied
                }
            };
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
        Ack::Applied
    }
    /// Apply what `Handed` recorded, at `now`, once the load resolved into `self.ride`; then
    /// answer it (the release's answer, when one was recorded).
    fn apply_recorded(&mut self, now: MonoTime, recorded: Recorded) {
        if !recorded.any {
            return;
        }
        let mut ack = Ack::Applied;
        // the other intent first: a release recorded beside it wins
        if let Some(intent) = recorded.other {
            ack = self.intent(now, intent);
        }
        if let Some(reason) = recorded.release {
            ack = self.intent(now, LinkIntent::Release { reason });
        }
        self.emit(Effect::Ack(ack));
    }
    /// `Handed` resolves: the ride was loaded.
    fn loaded(&mut self, now: MonoTime, id: String) {
        match &self.ride {
            Ride::Handed { recorded, .. } => {
                let recorded = *recorded;
                self.ride = Ride::Loaded {
                    id,
                    started: false,
                    input_at: now,
                    discard: None,
                };
                // a load that came meanwhile gets no links: this one is loaded
                if std::mem::replace(&mut self.waiting, false) {
                    self.emit(Effect::FailLoad);
                }
                self.apply_recorded(now, recorded);
            }
            // a load is installed only after its handover
            Ride::Free | Ride::Loaded { .. } => {}
        }
    }
    /// `Handed` resolves: the load is gone and the links are the keeper's again.
    fn refused(&mut self, now: MonoTime) {
        match &self.ride {
            Ride::Handed { recorded, stale } => {
                let (recorded, stale) = (*recorded, *stale);
                self.free(Phase::Warm);
                // the links came back for a load: they are held under its `Likely` (`T_cool`
                // lets them go), never with nothing due
                if self.intent == Intent::None {
                    self.intent = Intent::Likely {
                        ready: false,
                        since: now,
                    };
                }
                for d in DEVICES {
                    if stale[d as usize] {
                        self.cool(d, Cool::Unpaired);
                    }
                }
                // a trainer that comes back connected with a rider configuration that changed
                // meanwhile is reconnected (page 55 goes out only at connect)
                if self.devs[0].phase == Phase::Warm && self.trainer_rider != self.rider {
                    self.cool(LinkDevice::Trainer, Cool::Rider);
                    if self.likely() || self.waiting {
                        self.attempt(LinkDevice::Trainer, false);
                    }
                }
                if self.shut {
                    self.cool_all(Cool::Shutdown);
                }
                self.apply_recorded(now, recorded);
                self.resume_waiting(now);
            }
            Ride::Free | Ride::Loaded { .. } => {}
        }
    }
    /// The ride, or the load that was handed the links, let them go: they are `cold`.
    fn released(&mut self, now: MonoTime) {
        let recorded = match &self.ride {
            Ride::Free => return,
            Ride::Handed { recorded, .. } => *recorded,
            Ride::Loaded { discard, .. } => {
                // a discard by "Release trainer" that went through latches the trainer
                self.latched |= *discard == Some(Discard::RiderRelease);
                Recorded::default()
            }
        };
        self.free(Phase::Cold);
        self.intent = Intent::None;
        self.apply_recorded(now, recorded);
        self.resume_waiting(now);
    }
    /// A discard ended without discarding: the ride stays, and `T_ABANDON` and `Sleep` are
    /// armed again from now.
    fn discarded(&mut self, now: MonoTime, id: &str, discarded: bool) {
        if let Ride::Loaded {
            id: loaded,
            input_at,
            discard,
            ..
        } = &mut self.ride
            && loaded == id
            && !discarded
        {
            *discard = None;
            *input_at = now;
        }
    }
    /// A load that came while the links were still handed out gets them now.
    fn resume_waiting(&mut self, now: MonoTime) {
        if self.waiting && self.ride == Ride::Free {
            self.refresh(now);
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
    fn pairings(&mut self, ids: [Option<String>; 2]) {
        for (d, id) in DEVICES.into_iter().zip(ids) {
            if self.devs[d as usize].id == id {
                continue;
            }
            match &mut self.ride {
                Ride::Handed { stale, .. } if self.devs[d as usize].phase == Phase::Riding => {
                    stale[d as usize] = true;
                }
                // a ride's links go `cold` at its release, whatever they were paired as
                Ride::Free | Ride::Handed { .. } | Ride::Loaded { .. } => {
                    self.cool(d, Cool::Unpaired);
                }
            }
            let dev = &mut self.devs[d as usize];
            dev.id = id;
            dev.advert_window = None;
        }
        self.settle();
    }
    fn advertised(&mut self, now: MonoTime, id: &str) {
        if self.ride != Ride::Free || !self.likely() || self.shut {
            return;
        }
        let Some(window) = self.scan_window(now) else {
            return;
        };
        for d in DEVICES {
            let dev = &self.devs[d as usize];
            if dev.id.as_deref() == Some(id)
                && dev.phase == Phase::NotFound
                && !self.latched_out(d)
                && dev.advert_window != Some(window)
            {
                self.devs[d as usize].advert_window = Some(window);
                self.attempt(d, true);
            }
        }
    }
    /// A device became `not_found`: the scan cap starts again (continuous for
    /// `SCAN_CONTINUOUS`), with a new allowance of advertisement attempts.
    fn start_scan(&mut self, now: MonoTime) {
        self.scan_since = Some(now);
        for dev in &mut self.devs {
            dev.advert_window = None;
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
        for d in DEVICES {
            let dev = &self.devs[d as usize];
            let due = match dev.phase {
                Phase::Cold => true,
                Phase::NotFound => not_found,
                Phase::Searching
                | Phase::Connecting
                | Phase::Handshake
                | Phase::Warm
                | Phase::Riding => false,
            };
            if dev.id.is_some() && due && !self.latched_out(d) {
                self.attempt(d, false);
            }
        }
    }
    fn attempt(&mut self, d: LinkDevice, advertised: bool) {
        if d == LinkDevice::Trainer {
            self.trainer_rider = self.rider;
        }
        let dev = &mut self.devs[d as usize];
        dev.era += 1;
        dev.phase = Phase::Searching;
        dev.advertised = advertised;
        let era = dev.era;
        self.emit(Effect::Attempt(d, era));
    }
    /// Let `d` go if the keeper holds it or attempts it; it is `cold` after.
    fn cool(&mut self, d: LinkDevice, reason: Cool) {
        let dev = &mut self.devs[d as usize];
        if dev.phase == Phase::Riding {
            return;
        }
        let was = dev.phase;
        dev.phase = Phase::Cold;
        if was.held() {
            // a late result of the attempt let go is stale
            dev.era += 1;
            self.emit(Effect::Cool {
                device: d,
                was,
                reason,
            });
        }
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
                self.attempt(LinkDevice::Trainer, false);
            }
        }
    }
    /// A load wants the links. Only one load runs at a time (the core's `loading` flag): one
    /// that comes while another waits replaces it, which was cancelled. One that comes while
    /// the links are still handed to a load that has ended (its release or their return is
    /// reported on another channel, so it may come later) waits for them.
    fn acquire(&mut self, now: MonoTime) {
        match self.ride {
            Ride::Loaded { .. } => self.emit(Effect::FailLoad),
            Ride::Free | Ride::Handed { .. } if self.shut => self.emit(Effect::FailLoad),
            // a rider action: it supersedes the release recorded so far
            Ride::Handed {
                ref mut recorded, ..
            } => {
                if supersede(recorded) {
                    self.emit(Effect::Ack(Ack::Superseded));
                }
                self.waiting = true;
            }
            Ride::Free => {
                self.latched = false;
                self.refresh(now);
                self.waiting = true;
                self.warm_up(true);
                self.settle();
            }
        }
    }
    /// Answer a waiting load once the trainer has settled: hand over when it is `warm` and the
    /// strap has settled too, fail it when the trainer is not to be had.
    fn settle(&mut self) {
        if !self.waiting || self.ride != Ride::Free {
            return;
        }
        let [trainer, strap] = [&self.devs[0], &self.devs[1]].map(|d| d.phase);
        let strap_settled = self.devs[1].id.is_none() || !strap.attempting();
        if trainer == Phase::Warm && strap_settled {
            self.waiting = false;
            self.ride = Ride::Handed {
                recorded: Recorded::default(),
                stale: [false; 2],
            };
            for dev in &mut self.devs {
                if dev.phase == Phase::Warm {
                    dev.phase = Phase::Riding;
                }
            }
            self.emit(Effect::HandOver);
        } else if self.devs[0].id.is_none() || !trainer.held() {
            self.waiting = false;
            self.emit(Effect::FailLoad);
        }
    }
    fn tick(&mut self, now: MonoTime) {
        match &mut self.ride {
            Ride::Loaded {
                id,
                started: false,
                input_at,
                discard: discard @ None,
            } if now >= after(*input_at, T_ABANDON) => {
                *discard = Some(Discard::Abandoned);
                let id = id.clone();
                self.emit(Effect::Discard(id));
            }
            Ride::Loaded { .. } | Ride::Handed { .. } => {}
            Ride::Free => {
                if let Some((due, reason)) = self.cool_due()
                    && now >= due
                {
                    self.cool_all(reason);
                    self.intent = Intent::None;
                }
            }
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
    /// The capped retry scan at `now` (§5.1.2): on (`Some(window)`, 0 being the continuous
    /// scan) or off (`None`), and when that changes next; `None` when no scan is wanted.
    fn scan_at(&self, now: MonoTime) -> Option<(Option<u64>, MonoTime)> {
        let since = self.scan_since?;
        let wanted = !self.shut
            && self.ride == Ride::Free
            && self.likely()
            && DEVICES.into_iter().any(|d| {
                let dev = &self.devs[d as usize];
                dev.id.is_some() && dev.phase == Phase::NotFound && !self.latched_out(d)
            });
        if !wanted {
            return None;
        }
        let elapsed = now.0.saturating_sub(since.0);
        let (cont, period, window) = (us(SCAN_CONTINUOUS), us(SCAN_PERIOD), us(SCAN_WINDOW));
        if elapsed < cont {
            return Some((Some(0), MonoTime(since.0 + cont)));
        }
        // window k (from 1) opens `k` periods after the continuous scan ended
        let k = (elapsed - cont) / period;
        let start = since.0 + cont + k * period;
        Some(if k > 0 && now.0 - start < window {
            (Some(k), MonoTime(start + window))
        } else {
            (None, MonoTime(start + period))
        })
    }
    fn scan_window(&self, now: MonoTime) -> Option<u64> {
        self.scan_at(now).and_then(|(on, _)| on)
    }
    /// Whether the retry scan is on at `now`: `Some(true)` continuous, `Some(false)` a window.
    pub(crate) fn scanning(&self, now: MonoTime) -> Option<bool> {
        self.scan_window(now).map(|window| window == 0)
    }
    /// The next time something is due without an input: a cool-down, `T_ABANDON` or a change
    /// of the scan.
    pub(crate) fn next_wake(&self, now: MonoTime) -> Option<MonoTime> {
        let abandon = match &self.ride {
            Ride::Loaded {
                started: false,
                input_at,
                discard: None,
                ..
            } => Some(after(*input_at, T_ABANDON)),
            Ride::Loaded { .. } | Ride::Handed { .. } | Ride::Free => None,
        };
        let cool = (self.ride == Ride::Free)
            .then(|| self.cool_due().map(|(due, _)| due))
            .flatten();
        let scan = self.scan_at(now).map(|(_, change)| change);
        [abandon, cool, scan].into_iter().flatten().min()
    }
    /// Invariants every transition keeps (checked by the totality test).
    #[cfg(test)]
    pub(crate) fn invariant(&self) -> Result<(), String> {
        let held = DEVICES.iter().any(|&d| self.devs[d as usize].phase.held());
        let riding = DEVICES
            .iter()
            .any(|&d| self.devs[d as usize].phase == Phase::Riding);
        if riding && self.ride == Ride::Free {
            return Err("a device is riding with no ride".into());
        }
        if self.ride == Ride::Free && held && self.intent == Intent::None && !self.waiting {
            return Err("links held with no intent and no load: nothing would let them go".into());
        }
        if self.shut && DEVICES.iter().any(|&d| self.devs[d as usize].phase.held()) {
            return Err("links held after the quit".into());
        }
        if self.shut && self.waiting {
            return Err("a load waits after the quit".into());
        }
        // a load is a `Likely` (§5.1.4); only a cool-down or release takes the intent, and it
        // fails the load first
        if self.waiting && self.ride == Ride::Free && self.intent == Intent::None {
            return Err("a load waits with no intent".into());
        }
        if self.waiting && matches!(self.ride, Ride::Loaded { .. }) {
            return Err("a load waits beside a loaded ride".into());
        }
        if DEVICES
            .iter()
            .any(|&d| self.devs[d as usize].id.is_none() && self.devs[d as usize].phase.held())
        {
            return Err("an unpaired device is held".into());
        }
        Ok(())
    }
}
