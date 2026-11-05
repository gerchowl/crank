//! The keeper's task: it runs the [`Keeper`] over the clock and owns what its effects act on,
//! the connect attempts, the warm links, the retry scan and the waiting load.
//!
//! A warm link is held by a drain task that reads its events and drops them (§5.1.3: the
//! drivers `try_send` into bounded channels, so an unread link would fault) until the keeper
//! takes it back, to hand it over or release it, or the link is lost. Nothing is ever sent on
//! a warm trainer's control channel except the `Disconnect` that releases it. The task ends
//! when the core drops its [`Handle`]: every link and attempt it holds is dropped with it,
//! which the drivers' cleanup disconnects.
use super::{Cool, Effect, Input, Keeper, Phase};
use crate::dto::LinkIntent;
use crate::error::{ApiError, ErrorCode};
use crate::links::{self, Attempts, LinkDevice, Timeline};
use crank_devices::{
    Clock, DeviceError, DeviceRef, DiscoveredDevice, HeartRateDriver, HeartRateLink, HrEvent,
    LinkState, MonoTime, RiderConfig, TrainerCommand, TrainerDriver, TrainerEvent, TrainerLink,
};
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream::BoxStream};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

/// Longest the release of a keeper link is waited for: the driver's bounded disconnect
/// (`DISCONNECT_TIMEOUT`, 2 s, ADR-0002 §7.9).
pub(crate) const RELEASE_BOUND: Duration = Duration::from_secs(2);
const TARGET: &str = "crank_core::links";

/// What the keeper's task needs from the core.
pub(crate) trait Devices: Send + Sync + 'static {
    /// A trainer driver.
    fn trainer(&self) -> BoxFuture<'static, Result<Arc<dyn TrainerDriver>, DeviceError>>;
    /// A strap driver.
    fn heart(&self) -> BoxFuture<'static, Result<Arc<dyn HeartRateDriver>, DeviceError>>;
    /// A scan for advertisements.
    fn scan(&self)
    -> BoxFuture<'static, Result<BoxStream<'static, DiscoveredDevice>, DeviceError>>;
    /// End this loaded ride if it has not started (the path of Cancel).
    fn discard(&self, ride_id: String);
}

/// The links a load is handed (§5.1.4).
pub(crate) struct Handed {
    /// The trainer, `Ready`.
    pub(crate) trainer: TrainerLink,
    /// The strap, if it was warm.
    pub(crate) strap: Option<HeartRateLink>,
    /// How each device was found at the load: `warm`, `awaited` or `cold` (§5.1.6).
    pub(crate) how: [&'static str; 2],
    /// Dropping it tells the keeper the links are gone (the ride's end, or a failed load).
    pub(crate) hold: Hold,
}
/// A ride's hold on the links it was handed; the keeper is inactive while one exists.
pub(crate) struct Hold(Option<mpsc::UnboundedSender<Internal>>);
impl Drop for Hold {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(Internal::Released);
        }
    }
}

/// Facts the core tells the keeper.
pub(crate) enum Note {
    /// The paired platform ids changed.
    Pairings([Option<String>; 2]),
    /// The handed links' ride was loaded.
    Loaded(String),
    /// That ride started.
    Started(String),
}
enum Msg {
    Intent(LinkIntent, oneshot::Sender<()>),
    Acquire(RiderConfig, oneshot::Sender<Result<Handed, ApiError>>),
    Note(Note),
    Shutdown(oneshot::Sender<()>),
    /// The phases, for a test.
    #[cfg(test)]
    Phases(oneshot::Sender<[Option<Phase>; 2]>),
}
/// What the keeper's own tasks report.
pub(crate) enum Internal {
    Phase(LinkDevice, u32, Phase),
    Attempted(LinkDevice, u32, Result<Link, ApiError>),
    Lost(LinkDevice, u32),
    /// The links of a cancelled load, with their attempts.
    Returned(u32, TrainerLink, Option<(u32, HeartRateLink)>),
    Released,
}
pub(crate) enum Link {
    Trainer(TrainerLink),
    Strap(HeartRateLink),
}

/// What the task starts with: the devices, the clock, the rider configuration and the paired
/// ids.
pub(crate) type Setup = (
    Arc<dyn Devices>,
    Arc<dyn Clock>,
    RiderConfig,
    [Option<String>; 2],
);
/// The core's handle on its keeper task.
pub(crate) struct Handle {
    tx: mpsc::UnboundedSender<Msg>,
    rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<Msg>>>,
}
impl Handle {
    pub(crate) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: std::sync::Mutex::new(Some(rx)),
        }
    }
    /// Start the task, once; later calls do nothing. Messages sent before it are queued.
    /// `setup` gives what it starts with, the paired ids read only then.
    pub(crate) fn start<E>(&self, setup: impl FnOnce() -> Result<Setup, E>) -> Result<(), E> {
        let mut pending = self
            .rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.is_none() {
            return Ok(());
        }
        let (devices, clock, rider, pairings) = setup()?;
        let Some(rx) = pending.take() else {
            return Ok(());
        };
        let (internal, internal_rx) = mpsc::unbounded_channel();
        let mut task = Task {
            keeper: Keeper::new(rider),
            devices,
            attempts: Attempts::default(),
            internal,
            slots: Default::default(),
            waiting: None,
            scan: None,
            clock,
        };
        task.apply(Input::Pairings(pairings));
        tokio::spawn(task.run(rx, internal_rx));
        Ok(())
    }
    /// Tell the keeper a fact.
    pub(crate) fn note(&self, note: Note) {
        let _ = self.tx.send(Msg::Note(note));
    }
    /// Report an intent; returns once any release it causes is done (bounded).
    pub(crate) async fn intent(&self, intent: LinkIntent) {
        let (ack, done) = oneshot::channel();
        if self.tx.send(Msg::Intent(intent, ack)).is_ok() {
            let _ = done.await;
        }
    }
    /// The links for a load: the warm ones at once, running attempts awaited, the others
    /// connected now, trainer and strap in parallel.
    pub(crate) async fn acquire(&self, rider: RiderConfig) -> Result<Handed, ApiError> {
        let gone = || ApiError::new(ErrorCode::Internal, "the link keeper is gone");
        let (reply, answer) = oneshot::channel();
        self.tx
            .send(Msg::Acquire(rider, reply))
            .map_err(|_| gone())?;
        answer.await.map_err(|_| gone())?
    }
    /// Release every keeper link (no write of any kind) and take no new one; returns once the
    /// releases are done, each bounded by [`RELEASE_BOUND`].
    pub(crate) async fn shutdown(&self) {
        let (ack, done) = oneshot::channel();
        if self.tx.send(Msg::Shutdown(ack)).is_ok() {
            let _ = done.await;
        }
    }
    #[cfg(test)]
    pub(crate) async fn phases(&self) -> [Option<Phase>; 2] {
        let (ack, done) = oneshot::channel();
        let _ = self.tx.send(Msg::Phases(ack));
        done.await.unwrap_or([None, None])
    }
}

/// One device's attempt, warm link and last release.
#[derive(Default)]
struct Slot {
    attempt: Option<JoinHandle<()>>,
    held: Option<Held>,
    /// Done once the last release has finished: the next attempt waits for it.
    released: Option<watch::Receiver<bool>>,
    /// Why the last attempt failed.
    error: Option<ApiError>,
}
impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(attempt) = self.attempt.take() {
            attempt.abort();
        }
    }
}
/// A warm link, inside its drain task.
struct Held {
    era: u32,
    since: MonoTime,
    /// Dropped or fired: the drain task hands the link back.
    stop: oneshot::Sender<()>,
    drain: JoinHandle<Link>,
}
struct ScanRun {
    /// `None` once the stream ended: no restart until the scan is off again.
    stream: Option<BoxStream<'static, DiscoveredDevice>>,
    continuous: bool,
    since: MonoTime,
}
/// The waiting load: its reply, and how each device was found.
type Waiting = (oneshot::Sender<Result<Handed, ApiError>>, [&'static str; 2]);
struct Task {
    keeper: Keeper,
    devices: Arc<dyn Devices>,
    clock: Arc<dyn Clock>,
    attempts: Attempts,
    internal: mpsc::UnboundedSender<Internal>,
    slots: [Slot; 2],
    waiting: Option<Waiting>,
    scan: Option<ScanRun>,
}
fn ms(from: MonoTime, to: MonoTime) -> u64 {
    to.0.saturating_sub(from.0) / 1000
}
/// The next advertisement of a running scan; pending while none runs.
async fn advertised(scan: &mut Option<ScanRun>) -> DiscoveredDevice {
    if let Some(ScanRun {
        stream: stream @ Some(_),
        ..
    }) = scan
        && let Some(s) = stream.as_mut()
    {
        match s.next().await {
            Some(seen) => return seen,
            None => *stream = None,
        }
    }
    std::future::pending().await
}
/// Read and drop a warm link's events until it is lost.
async fn lost(link: &mut Link) {
    match link {
        Link::Trainer(t) => {
            while let Some(event) = t.events.recv().await {
                if event == TrainerEvent::Link(LinkState::Lost) {
                    return;
                }
            }
        }
        Link::Strap(s) => {
            while let Some(event) = s.events.recv().await {
                if event == HrEvent::Link(LinkState::Lost) {
                    return;
                }
            }
        }
    }
}
/// Release a link the keeper took back: the trainer's `Disconnect`, then its stream closing;
/// the strap's stream closed towards its driver, which disconnects.
async fn release(link: Link) {
    match link {
        Link::Trainer(mut t) => {
            let _ = t.control.try_send(TrainerCommand::Disconnect);
            while t.events.recv().await.is_some() {}
        }
        Link::Strap(mut s) => crank_devices::release_events(&mut s.events).await,
    }
}
fn phase_of(entered: &str) -> Option<Phase> {
    match entered {
        "gatt" | "identity" => Some(Phase::Connecting),
        "handshake" => Some(Phase::Handshake),
        _ => None,
    }
}

impl Task {
    async fn run(
        mut self,
        mut rx: mpsc::UnboundedReceiver<Msg>,
        mut internal: mpsc::UnboundedReceiver<Internal>,
    ) {
        loop {
            let now = self.clock.now();
            self.update_scan(now).await;
            let wake = self
                .keeper
                .next_wake(now)
                .map(|at| self.clock.sleep_until(at));
            let tick = async {
                match wake {
                    Some(sleep) => sleep.await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(msg) => self.message(msg),
                    None => break,
                },
                Some(event) = internal.recv() => self.internal(event),
                () = tick => drop(self.apply(Input::Tick)),
                seen = advertised(&mut self.scan) => {
                    drop(self.apply(Input::Advertised(&seen.device.0)));
                }
            }
        }
    }
    fn message(&mut self, msg: Msg) {
        match msg {
            Msg::Intent(intent, ack) => {
                tracing::info!(target: TARGET, ?intent, "intent");
                let releases = self.apply(Input::Intent(intent));
                acknowledge(releases, ack);
            }
            Msg::Acquire(rider, reply) => {
                let how =
                    [LinkDevice::Trainer, LinkDevice::Strap].map(|d| match self.keeper.phase(d) {
                        Some(Phase::Warm) => "warm",
                        Some(p) if p.attempting() => "awaited",
                        Some(_) => "cold",
                        None => "none",
                    });
                // (a waiting load that was cancelled is replaced: its reply is gone)
                self.waiting = Some((reply, how));
                drop(self.apply(Input::Rider(rider)));
                drop(self.apply(Input::Acquire));
            }
            Msg::Note(note) => drop(self.apply(match note {
                Note::Pairings(ids) => Input::Pairings(ids),
                Note::Loaded(id) => Input::Loaded(id),
                Note::Started(id) => Input::Started(id),
            })),
            Msg::Shutdown(ack) => {
                let releases = self.apply(Input::Shutdown);
                acknowledge(releases, ack);
            }
            #[cfg(test)]
            Msg::Phases(ack) => {
                let _ = ack
                    .send([LinkDevice::Trainer, LinkDevice::Strap].map(|d| self.keeper.phase(d)));
            }
        }
    }
    fn internal(&mut self, event: Internal) {
        let input = match event {
            Internal::Phase(d, era, phase) => Input::Phase(d, era, phase),
            Internal::Attempted(d, era, result) => {
                let current = self.keeper.current(d, era);
                if current {
                    self.slots[d as usize].attempt = None;
                }
                let linked = match result {
                    Ok(link) if current => {
                        self.slots[d as usize].held = Some(self.drain(d, era, link));
                        true
                    }
                    // a stale attempt's link is dropped, which releases it
                    Ok(_) => false,
                    Err(e) => {
                        self.slots[d as usize].error = Some(e);
                        false
                    }
                };
                Input::Attempted(d, era, linked)
            }
            Internal::Lost(d, era) => Input::Lost(d, era),
            Internal::Returned(era, trainer, strap) => {
                self.slots[0].held =
                    Some(self.drain(LinkDevice::Trainer, era, Link::Trainer(trainer)));
                if let Some((era, strap)) = strap {
                    self.slots[1].held =
                        Some(self.drain(LinkDevice::Strap, era, Link::Strap(strap)));
                }
                Input::Refused
            }
            Internal::Released => Input::Released,
        };
        drop(self.apply(input));
    }
    /// Step the keeper and act on its effects; returns the releases started.
    fn apply(&mut self, input: Input<'_>) -> Vec<watch::Receiver<bool>> {
        let now = self.clock.now();
        let mut releases = Vec::new();
        for effect in self.keeper.step(now, input) {
            match effect {
                Effect::Attempt(d, era) => self.attempt(d, era),
                Effect::Cool {
                    device,
                    was,
                    reason,
                } => {
                    releases.extend(self.cool(now, device, was, reason));
                }
                Effect::HandOver => self.hand_over(),
                Effect::FailLoad => self.fail_load(),
                Effect::Discard(id) => self.devices.discard(id),
            }
        }
        releases
    }
    fn attempt(&mut self, d: LinkDevice, era: u32) {
        let slot = &mut self.slots[d as usize];
        slot.error = None;
        let wait = slot.released.clone();
        let (devices, clock, internal) = (
            self.devices.clone(),
            self.clock.clone(),
            self.internal.clone(),
        );
        let (number, rider) = (self.attempts.next(d), self.keeper.rider());
        let dev = DeviceRef(self.keeper.id(d).unwrap_or_default().to_owned());
        slot.attempt = Some(tokio::spawn(async move {
            // a release still running holds the peripheral (and the driver's one connection)
            if let Some(mut released) = wait {
                let _ = released.wait_for(|done| *done).await;
            }
            let watch = internal.clone();
            let timeline = Timeline::watched(
                d,
                number,
                clock,
                Some(Box::new(move |entered| {
                    if let Some(phase) = phase_of(entered) {
                        let _ = watch.send(Internal::Phase(d, era, phase));
                    }
                })),
            );
            let timed_out = || {
                ApiError::new(
                    ErrorCode::DeviceUnavailable,
                    format!("{} connect timed out", d.as_str()),
                )
            };
            let limit = crate::core::CONNECT_TIMEOUT;
            let connect = async move {
                match d {
                    LinkDevice::Trainer => match devices.trainer().await {
                        Ok(driver) => links::attempt(timeline, limit, driver.connect(&dev, rider))
                            .await
                            .ok_or_else(timed_out)
                            .and_then(|r| r.map_err(ApiError::from))
                            .map(Link::Trainer),
                        Err(e) => Err(e.into()),
                    },
                    LinkDevice::Strap => match devices.heart().await {
                        Ok(driver) => links::attempt(timeline, limit, driver.connect(&dev))
                            .await
                            .ok_or_else(timed_out)
                            .and_then(|r| r.map_err(ApiError::from))
                            .map(Link::Strap),
                        Err(e) => Err(e.into()),
                    },
                }
            };
            // a driver that panics fails its attempt, as any other failure
            let result = std::panic::AssertUnwindSafe(connect)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(ApiError::new(
                        ErrorCode::Internal,
                        format!("the {} connect panicked", d.as_str()),
                    ))
                });
            let _ = internal.send(Internal::Attempted(d, era, result));
        }));
    }
    /// Spawn the drain task of a warm link.
    fn drain(&self, d: LinkDevice, era: u32, mut link: Link) -> Held {
        let (stop, mut stopped) = oneshot::channel::<()>();
        let internal = self.internal.clone();
        let drain = tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = &mut stopped => {}
                () = lost(&mut link) => {
                    let _ = internal.send(Internal::Lost(d, era));
                }
            }
            link
        });
        Held {
            era,
            since: self.clock.now(),
            stop,
            drain,
        }
    }
    /// Take a warm link back from its drain task.
    async fn take(held: Held) -> Option<(u32, Link)> {
        let _ = held.stop.send(());
        held.drain.await.ok().map(|link| (held.era, link))
    }
    /// Let a device go: cancel its attempt, or release its warm link in a task the next
    /// attempt waits for. Returns that release.
    fn cool(
        &mut self,
        now: MonoTime,
        d: LinkDevice,
        was: Phase,
        reason: Cool,
    ) -> Option<watch::Receiver<bool>> {
        let slot = &mut self.slots[d as usize];
        if let Some(attempt) = slot.attempt.take() {
            // dropping the connect releases a half-open peripheral (#304)
            attempt.abort();
        }
        let held = slot.held.take();
        tracing::info!(
            target: TARGET,
            device = d.as_str(),
            ?was,
            ?reason,
            held_s = held.as_ref().map_or(0, |h| ms(h.since, now) / 1000),
            "cool"
        );
        let held = held?;
        let (done_tx, done) = watch::channel(false);
        tokio::spawn(async move {
            if let Some((_, link)) = Self::take(held).await
                && tokio::time::timeout(RELEASE_BOUND, release(link))
                    .await
                    .is_err()
            {
                tracing::warn!(target: TARGET, device = d.as_str(), "a release timed out");
            }
            done_tx.send_replace(true);
        });
        slot.released = Some(done.clone());
        Some(done)
    }
    /// Give the warm links to the waiting load; if it is gone, they come back
    /// ([`Internal::Returned`]). The drain tasks hand them back at once, from a task, so that
    /// the keeper's loop never waits.
    fn hand_over(&mut self) {
        let Some((reply, how)) = self.waiting.take() else {
            return;
        };
        let held = [self.slots[0].held.take(), self.slots[1].held.take()];
        let internal = self.internal.clone();
        tokio::spawn(async move {
            let [trainer, strap] = held;
            let trainer = match trainer {
                Some(held) => Self::take(held).await,
                None => None,
            };
            let strap = match strap {
                Some(held) => Self::take(held).await,
                None => None,
            };
            let Some((trainer_gen, Link::Trainer(trainer))) = trainer else {
                let _ = reply.send(Err(ApiError::new(
                    ErrorCode::Internal,
                    "the warm trainer link was gone at the handover",
                )));
                drop(Hold(Some(internal)));
                return;
            };
            let (strap_gen, strap) = match strap {
                Some((era, Link::Strap(s))) => (era, Some(s)),
                _ => (0, None),
            };
            let handed = Handed {
                trainer,
                strap,
                how,
                hold: Hold(Some(internal.clone())),
            };
            if let Err(Ok(mut handed)) = reply.send(Ok(handed)) {
                // the load was cancelled before it took them
                handed.hold.0 = None;
                let strap = handed.strap.map(|s| (strap_gen, s));
                let _ = internal.send(Internal::Returned(trainer_gen, handed.trainer, strap));
            }
        });
    }
    /// Answer a waiting load with the trainer's failure.
    fn fail_load(&mut self) {
        if let Some((reply, _)) = self.waiting.take() {
            let error = self.slots[0].error.clone().unwrap_or_else(|| {
                ApiError::new(ErrorCode::DeviceUnavailable, "the trainer was released")
            });
            let _ = reply.send(Err(error));
        }
    }
    /// Start or stop the retry scan as the cap says, and log its radio time (§5.1.6).
    async fn update_scan(&mut self, now: MonoTime) {
        let wanted = self.keeper.scanning(now);
        match (&self.scan, wanted) {
            (None, Some(continuous)) => {
                let stream = match self.devices.scan().await {
                    Ok(stream) => Some(stream),
                    Err(error) => {
                        tracing::warn!(target: TARGET, %error, "the retry scan did not start");
                        None
                    }
                };
                self.scan = Some(ScanRun {
                    stream,
                    continuous,
                    since: now,
                });
            }
            (Some(run), None) => {
                let mode = if run.continuous {
                    "continuous"
                } else {
                    "window"
                };
                tracing::info!(target: TARGET, mode, ms = ms(run.since, now), "scan");
                self.scan = None;
            }
            _ => {}
        }
    }
}
/// Fire `ack` once every release in `releases` is done.
fn acknowledge(releases: Vec<watch::Receiver<bool>>, ack: oneshot::Sender<()>) {
    tokio::spawn(async move {
        for mut done in releases {
            let _ = done.wait_for(|d| *d).await;
        }
        let _ = ack.send(());
    });
}
