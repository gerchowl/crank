//! The [`Core`] facade (ADR-0001 §4).
//!
//! **Pairing.** Pairing remembers a peripheral: connect once to read its identity, then
//! release it (BLE allows one central, and nothing holds a device outside a ride).
//! Live readings before a ride are #267.
//!
//! **Ride.** `ride_load` connects the paired trainer (and strap, if any), builds a ride
//! engine over the links and loads the workout. The devices are released only when the
//! engine drops the ride-active gate, never on the `Finished` status: the engine first
//! puts the trainer into its end mode and holds the gate until that was delivered, the
//! link is lost, or a deadline passes (ADR-0002 §7.9, #269). Streams belong to the
//! loaded ride's engine and to a window label; they end with the ride.
use crate::backend::{Backend, DeviceMode};
use crate::dto::{
    CoreInfo, DeviceLinkInfo, DeviceRole, PairedDevice, RideLoaded, RideOutcome, RideRecord,
    RideStatusView, RideSummary, ScannedDevice, WorkoutDetail, WorkoutSummary,
};
use crate::error::{ApiError, ErrorCode};
use crate::library::Library;
use crate::recorder::{RideMeta, StoreRecorder};
use crank_ble::{Pairing, RideGate};
use crank_devices::{
    Clock, CommandSeq, ControlMode, DeviceInfo, DeviceRef, HeartRateLink, RiderConfig, SeqSource,
    TrainerCommand, TrainerEvent, TrainerLink,
};
use crank_domain::workout::{CompiledWorkout, Severity, Workout};
use crank_engine::{
    CommandError, Engine, EngineCmd, EngineHandle, MetricId, MonotonicClock, RideSetup, RideState,
    StreamFrame, StreamId, StreamSnapshot, StreamStatus, SubId, SubscribeOpts,
};
use crank_store::{Device, RideListing, Store};
use futures_util::StreamExt;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex, Notify, watch},
    task::JoinHandle,
};

/// Longest a connect may take before the command fails, so the UI never waits on a
/// device indefinitely.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a released link may take to close before pairing stops waiting.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(3);
/// Longest an ending engine may take to drop the ride-active signal after it was told to
/// stop: the end mode's own deadline is 2 s (ADR-0002 §7.9), so this only fires when the
/// engine task died with the signal up. The session is then reaped by force.
const REAP_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest `ride_end` waits for the engine to accept `Stop` (a healthy engine answers within
/// a tick), held under the state lock. A stuck or dead engine is then ended by force
/// ([`RideEnd::ForcedEnd`]). It is below `QUIT_CAP` (3 s, `app/src-tauri/src/lifecycle.rs`)
/// less the work after it (aborting the task, closing the record), so that a quit with a
/// stuck engine still ends and records the ride inside its cap. The whole path is bounded:
/// this, then `REAP_TIMEOUT` (10 s) for the engine to drop the ride-active signal, then
/// `RELEASE_TIMEOUT` (3 s) for the devices (not at all when quitting).
const STOP_TIMEOUT: Duration = Duration::from_secs(1);
/// Longest a forced end waits for the trainer to deliver the end mode (`Resistance{0}`,
/// ADR-0002 §7.9) that the core sends itself: one to two writer ticks (250 ms) of the
/// driver. With `STOP_TIMEOUT`, `ABORT_JOIN_TIMEOUT` and `STORE_TIMEOUT` it keeps a forced
/// end (about 2.25 s at worst) inside the 3 s quit cap.
const END_MODE_TIMEOUT: Duration = Duration::from_millis(500);
/// Longest a forced end waits for an aborted engine task to finish. A task blocked in
/// synchronous code (a store write) never does: it is then detached, not awaited.
const ABORT_JOIN_TIMEOUT: Duration = Duration::from_millis(250);
/// Longest a blocking store call on the end path is waited for.
const STORE_TIMEOUT: Duration = Duration::from_millis(500);
/// Longest the core waits for the engine to take or answer any other request: `Load`, the
/// ride commands and the stream requests (#351). A healthy engine answers between two ticks;
/// one that does not is wedged, and the caller gets [`ErrorCode::EngineUnresponsive`] instead
/// of a command that never returns. None of these waits holds the state lock, so End is never
/// behind one of them. The three `Stop`s (`ride_end`, the replacing `ride_load`, `discard`) are
/// the only engine requests under the lock, by design (no `ride_start` may reach the engine
/// first) and bounded by `STOP_TIMEOUT`.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);
/// Default scan length of `devices_scan`.
const SCAN_FOR: Duration = Duration::from_secs(5);
/// Rider and bike mass until athlete inputs exist (#247).
const RIDER: RiderConfig = RiderConfig {
    rider_kg: 72.0,
    bike_kg: 9.0,
    wheel_m: 2.096,
};

/// How to build a [`Core`].
pub struct CoreConfig {
    /// Directory of `crank.db` and `ride.lock`.
    pub data_dir: PathBuf,
    /// Real or simulated devices.
    pub devices: DeviceMode,
    /// Engine and driver clock; real monotonic time when `None`. Tests pass a
    /// `VirtualClock`.
    pub clock: Option<Arc<dyn Clock>>,
    /// Where each ride's engine runs: [`EngineHost::Thread`] in the application.
    pub engine_host: EngineHost,
}

/// Where a ride's engine runs (#344).
///
/// The core's deadlines on a ride end (`STOP_TIMEOUT`, `ABORT_JOIN_TIMEOUT`, `STORE_TIMEOUT`) are
/// tokio timers. An engine on the core's own runtime that blocks in synchronous code (a store
/// write) holds one of its worker threads, and with it, when that worker drives the timer, every
/// timer of the runtime: the deadlines would not fire until the engine returns. On its own thread
/// the engine cannot starve the core.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineHost {
    /// A dedicated OS thread with its own current-thread tokio runtime, the application's host.
    #[default]
    Thread,
    /// A task on the caller's runtime. For tests on paused time (`start_paused`): a
    /// `VirtualClock` sleeps on the timer of the runtime that polls it, so an engine on another
    /// runtime would not see the test's time.
    Task,
}
/// A running engine, as the core holds it: [`EngineTask::abort`] asks it to end, and
/// [`EngineTask::join`] waits for that, bounded. An engine that does not end (blocked in
/// synchronous code, which neither a task abort nor a signal can interrupt) is then dropped
/// from the session and left to finish on its own: detached.
enum EngineTask {
    /// A task on the core's runtime ([`EngineHost::Task`]).
    Tokio(JoinHandle<()>),
    /// A dedicated thread ([`EngineHost::Thread`]): `abort` makes its runtime drop the engine
    /// at its next await, and `done` turns true when the thread has ended, panics included.
    Thread {
        done: watch::Receiver<bool>,
        abort: Arc<Notify>,
    },
}
/// Sets the engine thread's `done` when the thread ends, however it ends.
struct Done(watch::Sender<bool>);
impl Drop for Done {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
impl EngineTask {
    /// Run `engine` on `host`.
    fn spawn(
        host: EngineHost,
        engine: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Result<Self, ApiError> {
        if host == EngineHost::Task {
            return Ok(Self::Tokio(tokio::spawn(engine)));
        }
        let failed =
            |e: std::io::Error| ApiError::new(ErrorCode::Internal, format!("engine thread: {e}"));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(failed)?;
        let (done, ended) = watch::channel(false);
        let abort = Arc::new(Notify::new());
        let aborted = abort.clone();
        std::thread::Builder::new()
            .name("crank-engine".into())
            .spawn(move || {
                // dropped last, after the runtime and the engine in it
                let _done = Done(done);
                runtime.block_on(async {
                    tokio::select! {
                        biased;
                        () = aborted.notified() => {}
                        () = engine => {}
                    }
                });
            })
            .map_err(failed)?;
        Ok(Self::Thread { done: ended, abort })
    }
    /// Ask the engine to end now: a task is aborted; a thread drops the engine at its next
    /// await, which a thread blocked in synchronous code reaches only once that returns.
    fn abort(&self) {
        match self {
            Self::Tokio(task) => task.abort(),
            Self::Thread { abort, .. } => abort.notify_one(),
        }
    }
    /// Wait at most `wait` for the engine to end; `false` if it has not.
    async fn join(&mut self, wait: Duration) -> bool {
        match self {
            Self::Tokio(task) => tokio::time::timeout(wait, task).await.is_ok(),
            // (an error is the thread's `done` gone: it has ended)
            Self::Thread { done, .. } => tokio::time::timeout(wait, done.wait_for(|d| *d))
                .await
                .is_ok(),
        }
    }
}

/// Where a window's frames go. Core never awaits it.
pub trait FrameSink: Send + 'static {
    /// Deliver one frame; an error ends and drops the stream (ADR-0001 §4.3).
    ///
    /// # Errors
    /// [`SinkClosed`] when the window can no longer receive.
    fn send(&self, frame: StreamFrame) -> Result<(), SinkClosed>;
}
impl<F: Fn(StreamFrame) -> Result<(), SinkClosed> + Send + 'static> FrameSink for F {
    fn send(&self, frame: StreamFrame) -> Result<(), SinkClosed> {
        self(frame)
    }
}
/// The window behind a [`FrameSink`] is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SinkClosed;

/// How [`Core::shutdown`] ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shutdown {
    /// No ride was loaded: nothing to end.
    Idle,
    /// The engine ended the ride: the end mode was delivered, the link was lost or its own
    /// deadline passed. The devices' drivers may still be handing their peripherals back;
    /// that is not awaited (process exit releases the radio).
    Ended,
    /// The engine had not ended the ride within the cap; the caller exits anyway.
    TimedOut,
    /// Ending the ride failed.
    Failed(ApiError),
}

/// How a ride ended, from [`Core::ride_end_outcome`].
///
/// The ride is over and its devices are on their way back either way; the difference is
/// what was recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RideEnd {
    /// The engine stopped the ride. `Some(id)` is a ride that ran and was recorded as
    /// finished; `None` an unstarted ride that was only unloaded.
    Ended(Option<String>),
    /// The engine did not answer `Stop` within `STOP_TIMEOUT` (stuck) or was gone (its
    /// task died). The core aborted it and closed the record as **aborted**: the seconds
    /// persisted so far are kept, there is no summary, and the trainer did not get its end
    /// mode from the engine (its link is dropped and disconnected, ADR-0002 §7.9).
    ForcedEnd(String),
}
impl From<RideEnd> for crate::dto::RideEnded {
    fn from(end: RideEnd) -> Self {
        Self {
            forced: matches!(end, RideEnd::ForcedEnd(_)),
            ride_id: end.ride_id().map(str::to_owned),
        }
    }
}
impl RideEnd {
    /// The ride's id: `None` only for an unstarted ride that was unloaded.
    #[must_use]
    pub fn ride_id(&self) -> Option<&str> {
        match self {
            Self::Ended(id) => id.as_deref(),
            Self::ForcedEnd(id) => Some(id),
        }
    }
}

/// What a forced end found of the ride's record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Closed {
    /// Closed by the force, as aborted.
    Aborted,
    /// The engine finished the ride after all (a late `Stop`): its record wins.
    Finished,
    /// No record to speak of (the ride never started) or the store did not answer.
    Unknown,
}

struct Session {
    handle: EngineHandle,
    /// Core stream id → (the engine's id of it, owning window). Core ids are unique for
    /// the process lifetime; the engine's restart with every ride.
    streams: BTreeMap<StreamId, (StreamId, Option<String>)>,
    /// The engine task or thread: ending it drops the device links.
    task: EngineTask,
    ride_id: String,
    /// Set once a driver has closed its event stream, i.e. handed the peripheral back
    /// (the same confirmation pairing waits for).
    release: LinkRelease,
    /// The core's own hold on the trainer, independent of the engine task, for the end mode
    /// of a forced end.
    trainer: TrainerHold,
    /// Raised by a forced end: the engine is cut off from its links and its ride-active
    /// signal, and then the links are released through the relays (see [`Takeover`]).
    takeover: Arc<watch::Sender<Takeover>>,
    /// The engine's own sender through the trainer's command gate, for a test to send what a
    /// waking engine would.
    #[cfg(test)]
    engine_control: tokio::sync::mpsc::Sender<TrainerCommand>,
}
/// How far a forced end has taken a session over from its engine (#331, #344). Each step
/// works through what the core owns (the relays, its own sender through the trainer's command
/// gate), so it works with the engine wedged on its thread, its links still held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Takeover {
    /// The engine has the session.
    Engine,
    /// The engine is cut off: the relays observe the drivers' events (the core waits for its
    /// end mode's `Delivered`) but no longer pass them on, the trainer's command gate drops
    /// the engine's commands ([`gate_commands`]), and the engine's ride-active signal is
    /// ignored, so that an engine waking up later cannot touch the trainer or the next ride's
    /// signal.
    CutOff,
    /// The core releases the devices: the relays stop and close the drivers' event streams,
    /// and the trainer is told to disconnect, whatever the engine still holds.
    Release,
}
/// What the core keeps of the trainer link so that it can end the ride's control itself
/// when the engine cannot (#331). Its sender is the core's own way through the command gate
/// ([`gate_commands`]), which never blocks it: it keeps the driver's control channel open
/// until the session is reaped, which follows the engine's end at once in every normal case.
#[derive(Clone)]
struct TrainerHold {
    control: tokio::sync::mpsc::Sender<TrainerCommand>,
    /// The link's sequences, shared with the engine: the core's end mode has one of its own.
    seq: SeqSource,
    /// The sequence of the last `SetMode` the driver reported as delivered.
    delivered: Arc<watch::Sender<Option<CommandSeq>>>,
    /// The engine commands the gate dropped after the takeover.
    #[cfg(test)]
    dropped: Arc<std::sync::Mutex<Vec<TrainerCommand>>>,
}
/// The release signals of a session's links.
///
/// Two different things are awaited when a ride is released, in this order: the **session
/// reap** (`wait_released`: the engine dropped the ride-active signal and its task ended),
/// then **the drivers' streams closing** (`LinkRelease::wait`: the trainer and the strap
/// handed their peripherals back). `discard` and `ride_end` await both, so a pairing
/// right after either finds the devices free; the second step is bounded by
/// [`RELEASE_TIMEOUT`].
#[derive(Clone)]
struct LinkRelease {
    trainer: watch::Receiver<bool>,
    hr: Option<watch::Receiver<bool>>,
}
impl LinkRelease {
    async fn wait(mut self) {
        let trainer = async {
            let _ = self.trainer.wait_for(|c| *c).await;
        };
        let hr = async {
            if let Some(hr) = self.hr.as_mut() {
                let _ = hr.wait_for(|c| *c).await;
            }
        };
        if tokio::time::timeout(RELEASE_TIMEOUT, async { tokio::join!(trainer, hr) })
            .await
            .is_err()
        {
            tracing::warn!("a device did not confirm release");
        }
    }
}
/// Pass a driver's events on to the engine and report, on the returned watch, when the
/// driver has closed its stream (the peripheral is handed back).
///
/// The relay outlives the engine, which drops its end when it ends, only to see the driver
/// close: at most [`RELEASE_TIMEOUT`] after the engine is gone, so a driver that never
/// closes cannot keep it alive. A strap has no control channel: its driver disconnects when
/// a send fails, so with `close_on_gone` the channel is closed towards the driver then
/// (see [`driver_closed`]). On [`Takeover::Release`] the relay does not wait for the engine:
/// it closes the channel towards the driver at once, for either device.
fn relay_events<E: Send + 'static>(
    mut driver_events: tokio::sync::mpsc::Receiver<E>,
    close_on_gone: bool,
    tap: impl Fn(&E) + Send + Sync + 'static,
    mut takeover: watch::Receiver<Takeover>,
) -> (tokio::sync::mpsc::Receiver<E>, watch::Receiver<bool>) {
    let (relay, events) = tokio::sync::mpsc::channel(256);
    let (closed_tx, closed) = watch::channel(false);
    tokio::spawn(async move {
        let mut released = false;
        loop {
            // (`Ok` patterns: once the session is gone, so is its takeover sender, and that
            // branch is off)
            let event = tokio::select! {
                event = driver_events.recv() => event,
                () = relay.closed() => break,
                Ok(_) = takeover.wait_for(|t| *t == Takeover::Release) => {
                    released = true;
                    break;
                }
            };
            // the driver closed first
            let Some(event) = event else { break };
            tap(&event);
            // Once the core has taken over (a forced end), the engine is cut off: events are
            // observed but no longer passed on, and a full channel to a stuck engine does
            // not hold the relay.
            if *takeover.borrow() != Takeover::Engine {
                continue;
            }
            tokio::select! {
                sent = relay.send(event) => if sent.is_err() { break },
                Ok(_) = takeover.wait_for(|t| *t != Takeover::Engine) => {}
            }
        }
        // The engine is gone, the driver ended, or the core releases the device: see the
        // driver close.
        if tokio::time::timeout(
            RELEASE_TIMEOUT,
            driver_closed(&mut driver_events, close_on_gone || released, &tap),
        )
        .await
        .is_err()
        {
            tracing::warn!("a driver did not close its stream after the engine");
        }
        closed_tx.send_replace(true);
    });
    (events, closed)
}
/// The trainer's command gate (#345): the only way to the driver's control channel, with two
/// senders into it, the engine's (handed to the engine in its link) and the core's own.
///
/// While the engine has the session, its commands are forwarded in order. From the takeover
/// ([`Takeover::CutOff`]) on, every engine command, whether sent after it or queued before
/// it and not yet forwarded, is **dropped and logged** with the ride id: an engine waking from
/// a wedge cannot put an ERG target back on a trainer the core has ended. The core's commands
/// (the forced end's `Resistance{0}`, its `Disconnect`) are never blocked.
///
/// Takeover is atomic with respect to the gate: one task forwards both senders, it looks at
/// the takeover after taking each engine command off its channel, and the core raises the
/// takeover before it sends anything. So once the core's end mode is forwarded, the takeover
/// was already seen, and nothing of the engine's follows it. An engine command that waits on
/// a full driver channel when the takeover comes is dropped too, which also keeps a stuck
/// engine's backlog from holding the core's end mode.
///
/// Failure modes: a driver that has closed its channel ends the gate, and the engine's sends
/// then fail as they would on the driver's own channel (its capacity is the driver's, so a
/// full channel still means a driver that is not running). The gate ends when both senders
/// are gone, which closes the driver's channel: the engine's end and the session's reap, as
/// before there was a gate.
fn gate_commands(
    ride_id: String,
    driver: tokio::sync::mpsc::Sender<TrainerCommand>,
    mut takeover: watch::Receiver<Takeover>,
    #[cfg(test)] dropped_log: Arc<std::sync::Mutex<Vec<TrainerCommand>>>,
) -> (
    tokio::sync::mpsc::Sender<TrainerCommand>,
    tokio::sync::mpsc::Sender<TrainerCommand>,
) {
    let (engine_tx, mut engine) = tokio::sync::mpsc::channel(driver.max_capacity());
    // the core sends its end mode and a disconnect, once per ride
    let (core_tx, mut core) = tokio::sync::mpsc::channel(4);
    let dropped = move |command: TrainerCommand| {
        tracing::warn!(
            ride_id,
            ?command,
            "the core has taken the ride over: an engine command is dropped"
        );
        #[cfg(test)]
        dropped_log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(command);
    };
    tokio::spawn(async move {
        let (mut engine_open, mut core_open) = (true, true);
        while engine_open || core_open {
            let command = tokio::select! {
                biased;
                command = core.recv(), if core_open => {
                    let Some(command) = command else {
                        core_open = false;
                        continue;
                    };
                    command
                }
                command = engine.recv(), if engine_open => {
                    let Some(command) = command else {
                        engine_open = false;
                        continue;
                    };
                    if *takeover.borrow() != Takeover::Engine {
                        dropped(command);
                        continue;
                    }
                    // (`Ok` pattern: once the session is gone, so is its takeover sender, and
                    // that branch is off)
                    tokio::select! {
                        biased;
                        sent = driver.send(command) => if sent.is_err() { break },
                        Ok(_) = takeover.wait_for(|t| *t != Takeover::Engine) => dropped(command),
                    }
                    continue;
                }
                () = driver.closed() => break,
            };
            if driver.send(command).await.is_err() {
                break;
            }
        }
    });
    (engine_tx, core_tx)
}
/// Put relays (see [`relay_events`]) between the drivers and the engine, which owns the links
/// and drops them when it ends: a closed stream is the driver's confirmation that the
/// peripheral is handed back. The trainer's commands go through the gate ([`gate_commands`]).
/// Returns the core's hold on the trainer and the release signals.
fn relay_links(
    ride_id: &str,
    trainer: &mut TrainerLink,
    hr: Option<&mut HeartRateLink>,
    takeover: &watch::Sender<Takeover>,
) -> (TrainerHold, LinkRelease) {
    #[cfg(test)]
    let dropped = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (engine_control, core_control) = gate_commands(
        ride_id.to_owned(),
        trainer.control.clone(),
        takeover.subscribe(),
        #[cfg(test)]
        dropped.clone(),
    );
    let hold = TrainerHold {
        control: core_control,
        seq: trainer.seq.clone(),
        delivered: Arc::new(watch::channel(None).0),
        #[cfg(test)]
        dropped,
    };
    trainer.control = engine_control;
    let delivered = hold.delivered.clone();
    let (events, trainer_closed) = relay_events(
        std::mem::replace(&mut trainer.events, tokio::sync::mpsc::channel(1).1),
        false,
        move |event: &TrainerEvent| {
            if let TrainerEvent::Delivered { seq, .. } = event {
                delivered.send_replace(Some(*seq));
            }
        },
        takeover.subscribe(),
    );
    trainer.events = events;
    let hr_closed = hr.map(|link| {
        let (events, closed) = relay_events(
            std::mem::replace(&mut link.events, tokio::sync::mpsc::channel(1).1),
            true,
            |_| {},
            takeover.subscribe(),
        );
        link.events = events;
        closed
    });
    let release = LinkRelease {
        trainer: trainer_closed,
        hr: hr_closed,
    };
    (hold, release)
}
/// The engine's ride-active signal, as the core takes it: into the gate and `active`, until the
/// core has taken the ride over ([`Takeover::CutOff`]); then the signal is the core's own.
///
/// The engine calls this on its own thread. The check and the set happen under the takeover's
/// read lock, which the core's `send_replace(CutOff)` waits for: once that returns, no call of
/// the cut-off engine is between its check and its set, so none can land after the core has
/// cleared the signal or a next ride has raised it.
fn ride_activity(
    cut_off: watch::Receiver<Takeover>,
    gate: RideGate,
    active: watch::Sender<bool>,
) -> impl Fn(bool) + Send + Sync + 'static {
    move |on: bool| {
        let takeover = cut_off.borrow();
        if *takeover != Takeover::Engine {
            return;
        }
        #[cfg(test)]
        tests::BETWEEN_CHECK_AND_SET.with(|hook| {
            if let Some(hook) = hook.borrow().as_ref() {
                hook();
            }
        });
        gate.set_active(on);
        active.send_replace(on);
        drop(takeover);
    }
}
/// Resolve once the driver has dropped its sender; with `close` the channel is closed towards
/// it first so that a strap's driver, which has no control channel, learns that its consumer
/// is gone (see [`crank_devices::release_events`]).
async fn driver_closed<E>(
    events: &mut tokio::sync::mpsc::Receiver<E>,
    close: bool,
    tap: &impl Fn(&E),
) {
    if close {
        crank_devices::release_events(events).await;
    } else {
        while let Some(event) = events.recv().await {
            tap(&event);
        }
    }
}
/// The engine's answer to a request, waited for at most [`REQUEST_TIMEOUT`].
async fn bounded<T, E: Into<ApiError>>(
    request: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, ApiError> {
    match tokio::time::timeout(REQUEST_TIMEOUT, request).await {
        Ok(answer) => answer.map_err(Into::into),
        Err(_) => Err(ApiError::new(
            ErrorCode::EngineUnresponsive,
            "the ride engine did not answer in time; End still ends the ride",
        )),
    }
}
/// Step 2 of [`Core::force_end`]: `true` once the driver has confirmed this very command.
///
/// The end mode has a sequence of its own, and only a `Delivered` that carries it confirms
/// it: a late `Delivered` of an earlier `Resistance{0}` (the engine's, from a pause or its
/// own end) says nothing about this one (#345). Unconfirmed after `END_MODE_TIMEOUT`, the
/// end goes on and logs it.
async fn force_end_mode(ride_id: &str, hold: &TrainerHold) -> bool {
    let mode = ControlMode::Resistance { pct: 0.0 };
    let seq = hold.seq.next();
    let mut delivered = hold.delivered.subscribe();
    let confirmed = tokio::time::timeout(END_MODE_TIMEOUT, async {
        hold.control
            .send(TrainerCommand::SetMode { mode, seq })
            .await
            .is_ok()
            && delivered.wait_for(|d| *d == Some(seq)).await.is_ok()
    })
    .await
    .unwrap_or(false);
    if !confirmed {
        tracing::warn!(
            ride_id,
            "the trainer did not confirm the end mode of a forced end"
        );
    }
    confirmed
}
/// Step 3 of [`Core::force_end`]: the relays stop and close the drivers' event streams (a
/// closed stream is what makes a trainer's driver return and a strap's disconnect), and the
/// trainer is told to disconnect through the core's own control sender. The engine, which may
/// hold its links for as long as it is wedged, is not involved.
fn release_links(ride_id: &str, hold: &TrainerHold, takeover: &watch::Sender<Takeover>) {
    takeover.send_replace(Takeover::Release);
    if hold.control.try_send(TrainerCommand::Disconnect).is_err() {
        // its closed event stream still releases it
        tracing::warn!(ride_id, "the trainer's control channel took no disconnect");
    }
}
#[derive(Default)]
struct State {
    session: Option<Session>,
    /// Id and final status of the last ride that finished, until the next load.
    last: Option<(String, StreamStatus)>,
    /// The release signals of the session reaped last, kept until the next load so that a
    /// second `ride_end` awaits the same release as the first.
    last_release: Option<LinkRelease>,
    /// Id of the last ride that was ended by force, until the next load, so that a second
    /// `ride_end` reports the same outcome as the first.
    last_forced: Option<String>,
    /// Id of the ride a forced end is in progress for: an overlapping `ride_end` waits for
    /// it instead of spending `STOP_TIMEOUT` on the same stuck engine again.
    forcing: Option<String>,
}
/// Clears the loading flag when a `ride_load` ends in any way, including its future
/// being dropped (a cancelled command) or panicking.
///
/// It also owns the built session between the engine accepting `Load` and the session
/// being installed in the state: if the load is cut off in that window, the guard hands
/// the session to a task that installs it and stops it, so the engine is released and
/// reaped like any unloaded ride, and the flag stays up until that is done (#278).
struct LoadGuard {
    core: Arc<Inner>,
    built: Option<Session>,
}
impl LoadGuard {
    fn new(core: Arc<Inner>) -> Self {
        Self { core, built: None }
    }
}
impl Drop for LoadGuard {
    fn drop(&mut self) {
        let core = self.core.clone();
        match (self.built.take(), tokio::runtime::Handle::try_current()) {
            (Some(session), Ok(runtime)) => {
                runtime.spawn(async move {
                    Core(core.clone()).discard(session).await;
                    core.loading.store(false, Ordering::SeqCst);
                });
            }
            // No built session, or no runtime left to wind it down (the engine ends when
            // its handle drops).
            _ => core.loading.store(false, Ordering::SeqCst),
        }
    }
}
struct Inner {
    store: Arc<Store>,
    backend: Backend,
    clock: Arc<dyn Clock>,
    engine_host: EngineHost,
    gate: RideGate,
    /// Mirrors the gate: the engine's ride-active signal, awaitable.
    active: watch::Sender<bool>,
    /// Id of the ride whose engine the ride-active signal (`gate`, `active`) belongs to: the
    /// one built last, set before it is sent `Load`, which raises the signal. Only that
    /// ride's end may clear the signal for it (#370, [`Core::clear_signal`]). Not in
    /// [`State`]: taking it never waits, so a load is not held before `Load` by the state lock.
    signal_of: std::sync::Mutex<Option<String>>,
    library: Library,
    /// The status of an engine that has nothing loaded.
    idle: StreamStatus,
    state: Mutex<State>,
    /// Woken whenever a session has been released.
    released: Notify,
    /// A ride is being loaded (devices connecting) outside the state lock. Atomic, not
    /// part of [`State`], so a [`LoadGuard`] can clear it from `Drop`.
    loading: AtomicBool,
    /// The next core stream id (never 0).
    next_stream: AtomicU32,
    /// Lets a test interleave a command into `discard` (#278).
    #[cfg(test)]
    discard_hook: Rendezvous,
    /// Lets a test hold an End between deciding to force the ride and forcing it (#370).
    #[cfg(test)]
    force_hook: Rendezvous,
    /// While set, the engine's callback ignores "inactive": an engine task that died with
    /// the ride-active signal up (#290).
    #[cfg(test)]
    pin_active: Arc<AtomicBool>,
    /// While on, the engine task is not polled: it holds its links and answers nothing (#331).
    #[cfg(test)]
    stall: Arc<Stall>,
    /// Blocks the engine's thread in a record write (#344).
    #[cfg(test)]
    block_write: Arc<BlockWrite>,
    /// Up while `reap` waits for the engine task to end (#345).
    #[cfg(test)]
    joining: AtomicBool,
}
/// A switch that suspends a future without dropping it.
#[cfg(test)]
#[derive(Default)]
struct Stall {
    on: AtomicBool,
    waker: std::sync::Mutex<Option<std::task::Waker>>,
}
#[cfg(test)]
impl Stall {
    fn release(&self) {
        self.on.store(false, Ordering::SeqCst);
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }
}
#[cfg(test)]
struct Stalled<F> {
    future: std::pin::Pin<Box<F>>,
    stall: Arc<Stall>,
}
#[cfg(test)]
impl<F: std::future::Future> std::future::Future for Stalled<F> {
    type Output = F::Output;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<F::Output> {
        if self.stall.on.load(Ordering::SeqCst) {
            *self.stall.waker.lock().unwrap() = Some(cx.waker().clone());
            return std::task::Poll::Pending;
        }
        self.future.as_mut().poll(cx)
    }
}
/// A record write that blocks the engine's thread, as a slow store would (#344).
#[cfg(test)]
#[derive(Default)]
struct BlockWrite {
    /// The next `append_second` sleeps (synchronously) this long, then writes.
    next: std::sync::Mutex<Option<Duration>>,
    /// Set once that write is blocked.
    blocked: AtomicBool,
    /// What the write returned once it woke.
    late: std::sync::Mutex<Option<Result<(), crank_engine::RecordError>>>,
}
#[cfg(test)]
struct BlockingRecorder {
    inner: StoreRecorder,
    block: Arc<BlockWrite>,
}
#[cfg(test)]
impl crank_engine::RideRecorder for BlockingRecorder {
    fn start(&mut self, metrics: &[&str]) -> Result<(), crank_engine::RecordError> {
        self.inner.start(metrics)
    }
    fn append_second(
        &mut self,
        session_s: u32,
        row: &crank_engine::SecondRow,
    ) -> Result<(), crank_engine::RecordError> {
        let Some(blocked) = self.block.next.lock().unwrap().take() else {
            return self.inner.append_second(session_s, row);
        };
        self.block.blocked.store(true, Ordering::SeqCst);
        std::thread::sleep(blocked);
        let late = self.inner.append_second(session_s, row);
        *self.block.late.lock().unwrap() = Some(late.clone());
        late
    }
    fn finish(&mut self) -> Result<(), crank_engine::RecordError> {
        self.inner.finish()
    }
}
/// A rendezvous between a test and the core: when enabled, the core announces that it has
/// reached the hook's point and waits to be resumed.
#[cfg(test)]
#[derive(Default)]
struct Rendezvous {
    enabled: AtomicBool,
    reached: Notify,
    resume: Notify,
}

/// The application API. Cheap to clone; one per process (it owns `ride.lock`).
#[derive(Clone)]
pub struct Core(Arc<Inner>);

#[expect(clippy::cast_precision_loss, reason = "UTC seconds are far below 2^53")]
fn secs(unix: i64) -> f64 {
    unix as f64
}
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}
fn paired(d: Device) -> Option<PairedDevice> {
    let role = match d.kind.as_str() {
        "trainer" => DeviceRole::Trainer,
        "hr" => DeviceRole::HeartRate,
        _ => return None,
    };
    Some(PairedDevice {
        role,
        id: d.platform_id,
        name: d.name,
        serial: d.serial,
        model: d.model,
    })
}
fn pairing(d: &Device) -> Pairing {
    Pairing::from_connection(
        &DeviceRef(d.platform_id.clone()),
        &DeviceInfo {
            name: d.name.clone(),
            model: d.model.clone(),
            serial: d.serial.clone(),
            firmware: None,
        },
    )
}

impl Core {
    /// Open the store as its owner and build the core. Must run inside a tokio runtime
    /// when devices are used.
    ///
    /// # Errors
    /// `Conflict` when another process holds `ride.lock`; `Storage` for any store failure.
    pub fn open(config: CoreConfig) -> Result<Self, ApiError> {
        let devices = config.devices;
        Self::open_with(
            &config.data_dir,
            config.clock,
            config.engine_host,
            move |pairings, clock| Backend::new(devices, clock, pairings),
        )
    }
    fn open_with(
        data_dir: &std::path::Path,
        clock: Option<Arc<dyn Clock>>,
        engine_host: EngineHost,
        backend: impl FnOnce(Vec<Pairing>, Arc<dyn Clock>) -> Backend,
    ) -> Result<Self, ApiError> {
        let store = Arc::new(Store::open_owner(data_dir)?);
        let clock = clock.unwrap_or_else(|| Arc::new(MonotonicClock::new()));
        // A restarted process starts with an empty adapter memory: the stored pairings
        // are handed to the radio as soon as it exists (ADR-0002 §5).
        let pairings = store.devices()?.iter().map(pairing).collect();
        Self::assemble(store, backend(pairings, clock.clone()), clock, engine_host)
    }
    fn assemble(
        store: Arc<Store>,
        backend: Backend,
        clock: Arc<dyn Clock>,
        engine_host: EngineHost,
    ) -> Result<Self, ApiError> {
        let clock_for_idle = clock.clone();
        Ok(Self(Arc::new(Inner {
            store,
            backend,
            clock,
            engine_host,
            gate: RideGate::new(false),
            active: watch::channel(false).0,
            signal_of: std::sync::Mutex::default(),
            library: Library::builtin()?,
            idle: {
                let (_engine, handle) =
                    Engine::new(clock_for_idle, None, None, Arc::new(|_: bool| {}));
                StreamStatus::from(&handle.status())
            },
            state: Mutex::default(),
            released: Notify::new(),
            loading: AtomicBool::new(false),
            next_stream: AtomicU32::new(1),
            #[cfg(test)]
            discard_hook: Rendezvous::default(),
            #[cfg(test)]
            force_hook: Rendezvous::default(),
            #[cfg(test)]
            pin_active: Arc::default(),
            #[cfg(test)]
            stall: Arc::default(),
            #[cfg(test)]
            block_write: Arc::default(),
            #[cfg(test)]
            joining: AtomicBool::new(false),
        })))
    }

    /// Where this core runs each ride's engine.
    #[must_use]
    pub fn engine_host(&self) -> EngineHost {
        self.0.engine_host
    }

    /// Versions and mode.
    ///
    /// # Errors
    /// `Storage` if the schema version cannot be read.
    #[expect(
        clippy::unused_async,
        reason = "every facade method is async (ADR-0001 §4)"
    )]
    pub async fn core_ready(&self) -> Result<CoreInfo, ApiError> {
        Ok(CoreInfo {
            version: env!("CARGO_PKG_VERSION").into(),
            sim: self.0.backend.is_sim(),
            schema_version: self.0.store.schema_version()?,
        })
    }

    /// Pairing, forgetting and scanning need the one BLE central and the pairing rows a
    /// ride uses: not while a ride is loaded, in progress or still ending.
    async fn require_no_ride(&self) -> Result<(), ApiError> {
        let st = self.0.state.lock().await;
        if st.session.is_some() || self.0.loading.load(Ordering::SeqCst) {
            return Err(ApiError::new(
                ErrorCode::Conflict,
                "a ride is loaded: end it first",
            ));
        }
        Ok(())
    }

    /// Scan for trainers and straps for `for_ms` (default 5 s, 0.2–15 s), or until the
    /// scan source ends.
    ///
    /// A peripheral is listed once, with the union of what its advertisements showed.
    /// While another scan holds the central the list is empty; a typed busy result comes
    /// with #265 and maps to `Conflict` then.
    ///
    /// # Errors
    /// `Conflict` while a ride is loaded (one central), `PermissionDenied` or
    /// `DeviceUnavailable` from the adapter.
    pub async fn devices_scan(&self, for_ms: Option<u32>) -> Result<Vec<ScannedDevice>, ApiError> {
        self.require_no_ride().await?;
        let limit = for_ms.map_or(SCAN_FOR, |ms| {
            Duration::from_millis(u64::from(ms.clamp(200, 15_000)))
        });
        let known: Vec<String> = self
            .0
            .store
            .devices()?
            .into_iter()
            .map(|d| d.platform_id)
            .collect();
        let mut scan = self.0.backend.scan().await?;
        let mut found: Vec<ScannedDevice> = Vec::new();
        // The scan source may never end; the deadline is the normal way out.
        let _ = tokio::time::timeout(limit, async {
            while let Some(d) = scan.next().await {
                let seen = ScannedDevice {
                    paired: known.contains(&d.device.0),
                    id: d.device.0,
                    name: d.info.name,
                    trainer: d.trainer.is_some(),
                    heart_rate: d.heart_rate,
                };
                match found.iter_mut().find(|f| f.id == seen.id) {
                    Some(f) => {
                        f.name = seen.name.or(f.name.take());
                        f.trainer |= seen.trainer;
                        f.heart_rate |= seen.heart_rate;
                    }
                    None => found.push(seen),
                }
            }
        })
        .await;
        Ok(found)
    }

    /// Connect to `id` once to read its identity, remember it for `role` (replacing the
    /// previous pairing of that role) and release it.
    ///
    /// # Errors
    /// `Conflict` while a ride is loaded, `DeviceUnavailable` / `PermissionDenied` when the connect
    /// fails or exceeds 15 s,
    /// `InvalidInput` for an empty id or one that cannot act in `role`, `Storage`.
    pub async fn device_pair(
        &self,
        id: String,
        role: DeviceRole,
    ) -> Result<PairedDevice, ApiError> {
        self.require_no_ride().await?;
        if id.is_empty() {
            return Err(ApiError::new(ErrorCode::InvalidInput, "empty device id"));
        }
        if !self.0.backend.accepts(&id, role) {
            return Err(ApiError::new(
                ErrorCode::InvalidInput,
                format!("device {id:?} cannot be paired as {role:?}"),
            ));
        }
        let dev = DeviceRef(id.clone());
        let connect = async {
            match role {
                DeviceRole::Trainer => {
                    let driver = self
                        .0
                        .backend
                        .trainer(self.0.clock.clone(), self.0.gate.clone())
                        .await?;
                    let link = driver.connect(&dev, RIDER).await?;
                    let _ = link.control.send(TrainerCommand::Disconnect).await;
                    Ok::<_, ApiError>((link.info, Some(link.events)))
                }
                DeviceRole::HeartRate => {
                    let driver = self
                        .0
                        .backend
                        .heart(self.0.clock.clone(), self.0.gate.clone())
                        .await?;
                    // The strap has no command channel: dropping its link releases it.
                    Ok((driver.connect(&dev).await?.info, None))
                }
            }
        };
        let (info, events) = tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| ApiError::new(ErrorCode::DeviceUnavailable, "connect timed out"))??;
        if let Some(mut events) = events {
            // The trainer driver closes its stream once the peripheral is handed back:
            // wait for that before anyone connects again.
            let closed = async { while events.recv().await.is_some() {} };
            if tokio::time::timeout(RELEASE_TIMEOUT, closed).await.is_err() {
                tracing::warn!(id, "trainer did not confirm release");
            }
        }
        let kind = role.kind();
        let db_id = self.0.store.remember_device(Device {
            id: format!("{kind}:{id}"),
            kind: kind.into(),
            name: info.name,
            platform_id: id,
            serial: info.serial,
            model: info.model,
            last_seen: unix_now(),
        })?;
        // One trainer and one strap: a new pairing replaces the old one of its role.
        let devices = self.0.store.devices()?;
        for other in devices.iter().filter(|d| d.kind == kind && d.id != db_id) {
            self.0.store.forget_device(&other.id)?;
            self.0.backend.forget(&other.platform_id);
        }
        let kept = devices
            .into_iter()
            .find(|d| d.id == db_id)
            .ok_or_else(|| ApiError::new(ErrorCode::Internal, "pairing vanished"))?;
        self.0.backend.remember(pairing(&kept));
        paired(kept).ok_or_else(|| ApiError::new(ErrorCode::Internal, "unknown device kind"))
    }

    /// Forget the pairing of `role`.
    ///
    /// # Errors
    /// `Conflict` while a ride is loaded; `NotFound` when nothing is paired for it.
    pub async fn device_forget(&self, role: DeviceRole) -> Result<(), ApiError> {
        self.require_no_ride().await?;
        let kind = role.kind();
        let mut any = false;
        for d in self
            .0
            .store
            .devices()?
            .into_iter()
            .filter(|d| d.kind == kind)
        {
            any |= self.0.store.forget_device(&d.id)?;
            self.0.backend.forget(&d.platform_id);
        }
        any.then_some(())
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "nothing paired for that role"))
    }

    /// The pairing and live link of each role; the link is the loaded ride's, `null`
    /// without one and once it has finished.
    ///
    /// # Errors
    /// `Storage`.
    pub async fn device_links(&self) -> Result<Vec<DeviceLinkInfo>, ApiError> {
        let devices = self.0.store.devices()?;
        let live = self
            .0
            .state
            .lock()
            .await
            .session
            .as_ref()
            .map(|s| s.handle.status())
            .filter(|s| s.state != RideState::Finished)
            .map(|s| (s.trainer_link, s.hr_link));
        Ok([DeviceRole::Trainer, DeviceRole::HeartRate]
            .into_iter()
            .map(|role| DeviceLinkInfo {
                role,
                device: devices
                    .iter()
                    .find(|d| d.kind == role.kind())
                    .cloned()
                    .and_then(paired),
                link: live
                    .and_then(|(t, h)| if role == DeviceRole::Trainer { t } else { h })
                    .map(Into::into),
            })
            .collect())
    }

    /// The workout library.
    ///
    /// # Errors
    /// Never fails today; async and fallible for the store-backed library (#110).
    #[expect(
        clippy::unused_async,
        reason = "every facade method is async (ADR-0001 §4)"
    )]
    pub async fn workouts_list(&self) -> Result<Vec<WorkoutSummary>, ApiError> {
        Ok(self.0.library.summaries())
    }

    /// One workout with its DSL and compiled timeline.
    ///
    /// # Errors
    /// `NotFound`.
    #[expect(
        clippy::unused_async,
        reason = "every facade method is async (ADR-0001 §4)"
    )]
    pub async fn workout_get(&self, id: String) -> Result<WorkoutDetail, ApiError> {
        self.0.library.detail(&id)
    }

    // ---- ride -------------------------------------------------------------------

    /// The one place ride start asks "may this rider ride?": the medical-stop guard is
    /// #81 (owner decision pending); it plugs in here and nowhere else.
    #[expect(
        clippy::unused_self,
        clippy::unnecessary_wraps,
        reason = "the #81 seam"
    )]
    fn ensure_start_allowed(&self) -> Result<(), ApiError> {
        Ok(())
    }

    async fn connect_links(
        &self,
    ) -> Result<(TrainerLink, Option<HeartRateLink>, [Option<String>; 2]), ApiError> {
        let devices = self.0.store.devices()?;
        let find = |kind: &str| devices.iter().find(|d| d.kind == kind);
        let trainer = find("trainer")
            .ok_or_else(|| ApiError::new(ErrorCode::DeviceUnavailable, "no trainer paired"))?;
        let (clock, gate) = (self.0.clock.clone(), self.0.gate.clone());
        let driver = self.0.backend.trainer(clock.clone(), gate.clone()).await?;
        let link = tokio::time::timeout(
            CONNECT_TIMEOUT,
            driver.connect(&DeviceRef(trainer.platform_id.clone()), RIDER),
        )
        .await
        .map_err(|_| ApiError::new(ErrorCode::DeviceUnavailable, "trainer connect timed out"))??;
        let mut hr = None;
        let mut hr_id = None;
        if let Some(strap) = find("hr") {
            let driver = self.0.backend.heart(clock, gate).await?;
            let connect = driver.connect(&DeviceRef(strap.platform_id.clone()));
            match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
                Ok(Ok(l)) => {
                    hr = Some(l);
                    hr_id = Some(strap.id.clone());
                }
                Ok(Err(e)) => tracing::warn!(%e, "strap unavailable: riding without heart rate"),
                Err(_) => tracing::warn!("strap connect timed out: riding without heart rate"),
            }
        }
        Ok((link, hr, [Some(trainer.id.clone()), hr_id]))
    }

    /// Release the session of `ride_id` once the engine has dropped the ride-active gate
    /// (or its task ended): drop the handle, wait for the engine to end and its links to
    /// close, keep the final status of a finished ride.
    ///
    /// The state lock is held only to take the session out, never across the wait for the
    /// engine task, and that wait is bounded: the task gets `RELEASE_TIMEOUT` to end after
    /// its handle is gone, or with `force` (the engine did not release the ride in time) is
    /// aborted and given `ABORT_JOIN_TIMEOUT`. A task that is still there after that is
    /// blocked in synchronous code, which `abort` cannot interrupt: it is detached, not
    /// awaited.
    async fn reap(&self, ride_id: &str, force: bool) {
        let task = {
            let mut st = self.0.state.lock().await;
            if st.session.as_ref().is_some_and(|s| s.ride_id == ride_id)
                && let Some(session) = st.session.take()
            {
                let status = session.handle.status();
                st.last = (status.state == RideState::Finished)
                    .then(|| (session.ride_id.clone(), StreamStatus::from(&status)));
                st.last_release = Some(session.release.clone());
                drop(session.handle);
                if force {
                    session.task.abort();
                }
                Some(session.task)
            } else {
                None
            }
        };
        if let Some(mut task) = task {
            let wait = if force {
                ABORT_JOIN_TIMEOUT
            } else {
                RELEASE_TIMEOUT
            };
            #[cfg(test)]
            self.0.joining.store(true, Ordering::SeqCst);
            let joined = task.join(wait).await;
            #[cfg(test)]
            self.0.joining.store(false, Ordering::SeqCst);
            if !joined {
                if force {
                    // aborted above, and still not ended
                    tracing::warn!(ride_id, "the engine ignores the abort: detaching it");
                } else {
                    tracing::warn!(ride_id, "the engine did not end: aborting it");
                    task.abort();
                    if !task.join(ABORT_JOIN_TIMEOUT).await {
                        tracing::warn!(ride_id, "the engine ignores the abort: detaching it");
                    }
                }
            }
        }
        self.0.released.notify_waiters();
    }
    /// Wait until `ride_id` has no session.
    async fn wait_released(&self, ride_id: &str) {
        loop {
            let woken = self.0.released.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            let held = self
                .0
                .state
                .lock()
                .await
                .session
                .as_ref()
                .is_some_and(|s| s.ride_id == ride_id);
            if !held {
                return;
            }
            woken.await;
        }
    }

    /// The workout and compiled timeline to ride; `None` is a free ride.
    fn resolve_workout(&self, id: Option<&str>) -> Result<(Workout, CompiledWorkout), ApiError> {
        let Some(id) = id else {
            let workout = Workout {
                id: "free-ride".into(),
                name: "Free ride".into(),
                ftp: 0.0,
                nodes: Vec::new(),
            };
            let compiled = CompiledWorkout {
                blocks: Vec::new(),
                total_s: 0.0,
                total_range_s: [0.0, 0.0],
                tss: 0.0,
                intensity_factor: 0.0,
                time_in_zone_s: [0; 7],
                issues: Vec::new(),
            };
            return Ok((workout, compiled));
        };
        let e = self.0.library.get(id)?;
        if let Some(issue) = e
            .compiled
            .issues
            .iter()
            .find(|i| i.severity == Severity::Error)
        {
            return Err(ApiError::new(
                ErrorCode::InvalidInput,
                issue.message.clone(),
            ));
        }
        Ok((e.workout.clone(), e.compiled.clone()))
    }

    /// Load a workout (or a free ride for `None`): connect the paired devices, build
    /// the engine and load the workout. The ride starts with `ride_start`.
    ///
    /// A request that is invalid (unknown workout, a workout with errors) changes
    /// nothing. Otherwise the load first replaces any loaded-but-unstarted ride (stops
    /// it and waits for its devices): if connecting then fails, nothing is loaded and
    /// the status is idle. A cancelled or panicking load leaves the core usable.
    ///
    /// # Errors
    /// `Conflict` if a ride is in progress; `NotFound` for an unknown workout;
    /// `DeviceUnavailable` without a paired or reachable trainer; `EngineUnresponsive` if the
    /// new engine does not take the ride within 1 s (its devices are then released).
    pub async fn ride_load(&self, workout_id: Option<String>) -> Result<RideLoaded, ApiError> {
        // Nothing is touched until the request is known to be good: a failed load leaves
        // the previous status as it was.
        let (workout, compiled) = self.resolve_workout(workout_id.as_deref())?;
        self.ensure_start_allowed()?;
        // A loaded-but-unstarted or finished ride is replaced, but only once it has
        // released its devices.
        loop {
            let ending = {
                let st = self.0.state.lock().await;
                match &st.session {
                    None => None,
                    Some(s) => {
                        if StreamStatus::from(&s.handle.status()).in_progress() {
                            return Err(ApiError::new(
                                ErrorCode::Conflict,
                                "a ride is in progress",
                            ));
                        }
                        // `Stop` unloads a loaded ride; a finished one is refused, harmlessly.
                        // Bounded like the one of `ride_end`: an engine that does not
                        // answer is ended by force below, not waited for under the lock.
                        let stopped =
                            tokio::time::timeout(STOP_TIMEOUT, s.handle.send(EngineCmd::Stop))
                                .await;
                        let stuck =
                            matches!(stopped, Err(_) | Ok(Err(CommandError::EngineStopped)));
                        Some((s.ride_id.clone(), s.release.clone(), stuck))
                    }
                }
            };
            match ending {
                Some((id, release, stuck)) => {
                    if stuck {
                        self.force_end(&id, false).await;
                    } else {
                        self.settle(&id).await;
                    }
                    release.wait().await;
                }
                None => break,
            }
        }
        {
            let st = self.0.state.lock().await;
            if st.session.is_some() || self.0.loading.swap(true, Ordering::SeqCst) {
                return Err(ApiError::new(
                    ErrorCode::Conflict,
                    "another ride is loading or loaded",
                ));
            }
        }
        // From here every exit (error, cancellation, panic) clears the flag.
        let mut loading = LoadGuard::new(self.0.clone());
        // Connecting takes seconds: it runs outside the state lock, and `ride_status`
        // reports `loading` meanwhile.
        let built = self.build_session(workout, compiled).await;
        let (session, hr_linked) = built?;
        // The lock below may be contended, and waiting for it is a cancellation point:
        // until the session is installed the guard owns it.
        loading.built = Some(session);
        let mut st = self.0.state.lock().await;
        let session = loading
            .built
            .take()
            .ok_or_else(|| ApiError::new(ErrorCode::Internal, "built session missing"))?;
        st.last = None;
        st.last_release = None;
        st.last_forced = None;
        st.forcing = None;
        let ride_id = session.ride_id.clone();
        st.session = Some(session);
        drop(st);
        self.spawn_reaper(&ride_id);
        Ok(RideLoaded {
            ride_id,
            workout_id,
            hr_linked,
        })
    }

    /// `Load` raised the ride-active signal; the ride's devices are released when it
    /// falls again. The watcher holds the core weakly: dropping the last `Core` handle
    /// drops the engine handle, which ends and finishes the ride (ADR-0001 §5).
    fn spawn_reaper(&self, ride_id: &str) {
        let mut active = self.0.active.subscribe();
        let (weak, id) = (Arc::downgrade(&self.0), ride_id.to_owned());
        tokio::spawn(async move {
            let _ = active.wait_for(|on| !*on).await;
            if let Some(inner) = weak.upgrade() {
                Core(inner).reap(&id, false).await;
            }
        });
    }
    /// Unload a session whose `ride_load` was cancelled before it was installed: stop it,
    /// install it so that it is replaced like any loaded ride, and wait until its devices
    /// are released: the engine has ended and the trainer driver has closed its event
    /// stream, bounded by [`RELEASE_TIMEOUT`] as for pairing.
    ///
    /// `Stop` is sent under the state lock, before the session is visible, so that no
    /// `ride_start` can reach the engine first and open a ride row (as in the replace
    /// path of `ride_load` and in `ride_end`). As there, it relies on `Stop` always
    /// dropping the ride-active gate of a loaded ride, which is what fires the reaper.
    async fn discard(&self, session: Session) {
        let ride_id = session.ride_id.clone();
        let release = session.release.clone();
        let stuck = {
            let mut st = self.0.state.lock().await;
            // Bounded (see `ride_end`): a stuck engine is ended by force below.
            let stopped =
                tokio::time::timeout(STOP_TIMEOUT, session.handle.send(EngineCmd::Stop)).await;
            st.session = Some(session);
            matches!(stopped, Err(_) | Ok(Err(CommandError::EngineStopped)))
        };
        #[cfg(test)]
        if self.0.discard_hook.enabled.load(Ordering::SeqCst) {
            self.0.discard_hook.reached.notify_one();
            self.0.discard_hook.resume.notified().await;
        }
        self.spawn_reaper(&ride_id);
        if stuck {
            self.force_end(&ride_id, false).await;
        } else {
            self.settle(&ride_id).await;
        }
        release.wait().await;
    }
    /// Wait until `ride_id` has no session, for at most [`REAP_TIMEOUT`]. If the engine
    /// never drops the ride-active signal (its task died with it up), the session is reaped
    /// by force and the signal cleared, so a command or the `loading` flag cannot be left
    /// waiting for ever.
    async fn settle(&self, ride_id: &str) {
        if tokio::time::timeout(REAP_TIMEOUT, self.wait_released(ride_id))
            .await
            .is_err()
        {
            tracing::warn!(ride_id, "the engine did not release the ride: reaping it");
            self.reap(ride_id, true).await;
            self.clear_signal(ride_id);
        }
    }
    /// Clear the ride-active signal that the engine of `ride_id` can no longer drop, if it is
    /// still that ride's (`signal_of`); `true` if it was.
    ///
    /// The gate and `active` are the core's, one pair for every ride. An end that clears them
    /// runs outside the state lock and can run late: an End and a replacing `ride_load` both
    /// force the same stuck ride, the load's forced end finishes first and the load builds the
    /// newer ride, whose `Load` raises the signal. Cleared then, the newer trainer's command
    /// gate would be off and the newer ride's reaper would unload it (#370). The newer engine
    /// takes the signal over (`signal_of`, set before its `Load`), and the check and the clear
    /// run under that same lock: a clear either runs before the newer engine can raise the
    /// signal, or not at all.
    fn clear_signal(&self, ride_id: &str) -> bool {
        let owner = self
            .0
            .signal_of
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let own = owner.as_deref() == Some(ride_id);
        if own {
            self.0.gate.set_active(false);
            self.0.active.send_replace(false);
        }
        drop(owner);
        own
    }

    /// Connect the devices, build the engine and load the workout into it.
    async fn build_session(
        &self,
        workout: Workout,
        compiled: CompiledWorkout,
    ) -> Result<(Session, bool), ApiError> {
        let (mut trainer, mut hr, [trainer_device_id, hr_device_id]) = self.connect_links().await?;
        let ride_id = format!(
            "ride-{:x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        let takeover = Arc::new(watch::channel(Takeover::Engine).0);
        let (hold, release) = relay_links(&ride_id, &mut trainer, hr.as_mut(), &takeover);
        let hr_linked = hr.is_some();
        let (gate, active) = (self.0.gate.clone(), self.0.active.clone());
        #[cfg(test)]
        let pin = self.0.pin_active.clone();
        let activity = ride_activity(takeover.subscribe(), gate, active);
        #[cfg(test)]
        let engine_control = trainer.control.clone();
        let (engine, handle) = Engine::new(
            self.0.clock.clone(),
            Some(trainer),
            hr,
            Arc::new(move |on: bool| {
                #[cfg(test)]
                if !on && pin.load(Ordering::SeqCst) {
                    return;
                }
                activity(on);
            }),
        );
        #[cfg(test)]
        let task = EngineTask::spawn(
            self.0.engine_host,
            Stalled {
                future: Box::pin(engine.run()),
                stall: self.0.stall.clone(),
            },
        )?;
        #[cfg(not(test))]
        let task = EngineTask::spawn(self.0.engine_host, engine.run())?;
        let recorder = StoreRecorder::new(
            self.0.store.clone(),
            RideMeta {
                id: ride_id.clone(),
                workout,
                compiled: compiled.clone(),
                trainer_device_id,
                hr_device_id,
                telemetry_source: self.0.backend.telemetry_source(),
            },
        );
        #[cfg(test)]
        let recorder = BlockingRecorder {
            inner: recorder,
            block: self.0.block_write.clone(),
        };
        // From here the signal is this ride's: a late forced end of an older ride leaves it.
        *self
            .0
            .signal_of
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ride_id.clone());
        let loaded = bounded(handle.send(EngineCmd::Load(RideSetup {
            blocks: compiled.blocks,
            recorder: Box::new(recorder),
            // The health cap is derived from the athlete profile (#244).
            cap_w: None,
        })))
        .await;
        if let Err(e) = loaded {
            if e.code == ErrorCode::EngineUnresponsive {
                // A wedged engine holds the links it was built with: the core cuts it off
                // and releases them itself, as a forced end does, so the next load finds the
                // devices free.
                release_links(&ride_id, &hold, &takeover);
                task.abort();
                release.wait().await;
                return Err(ApiError::new(
                    ErrorCode::EngineUnresponsive,
                    "the ride engine did not take the ride in time; its devices were \
                     released, try again",
                ));
            }
            return Err(e);
        }
        Ok((
            Session {
                handle,
                streams: BTreeMap::new(),
                task,
                ride_id,
                release,
                trainer: hold,
                takeover,
                #[cfg(test)]
                engine_control,
            },
            hr_linked,
        ))
    }

    async fn engine(&self) -> Result<EngineHandle, ApiError> {
        self.0
            .state
            .lock()
            .await
            .session
            .as_ref()
            .map(|s| s.handle.clone())
            .ok_or_else(|| ApiError::new(ErrorCode::Conflict, "no ride loaded"))
    }
    async fn ride_cmd(&self, cmd: EngineCmd) -> Result<(), ApiError> {
        let engine = self.engine().await?;
        bounded(engine.send(cmd)).await
    }
    /// Start the loaded ride.
    ///
    /// # Errors
    /// `Conflict` without a loaded ride or in the wrong state; `DeviceUnavailable` while
    /// the trainer link is not ready; `Storage` if the record cannot be opened;
    /// `EngineUnresponsive` if the engine does not answer within 1 s (`REQUEST_TIMEOUT`).
    pub async fn ride_start(&self) -> Result<(), ApiError> {
        self.ensure_start_allowed()?;
        self.ride_cmd(EngineCmd::Start).await
    }
    /// Pause the ride (the rider's pause).
    ///
    /// # Errors
    /// `Conflict` without a ride in progress; `EngineUnresponsive` if the engine does not answer
    /// within 1 s (`REQUEST_TIMEOUT`).
    pub async fn ride_pause(&self) -> Result<(), ApiError> {
        self.ride_cmd(EngineCmd::Pause).await
    }
    /// End the rider's pause.
    ///
    /// # Errors
    /// `Conflict` unless paused by the rider; `EngineUnresponsive` if the engine does not answer
    /// within 1 s (`REQUEST_TIMEOUT`).
    pub async fn ride_resume(&self) -> Result<(), ApiError> {
        self.ride_cmd(EngineCmd::Resume).await
    }
    /// Jump to the next block now.
    ///
    /// # Errors
    /// `Conflict` without a ride in progress; `EngineUnresponsive` if the engine does not answer
    /// within 1 s (`REQUEST_TIMEOUT`).
    pub async fn ride_skip(&self) -> Result<(), ApiError> {
        self.ride_cmd(EngineCmd::SkipBlock).await
    }
    /// Finish the ride (or unload one that never started) and wait until its devices are
    /// released: the session is reaped and the trainer's and the strap's drivers have closed
    /// their streams (bounded by `RELEASE_TIMEOUT`, 3 s), so a pairing right after finds
    /// them free. Returns the id of the finished ride, `None` for an unloaded one; ending a
    /// ride that already finished returns its id again, after the same release.
    ///
    /// # Errors
    /// `Conflict` without a loaded or just-finished ride.
    pub async fn ride_end(&self) -> Result<Option<String>, ApiError> {
        Ok(self.ride_end_outcome().await?.ride_id().map(str::to_owned))
    }
    /// [`Core::ride_end`] with the typed outcome: whether the engine ended the ride
    /// ([`RideEnd::Ended`]) or the core had to end it by force ([`RideEnd::ForcedEnd`]).
    ///
    /// Never hangs on the engine: the `Stop` is bounded by `STOP_TIMEOUT` (1 s) and the rest
    /// by `REAP_TIMEOUT` and `RELEASE_TIMEOUT`. A stuck or dead engine is aborted, the ride
    /// recorded as aborted and the devices released, and a new `ride_load` works. Ending an
    /// already forced ride again returns the same outcome, after the same release.
    ///
    /// # Errors
    /// `Conflict` without a loaded or just-ended ride.
    pub async fn ride_end_outcome(&self) -> Result<RideEnd, ApiError> {
        self.end_ride(true).await
    }
    /// `ride_end`; without `release` it returns once the engine has ended (the end mode
    /// delivered, the link lost or its deadline passed) and does not wait for the devices'
    /// drivers to hand their peripherals back. That is for a process that is exiting, which
    /// releases the radio anyway and must not spend its quit cap on `RELEASE_TIMEOUT`.
    async fn end_ride(&self, release: bool) -> Result<RideEnd, ApiError> {
        let (id, finishes, link_release, forced) = loop {
            let mut st = self.0.state.lock().await;
            if st.forcing.is_some() {
                // Another end is forcing the ride: wait for it (bounded) instead of spending
                // the `Stop` bound on the same stuck engine again, then report its outcome.
                drop(st);
                self.wait_forcing_done().await;
                continue;
            }
            let Some(s) = &st.session else {
                // Already reaped, possibly by an end that is still waiting for the devices:
                // this call waits for the same release.
                let ended = match (&st.last, &st.last_forced) {
                    (Some((id, _)), _) => Ok(RideEnd::Ended(Some(id.clone()))),
                    (None, Some(id)) => Ok(RideEnd::ForcedEnd(id.clone())),
                    (None, None) => Err(ApiError::new(ErrorCode::Conflict, "no ride loaded")),
                };
                let pending = st.last_release.clone().filter(|_| release);
                drop(st);
                if let Some(pending) = pending {
                    pending.wait().await;
                }
                return ended;
            };
            let state = s.handle.status().state;
            let mut forced = false;
            if state != RideState::Finished {
                // Bounded, so that a stuck engine cannot hold the state lock (and with it
                // every other command, a second End included) for ever. Sent under the lock
                // so that no `ride_start` can reach the engine first.
                match tokio::time::timeout(STOP_TIMEOUT, s.handle.send(EngineCmd::Stop)).await {
                    // `WrongState`: the ride is already ending (it finished on its own, or
                    // an overlapping end got in first, also on an unstarted ride). That is
                    // not an error, and the waits below must still run.
                    Ok(Ok(()) | Err(CommandError::WrongState)) => {}
                    // The engine task is gone, or it did not answer: end the ride by force.
                    Ok(Err(CommandError::EngineStopped)) | Err(_) => forced = true,
                    Ok(Err(e)) => return Err(e.into()),
                }
            }
            let found = (
                s.ride_id.clone(),
                state != RideState::Idle,
                s.release.clone(),
                forced,
            );
            if forced {
                st.forcing = Some(found.0.clone());
            }
            break found;
        };
        let mut outcome = None;
        #[cfg(test)]
        if forced && self.0.force_hook.enabled.swap(false, Ordering::SeqCst) {
            self.0.force_hook.reached.notify_one();
            self.0.force_hook.resume.notified().await;
        }
        if forced {
            // Whichever record wins decides the outcome: a `Stop` that the engine took just
            // after the timeout may have finished the ride, which is then not a forced end.
            outcome = Some(match self.force_end(&id, finishes).await {
                Closed::Finished => RideEnd::Ended(Some(id.clone())),
                Closed::Aborted | Closed::Unknown => RideEnd::ForcedEnd(id.clone()),
            });
        } else {
            self.settle(&id).await;
        }
        if release {
            link_release.wait().await;
        }
        Ok(outcome.unwrap_or_else(|| RideEnd::Ended(finishes.then_some(id))))
    }
    /// End a ride whose engine cannot, in this order:
    /// 1. the engine is **cut off** ([`Takeover::CutOff`]): the relays stop feeding it and its
    ///    ride-active signal is ignored from now on;
    /// 2. **the end mode**, if the ride had started: the core sends `Resistance{0}` to the
    ///    trainer itself, through its own hold on the control channel, and waits for the
    ///    driver to deliver it (`END_MODE_TIMEOUT`), so that the last ERG target is not left
    ///    on a healthy trainer (ADR-0002 §7.9);
    /// 3. **the devices are released** by the core ([`Takeover::Release`], `release_links`),
    ///    through what it owns, not by dropping the engine's links, which an engine wedged on
    ///    its thread still holds; the caller awaits the drivers' confirmation;
    /// 4. the engine is aborted (and detached when it ignores that, `reap`);
    /// 5. the ride's record is closed as aborted, off the async executor (`close_record`);
    /// 6. the ride-active signal the engine can no longer drop is cleared, unless a newer
    ///    ride's engine has taken it over meanwhile ([`Core::clear_signal`], #370).
    ///
    /// Idempotent: an overlapping forced end finds the session gone and the record closed.
    async fn force_end(&self, ride_id: &str, started: bool) -> Closed {
        tracing::warn!(
            ride_id,
            "the engine did not stop the ride: ending it by force"
        );
        let taken = {
            let st = self.0.state.lock().await;
            st.session
                .as_ref()
                .filter(|s| s.ride_id == ride_id)
                .map(|s| (s.trainer.clone(), s.takeover.clone()))
        };
        if let Some((hold, takeover)) = taken {
            takeover.send_replace(Takeover::CutOff);
            if started {
                force_end_mode(ride_id, &hold).await;
            }
            release_links(ride_id, &hold, &takeover);
        }
        self.reap(ride_id, true).await;
        let closed = self.close_record(ride_id).await;
        {
            let mut st = self.0.state.lock().await;
            // A late end of a replaced ride (#370) clears nothing of the newer ride's: neither
            // its signal nor the last-end record that its End reports.
            if self.clear_signal(ride_id) {
                if closed == Closed::Finished {
                    if st.last.is_none() {
                        st.last = Some((ride_id.to_owned(), self.0.idle.clone()));
                    }
                } else if st.last.is_none() {
                    st.last_forced = Some(ride_id.to_owned());
                }
            }
            st.forcing = None;
        }
        self.0.released.notify_waiters();
        closed
    }
    /// Wait until no forced end is in progress, for at most `REAP_TIMEOUT` (a forced end that
    /// was cancelled halfway must not leave the flag up for ever).
    async fn wait_forcing_done(&self) {
        let wait = async {
            loop {
                let woken = self.0.released.notified();
                tokio::pin!(woken);
                woken.as_mut().enable();
                if self.0.state.lock().await.forcing.is_none() {
                    return;
                }
                woken.await;
            }
        };
        if tokio::time::timeout(REAP_TIMEOUT, wait).await.is_err() {
            self.0.state.lock().await.forcing = None;
        }
    }

    /// Close the record of a ride the engine cannot finish: aborted, keeping what was
    /// recorded. The store call blocks (its worker is reached over a channel), so it runs on
    /// the blocking pool and is waited for at most `STORE_TIMEOUT`.
    async fn close_record(&self, ride_id: &str) -> Closed {
        let (store, id) = (self.0.store.clone(), ride_id.to_owned());
        let job = tokio::task::spawn_blocking(move || match store.abort_ride(&id, unix_now()) {
            Ok(()) => Closed::Aborted,
            // Not live: finished by the engine after all, never started, or closed by the
            // other of two overlapping ends.
            Err(crank_store::StoreError::InvalidInput(_)) => match store.ride(&id) {
                Ok(Some(ride)) => match ride.state {
                    crank_store::RideState::Finished => Closed::Finished,
                    _ => Closed::Aborted,
                },
                _ => Closed::Unknown,
            },
            Err(error) => {
                tracing::warn!(ride_id = id, %error, "closing the ride's record failed");
                Closed::Unknown
            }
        });
        if let Ok(Ok(closed)) = tokio::time::timeout(STORE_TIMEOUT, job).await {
            closed
        } else {
            tracing::warn!(ride_id, "the store did not close the ride's record in time");
            Closed::Unknown
        }
    }
    /// The application is quitting: end the loaded ride like `ride_end` and wait, for at
    /// most `cap`, until the engine has ended it (not until the devices are released).
    ///
    /// Quitting is a ride end (ADR-0002 §7.9): the engine puts the trainer into its end
    /// mode (`Resistance{0}`) and holds the ride-active gate until that was delivered, the
    /// link was lost or its own 2 s deadline passed. A process that exits without waiting
    /// can leave ERG torque on the trainer (S1), and on macOS exiting does not drop `Core`.
    /// It does not wait for the drivers to hand the peripherals back (`end_ride(false)`):
    /// process exit releases the radio, and a silent strap must not eat the cap with
    /// `RELEASE_TIMEOUT`.
    /// The cap bounds a trainer that never answers; it should exceed the engine's 2 s.
    pub async fn shutdown(&self, cap: Duration) -> Shutdown {
        match tokio::time::timeout(cap, self.end_ride(false)).await {
            Err(_) => Shutdown::TimedOut,
            Ok(Ok(_)) => Shutdown::Ended,
            Ok(Err(e)) if e.code == ErrorCode::Conflict => Shutdown::Idle,
            Ok(Err(e)) => Shutdown::Failed(e),
        }
    }
    /// The ride's state; durable truth behind `resync` (ADR-0001 §4.3).
    ///
    /// # Errors
    /// Never fails today.
    pub async fn ride_status(&self) -> Result<RideStatusView, ApiError> {
        let st = self.0.state.lock().await;
        let status = match (&st.session, &st.last) {
            (Some(s), _) => StreamStatus::from(&s.handle.status()),
            (None, Some((_, last))) => last.clone(),
            (None, None) => self.0.idle.clone(),
        };
        Ok(RideStatusView {
            loading: self.0.loading.load(Ordering::SeqCst),
            status,
        })
    }

    // ---- streams ----------------------------------------------------------------

    /// Open the window's multiplexed stream on the loaded ride (dropping that window's
    /// previous streams). Frames go to `sink` until the ride ends or `sink` fails. The
    /// returned id is unique for the process lifetime and belongs to `window`: every
    /// other stream command names the same window and is refused for any other.
    ///
    /// # Errors
    /// `Conflict` without a loaded ride, with 16 streams open, or when the ride was replaced
    /// while the stream opened; `EngineUnresponsive` if the engine does not answer.
    pub async fn open_stream(
        &self,
        window: Option<String>,
        sink: impl FrameSink,
    ) -> Result<StreamId, ApiError> {
        let (engine, ride_id) = {
            let st = self.0.state.lock().await;
            let session = st
                .session
                .as_ref()
                .ok_or_else(|| ApiError::new(ErrorCode::Conflict, "no ride loaded"))?;
            (session.handle.clone(), session.ride_id.clone())
        };
        // Not under the state lock (#351): End must never wait behind the engine's answer.
        let (engine_id, mut frames) = bounded(engine.open_stream(window.clone())).await?;
        let id = loop {
            let n = self.0.next_stream.fetch_add(1, Ordering::Relaxed);
            if n != 0 {
                break StreamId(n);
            }
        };
        {
            let mut st = self.0.state.lock().await;
            // The ride may have ended or been replaced meanwhile; dropping `frames` then makes
            // its engine reap the stream.
            let session = st
                .session
                .as_mut()
                .filter(|s| s.ride_id == ride_id)
                .ok_or_else(|| ApiError::new(ErrorCode::Conflict, "no ride loaded"))?;
            session.streams.insert(id, (engine_id, window));
        }
        tokio::spawn(async move {
            while let Some(frame) = frames.recv().await {
                if sink.send(frame).is_err() {
                    // Dropping the receiver makes the engine reap the stream.
                    break;
                }
            }
        });
        Ok(id)
    }
    /// The engine and engine-side id of a stream, if `window` owns it.
    async fn owned(
        &self,
        window: Option<&str>,
        stream: StreamId,
    ) -> Result<(EngineHandle, StreamId), ApiError> {
        let st = self.0.state.lock().await;
        let session = st
            .session
            .as_ref()
            .ok_or_else(|| ApiError::new(ErrorCode::Conflict, "no ride loaded"))?;
        match session.streams.get(&stream) {
            Some((engine_id, owner)) if owner.as_deref() == window => {
                Ok((session.handle.clone(), *engine_id))
            }
            // A stream of another window or ride is indistinguishable from none.
            _ => Err(ApiError::new(ErrorCode::NotFound, "unknown stream")),
        }
    }
    /// Subscribe to a metric on a stream of `window`.
    ///
    /// # Errors
    /// `NotFound` for a stream `window` does not own, `Conflict` beyond 64
    /// subscriptions, `InvalidInput` for a rate or filter out of range;
    /// `EngineUnresponsive` if the engine does not answer within 1 s (`REQUEST_TIMEOUT`).
    pub async fn subscribe(
        &self,
        window: Option<&str>,
        stream: StreamId,
        metric: MetricId,
        opts: SubscribeOpts,
    ) -> Result<SubId, ApiError> {
        let (engine, id) = self.owned(window, stream).await?;
        bounded(engine.stream_subscribe(id, metric, opts)).await
    }
    /// End a subscription of a stream of `window`.
    ///
    /// # Errors
    /// `NotFound`; `EngineUnresponsive` if the engine does not answer within 1 s
    /// (`REQUEST_TIMEOUT`).
    pub async fn unsubscribe(
        &self,
        window: Option<&str>,
        stream: StreamId,
        sub: SubId,
    ) -> Result<(), ApiError> {
        let (engine, id) = self.owned(window, stream).await?;
        bounded(engine.stream_unsubscribe(id, sub)).await
    }
    /// The window has applied every frame up to `seq`.
    ///
    /// # Errors
    /// `NotFound` for a stream `window` does not own; `EngineUnresponsive` if the engine does not
    /// answer within 1 s (`REQUEST_TIMEOUT`).
    pub async fn stream_ack(
        &self,
        window: Option<&str>,
        stream: StreamId,
        seq: u32,
    ) -> Result<(), ApiError> {
        let (engine, id) = self.owned(window, stream).await?;
        bounded(engine.stream_ack(id, seq)).await
    }
    /// Latest sample of every subscription, the status and the links.
    ///
    /// # Errors
    /// `NotFound` for a stream `window` does not own; `EngineUnresponsive` if the engine does not
    /// answer within 1 s (`REQUEST_TIMEOUT`).
    pub async fn stream_snapshot(
        &self,
        window: Option<&str>,
        stream: StreamId,
    ) -> Result<StreamSnapshot, ApiError> {
        let (engine, id) = self.owned(window, stream).await?;
        bounded(engine.stream_snapshot(id)).await
    }
    /// A window was destroyed: drop its streams. The engine is told without the state lock
    /// and for at most 1 s (`REQUEST_TIMEOUT`; its request channel may be full, #351); if it does
    /// not take the request, the window's streams still end, as their sink fails.
    pub async fn close_window(&self, window: String) {
        let engine = {
            let mut st = self.0.state.lock().await;
            let Some(session) = st.session.as_mut() else {
                return;
            };
            session
                .streams
                .retain(|_, (_, owner)| owner.as_deref() != Some(window.as_str()));
            session.handle.clone()
        };
        if let Err(e) = bounded(engine.close_window(window)).await {
            tracing::warn!(%e, "the engine did not take a window's close");
        }
    }

    // ---- history ----------------------------------------------------------------

    fn record(ride: RideListing) -> RideRecord {
        let summary = ride.summary.and_then(|s| {
            serde_json::from_str::<RideSummary>(crank_store::JsonColumn::as_str(&s)).ok()
        });
        RideRecord {
            id: ride.id,
            started_at: secs(ride.started_at),
            ended_at: ride.ended_at.map(secs),
            outcome: match ride.state {
                crank_store::RideState::Open => RideOutcome::Open,
                crank_store::RideState::Finished => RideOutcome::Finished,
                crank_store::RideState::Aborted => RideOutcome::Aborted,
            },
            workout_id: (ride.workout_id != "free-ride").then_some(ride.workout_id),
            workout_name: ride.workout_name,
            summary,
        }
    }
    /// Stored rides, newest first.
    ///
    /// # Errors
    /// `Storage`.
    #[expect(
        clippy::unused_async,
        reason = "every facade method is async (ADR-0001 §4)"
    )]
    pub async fn rides_list(&self) -> Result<Vec<RideRecord>, ApiError> {
        Ok(self
            .0
            .store
            .rides()?
            .into_iter()
            .map(Self::record)
            .collect())
    }
    /// One stored ride with its summary.
    ///
    /// # Errors
    /// `NotFound`, `Storage`.
    pub async fn ride_summary(&self, id: String) -> Result<RideRecord, ApiError> {
        self.rides_list()
            .await?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, format!("no ride {id:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Radio;
    use crank_devices::{
        DriverKind, HeartRateDriver, TrainerDriver, TrainerLink,
        sim::{SimConfig, SimRig},
    };
    use crank_engine::VirtualClock;
    use futures_util::{FutureExt, future::BoxFuture, stream, stream::BoxStream};
    use std::sync::Mutex;

    /// A radio over the simulator that records what the core tells it.
    #[derive(Clone)]
    struct FakeRadio {
        rig: SimRig,
        script: Vec<DiscoveredDevice>,
        never_ends: bool,
        remembered: Arc<Mutex<Vec<String>>>,
        forgotten: Arc<Mutex<Vec<String>>>,
        commands: Arc<Mutex<Vec<TrainerCommand>>>,
        /// The ride-active gate the radio was given (the engine's signal, shared).
        gate: Arc<Mutex<Option<RideGate>>>,
        /// For each `Disconnect` sent: was the gate still active?
        disconnects: Arc<Mutex<Vec<bool>>>,
        /// For each `SetMode` sent: was the gate active?
        mode_gate: Arc<Mutex<Vec<bool>>>,
        /// How long a trainer connect takes.
        connect_delay: Duration,
        /// How long releasing the trainer (its `Disconnect`, or its control channel
        /// closing) takes.
        disconnect_delay: Duration,
        /// How long the trainer takes to take each `SetMode` (it delivers after that).
        mode_delay: Duration,
        /// The trainer never takes a `SetMode`: no `Delivered` ever reaches the engine.
        swallow_modes: Arc<std::sync::atomic::AtomicBool>,
        /// How long releasing the strap takes once the engine's end of its stream is gone.
        hr_delay: Duration,
        /// How many straps have been released.
        hr_released: Arc<std::sync::atomic::AtomicUsize>,
        /// `set_mode` and `disconnect`, in the order and at the times the trainer saw them.
        order: Arc<Mutex<Vec<(&'static str, tokio::time::Instant)>>>,
        /// Trainer connects fail while set.
        fail_connect: Arc<std::sync::atomic::AtomicBool>,
        /// Trainer connects panic while set.
        panic_connect: Arc<std::sync::atomic::AtomicBool>,
    }
    use crank_devices::DiscoveredDevice;
    struct Recording(FakeRadio);
    impl TrainerDriver for Recording {
        fn kind(&self) -> DriverKind {
            DriverKind::Sim
        }
        fn connect(
            &self,
            dev: &DeviceRef,
            rider: RiderConfig,
        ) -> BoxFuture<'static, Result<TrainerLink, crank_devices::DeviceError>> {
            let (inner, log) = (self.0.rig.trainer(), self.0.commands.clone());
            let radio = self.0.clone();
            let dev = dev.clone();
            let delay = self.0.connect_delay;
            let (mode_delay, swallow) = (self.0.mode_delay, self.0.swallow_modes.clone());
            let (fail, boom) = (self.0.fail_connect.clone(), self.0.panic_connect.clone());
            async move {
                tokio::time::sleep(delay).await;
                assert!(
                    !boom.load(std::sync::atomic::Ordering::SeqCst),
                    "the radio panicked while connecting"
                );
                if fail.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(crank_devices::DeviceError::NotFound);
                }
                let mut link = inner.connect(&dev, rider).await?;
                let (tx, mut rx) = tokio::sync::mpsc::channel(64);
                let real = std::mem::replace(&mut link.control, tx);
                tokio::spawn(async move {
                    // The peripheral is released by an explicit `Disconnect` or by the
                    // control channel closing; note the gate at the first of them.
                    let mut released = false;
                    let slow = radio.disconnect_delay;
                    let mut release = |radio: &FakeRadio| {
                        if !std::mem::replace(&mut released, true) {
                            let active = radio
                                .gate
                                .lock()
                                .unwrap()
                                .as_ref()
                                .is_some_and(RideGate::is_active);
                            radio.disconnects.lock().unwrap().push(active);
                            radio
                                .order
                                .lock()
                                .unwrap()
                                .push(("disconnect", tokio::time::Instant::now()));
                        }
                    };
                    while let Some(cmd) = rx.recv().await {
                        if cmd == TrainerCommand::Disconnect {
                            tokio::time::sleep(slow).await;
                            release(&radio);
                        }
                        if matches!(cmd, TrainerCommand::SetMode { .. }) {
                            let active = radio
                                .gate
                                .lock()
                                .unwrap()
                                .as_ref()
                                .is_some_and(RideGate::is_active);
                            radio.mode_gate.lock().unwrap().push(active);
                        }
                        log.lock().unwrap().push(cmd);
                        if matches!(cmd, TrainerCommand::SetMode { .. }) {
                            radio
                                .order
                                .lock()
                                .unwrap()
                                .push(("set_mode", tokio::time::Instant::now()));
                            if swallow.load(std::sync::atomic::Ordering::SeqCst) {
                                continue;
                            }
                            tokio::time::sleep(mode_delay).await;
                        }
                        if real.send(cmd).await.is_err() {
                            break;
                        }
                    }
                    tokio::time::sleep(slow).await;
                    release(&radio);
                });
                Ok(link)
            }
            .boxed()
        }
    }
    /// A strap driver as the real one behaves: when its consumer is gone it
    /// releases the peripheral, which takes `hr_delay`, and only then closes its stream.
    struct SlowHeart(FakeRadio);
    impl HeartRateDriver for SlowHeart {
        fn connect(
            &self,
            dev: &DeviceRef,
        ) -> BoxFuture<'static, Result<crank_devices::HeartRateLink, crank_devices::DeviceError>>
        {
            let (inner, radio, dev) = (self.0.rig.heart(), self.0.clone(), dev.clone());
            async move {
                let mut link = inner.connect(&dev).await?;
                let (tx, rx) = tokio::sync::mpsc::channel(64);
                let mut real = std::mem::replace(&mut link.events, rx);
                tokio::spawn(async move {
                    // (the virtual clock sends no samples, so the loss of the consumer is
                    // noticed through `closed`, where the real driver notices it on a send)
                    loop {
                        tokio::select! {
                            event = real.recv() => {
                                let Some(event) = event else { break };
                                if tx.send(event).await.is_err() {
                                    break;
                                }
                            }
                            () = tx.closed() => break,
                        }
                    }
                    drop(real);
                    tokio::time::sleep(radio.hr_delay).await;
                    radio
                        .hr_released
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    drop(tx);
                });
                Ok(link)
            }
            .boxed()
        }
    }
    impl Radio for FakeRadio {
        fn scan(&self) -> BoxStream<'static, DiscoveredDevice> {
            let s = stream::iter(self.script.clone());
            if self.never_ends {
                s.chain(stream::pending()).boxed()
            } else {
                s.boxed()
            }
        }
        fn trainer(&self, _: Arc<dyn Clock>, gate: RideGate) -> Arc<dyn TrainerDriver> {
            *self.gate.lock().unwrap() = Some(gate);
            Arc::new(Recording(self.clone()))
        }
        fn heart(&self, _: Arc<dyn Clock>, _: RideGate) -> Arc<dyn HeartRateDriver> {
            Arc::new(SlowHeart(self.clone()))
        }
        fn remember(&self, pairing: Pairing) {
            self.remembered.lock().unwrap().push(pairing.platform_id);
        }
        fn forget(&self, platform_id: &str) {
            self.forgotten.lock().unwrap().push(platform_id.into());
        }
    }
    fn found(id: &str, name: Option<&str>, trainer: bool, heart_rate: bool) -> DiscoveredDevice {
        DiscoveredDevice {
            device: DeviceRef(id.into()),
            info: DeviceInfo {
                name: name.map(Into::into),
                ..DeviceInfo::default()
            },
            trainer: trainer.then_some(DriverKind::FeC),
            heart_rate,
        }
    }
    fn fake(script: Vec<DiscoveredDevice>, never_ends: bool) -> FakeRadio {
        FakeRadio {
            rig: SimRig::new(SimConfig::default(), Arc::new(VirtualClock::new(1))),
            script,
            never_ends,
            remembered: Arc::default(),
            forgotten: Arc::default(),
            commands: Arc::default(),
            gate: Arc::default(),
            disconnects: Arc::default(),
            mode_gate: Arc::default(),
            connect_delay: Duration::ZERO,
            disconnect_delay: Duration::ZERO,
            mode_delay: Duration::ZERO,
            swallow_modes: Arc::default(),
            hr_delay: Duration::ZERO,
            hr_released: Arc::default(),
            order: Arc::default(),
            fail_connect: Arc::default(),
            panic_connect: Arc::default(),
        }
    }
    /// Re-opening the data dir with a radio the test controls.
    fn open(dir: &std::path::Path, radio: &FakeRadio) -> Core {
        let radio = radio.clone();
        let factory: crate::backend::RadioFactory = Arc::new(move || {
            let radio: Arc<dyn Radio> = Arc::new(radio.clone());
            async move { Ok(radio) }.boxed()
        });
        Core::open_with(
            dir,
            Some(Arc::new(VirtualClock::new(1))),
            EngineHost::Task,
            move |p, _| Backend::with_radio(factory, p),
        )
        .unwrap()
    }

    thread_local! {
        /// Called by `ride_activity` between its check and its set, on the calling thread.
        pub(super) static BETWEEN_CHECK_AND_SET: std::cell::RefCell<Option<Box<dyn Fn()>>> =
            const { std::cell::RefCell::new(None) };
    }
    /// #344 r2: an engine on its own thread that has passed the cut-off check when the core
    /// cuts it off cannot set the ride-active signal afterwards: the core's cut-off waits for
    /// it, so the next ride's raised signal stays raised.
    #[test]
    fn a_cut_off_engine_cannot_set_the_ride_active_signal_after_the_cut_off() {
        let takeover = Arc::new(watch::channel(Takeover::Engine).0);
        let (gate, active) = (RideGate::new(true), watch::channel(true).0);
        let activity = ride_activity(takeover.subscribe(), gate.clone(), active.clone());
        let (reached_tx, reached) = std::sync::mpsc::channel();
        let (go, go_rx) = std::sync::mpsc::channel::<()>();
        let go_rx = std::sync::Mutex::new(go_rx);
        // the engine thread: its ride ends (`false`), and it stops between check and set
        let engine = std::thread::spawn(move || {
            BETWEEN_CHECK_AND_SET.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    reached_tx.send(()).unwrap();
                    go_rx.lock().unwrap().recv().unwrap();
                }));
            });
            activity(false);
        });
        reached.recv().unwrap();
        // the core: cut off, clear, and the next ride raises the signal
        let core = {
            let (takeover, gate, active) = (takeover.clone(), gate.clone(), active.clone());
            std::thread::spawn(move || {
                takeover.send_replace(Takeover::CutOff);
                gate.set_active(true);
                active.send_replace(true);
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        go.send(()).unwrap();
        engine.join().unwrap();
        core.join().unwrap();
        assert!(
            gate.is_active(),
            "the cut-off engine dropped the next ride's gate"
        );
        assert!(*active.borrow());
    }
    /// As [`open`], on `host`, real time and the production clock.
    fn open_on(dir: &std::path::Path, radio: &FakeRadio, host: EngineHost) -> Core {
        let radio = radio.clone();
        let factory: crate::backend::RadioFactory = Arc::new(move || {
            let radio: Arc<dyn Radio> = Arc::new(radio.clone());
            async move { Ok(radio) }.boxed()
        });
        Core::open_with(dir, None, host, move |p, _| Backend::with_radio(factory, p)).unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn stored_pairings_reach_the_radio_when_it_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let first = fake(vec![], false);
        {
            let core = open(dir.path(), &first);
            core.device_pair("neo".into(), DeviceRole::Trainer)
                .await
                .unwrap();
            core.device_pair("hrm".into(), DeviceRole::HeartRate)
                .await
                .unwrap();
        }
        // A new process: its radio starts empty and is told before it is used.
        let second = fake(vec![], false);
        let core = open(dir.path(), &second);
        assert!(
            second.remembered.lock().unwrap().is_empty(),
            "lazy: not created yet"
        );
        core.devices_scan(Some(200)).await.unwrap();
        let mut told = second.remembered.lock().unwrap().clone();
        told.sort();
        assert_eq!(told, ["hrm", "neo"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_pairing_made_after_the_radio_exists_reaches_it() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.devices_scan(Some(200)).await.unwrap(); // the radio is created, empty
        assert!(radio.remembered.lock().unwrap().is_empty());
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        assert_eq!(*radio.remembered.lock().unwrap(), ["neo"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_pairing_forgotten_before_the_radio_exists_is_not_handed_over() {
        let dir = tempfile::tempdir().unwrap();
        {
            let core = open(dir.path(), &fake(vec![], false));
            core.device_pair("neo".into(), DeviceRole::Trainer)
                .await
                .unwrap();
            core.device_pair("hrm".into(), DeviceRole::HeartRate)
                .await
                .unwrap();
        }
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_forget(DeviceRole::Trainer).await.unwrap(); // radio not created yet
        core.devices_scan(Some(200)).await.unwrap();
        assert_eq!(*radio.remembered.lock().unwrap(), ["hrm"]);
    }

    #[tokio::test(start_paused = true)]
    async fn pairing_releases_the_trainer_and_a_new_pairing_replaces_the_old() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo-a".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.device_pair("neo-b".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        let disconnects = radio
            .commands
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == TrainerCommand::Disconnect)
            .count();
        assert_eq!(disconnects, 2, "every pairing hands the peripheral back");
        assert_eq!(*radio.forgotten.lock().unwrap(), ["neo-a"]);
        let links = core.device_links().await.unwrap();
        assert_eq!(links[0].device.as_ref().unwrap().id, "neo-b");
        core.device_forget(DeviceRole::Trainer).await.unwrap();
        assert_eq!(*radio.forgotten.lock().unwrap(), ["neo-a", "neo-b"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_peripheral_seen_three_times_is_listed_once() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(
            vec![
                found("neo", None, true, false),
                found("neo", Some("Tacx NEO"), false, false),
                found("neo", None, false, true),
                found("other", Some("Strap"), false, true),
            ],
            false,
        );
        let list = open(dir.path(), &radio).devices_scan(None).await.unwrap();
        assert_eq!(list.len(), 2);
        let neo = list.iter().find(|d| d.id == "neo").unwrap();
        assert_eq!(neo.name.as_deref(), Some("Tacx NEO"));
        assert!(neo.trainer && neo.heart_rate);
    }

    #[tokio::test(start_paused = true)]
    async fn the_scan_length_is_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], true);
        let core = open(dir.path(), &radio);
        let took = |ms| {
            let core = core.clone();
            async move {
                let t = tokio::time::Instant::now();
                core.devices_scan(ms).await.unwrap();
                t.elapsed()
            }
        };
        assert_eq!(took(Some(1)).await, Duration::from_millis(200));
        assert_eq!(took(Some(1_000_000)).await, Duration::from_secs(15));
        assert_eq!(took(None).await, SCAN_FOR);
    }

    #[tokio::test(start_paused = true)]
    async fn the_simulator_pairs_each_id_only_in_its_own_role() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open(CoreConfig {
            data_dir: dir.path().into(),
            devices: DeviceMode::Sim(SimConfig::default()),
            clock: Some(Arc::new(VirtualClock::new(1))),
            engine_host: EngineHost::Task,
        })
        .unwrap();
        for (id, role) in [
            ("strap", DeviceRole::Trainer),
            ("trainer", DeviceRole::HeartRate),
            ("nope", DeviceRole::Trainer),
        ] {
            let e = core.device_pair(id.into(), role).await.unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidInput, "{id} as {role:?}");
        }
        assert!(
            core.device_pair("trainer".into(), DeviceRole::Trainer)
                .await
                .is_ok()
        );
        assert!(
            core.device_pair("strap".into(), DeviceRole::HeartRate)
                .await
                .is_ok()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn devices_are_released_only_after_the_ride_active_gate_drops() {
        let dir = tempfile::tempdir().unwrap();
        let mut radio = fake(vec![], false);
        radio.mode_delay = Duration::from_secs(1);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        radio.disconnects.lock().unwrap().clear();
        radio.commands.lock().unwrap().clear();

        core.ride_load(Some("builtin-sensor-check".into()))
            .await
            .unwrap();
        core.ride_start().await.unwrap();
        // The workout ends itself; the trainer takes the end mode a second later, so the
        // gate stays up for that second after `Finished` is published (#269).
        tokio::time::sleep(Duration::from_mins(3)).await;
        let finished = async {
            while core.ride_status().await.unwrap().status.state
                != crank_engine::StreamRideState::Finished
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), finished)
            .await
            .expect("the workout finishes");
        assert!(
            radio.disconnects.lock().unwrap().is_empty(),
            "the devices are not released on `Finished`, only when the gate drops"
        );
        core.ride_end().await.unwrap();

        // The trainer was driven during the ride, and it was released once, after the
        // gate had dropped.
        assert!(
            radio
                .commands
                .lock()
                .unwrap()
                .iter()
                .any(|c| matches!(c, TrainerCommand::SetMode { .. }))
        );
        // The ride-active gate is up for every mode the engine sends (it is what the
        // reconnect supervisors read) and down when the trainer is released.
        let modes = radio.mode_gate.lock().unwrap().clone();
        assert!(!modes.is_empty());
        assert!(modes.iter().all(|active| *active), "{modes:?}");
        assert_eq!(*radio.disconnects.lock().unwrap(), [false]);
        // #269 / #34: the last mode the trainer is given is the non-torque end mode and
        // nothing follows it but the release (by closing the link, which `disconnects`
        // above saw with the gate down), so the release cannot drop it (S1).
        let commands = radio.commands.lock().unwrap().clone();
        assert_eq!(
            commands.last().and_then(mode_of),
            Some(ControlMode::Resistance { pct: 0.0 }),
            "{commands:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pairing_and_forgetting_are_refused_while_a_ride_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.ride_load(None).await.unwrap();
        // A strap is not connected by the ride, so only the guard can refuse this.
        let pair = core.device_pair("hrm".into(), DeviceRole::HeartRate).await;
        assert_eq!(pair.unwrap_err().code, ErrorCode::Conflict);
        let forget = core.device_forget(DeviceRole::Trainer).await;
        assert_eq!(forget.unwrap_err().code, ErrorCode::Conflict);
        core.ride_end().await.unwrap();
        // Once the ride has released its devices, both work again.
        core.device_pair("other".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.device_forget(DeviceRole::Trainer).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn loading_is_reported_while_connecting_and_never_blocks_status() {
        let dir = tempfile::tempdir().unwrap();
        let mut radio = fake(vec![], false);
        radio.connect_delay = Duration::from_secs(10);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        let loading = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_load(None).await })
        };
        tokio::time::sleep(Duration::from_secs(1)).await;
        // Mid-connect: the status answers at once, says so, and nothing else may start.
        let view = core.ride_status().await.unwrap();
        assert!(view.loading);
        assert_eq!(
            core.ride_load(None).await.unwrap_err().code,
            ErrorCode::Conflict
        );
        // A strap is not connected by the load, so only the `loading` check can refuse it.
        assert_eq!(
            core.device_pair("hrm".into(), DeviceRole::HeartRate)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        loading.await.unwrap().unwrap();
        let view = core.ride_status().await.unwrap();
        assert!(!view.loading && view.status.loaded);
    }

    #[tokio::test(start_paused = true)]
    async fn a_cancelled_load_leaves_the_core_usable() {
        let dir = tempfile::tempdir().unwrap();
        let mut radio = fake(vec![], false);
        radio.connect_delay = Duration::from_secs(10);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        // The caller gives up (a dropped command future) in the middle of the connect.
        let cancelled = tokio::time::timeout(Duration::from_secs(2), core.ride_load(None)).await;
        assert!(cancelled.is_err(), "the load was cut off");
        assert!(!core.ride_status().await.unwrap().loading);
        core.device_pair("hrm".into(), DeviceRole::HeartRate)
            .await
            .unwrap();
        core.devices_scan(Some(200)).await.unwrap();
        core.device_forget(DeviceRole::HeartRate).await.unwrap();
        core.ride_load(None).await.unwrap();
    }

    /// A core whose trainer takes `disconnect_delay` to release, with a load that was cut
    /// off after the engine had been sent `Load`, while it waited for the state lock
    /// (#278). Returns the core and the radio, the lock already given back.
    async fn cut_off_load(dir: &std::path::Path, pause_discard: bool) -> (Core, FakeRadio) {
        let mut radio = fake(vec![], false);
        radio.connect_delay = Duration::from_secs(10);
        radio.disconnect_delay = Duration::from_secs(2);
        let core = open(dir, &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        radio.disconnects.lock().unwrap().clear();

        let load = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_load(None).await })
        };
        // Mid-connect the state lock is taken by someone else, so the load parks on it
        // once it has connected and sent `Load`.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let held = core.0.state.lock().await;
        tokio::time::sleep(Duration::from_secs(15)).await;
        assert!(core.0.loading.load(Ordering::SeqCst));
        assert!(radio.gate.lock().unwrap().as_ref().unwrap().is_active());
        core.0
            .discard_hook
            .enabled
            .store(pause_discard, Ordering::SeqCst);
        load.abort();
        assert!(load.await.unwrap_err().is_cancelled());
        drop(held);
        (core, radio)
    }

    /// #278: a load cut off after `Load` still releases its engine and records nothing,
    /// and until the trainer is actually released nothing else may start over it.
    #[tokio::test(start_paused = true)]
    async fn a_load_cancelled_at_the_state_lock_releases_its_engine_and_records_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (core, radio) = cut_off_load(dir.path(), false).await;
        // The trainer takes 2 s to release: until then a pairing and a load are refused.
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert_eq!(
                core.device_pair("other".into(), DeviceRole::Trainer)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            assert_eq!(
                core.ride_load(None).await.unwrap_err().code,
                ErrorCode::Conflict
            );
            assert!(radio.disconnects.lock().unwrap().is_empty());
            assert!(core.ride_status().await.unwrap().loading);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Released: the trainer was disconnected after the gate fell, and the core is free.
        assert_eq!(*radio.disconnects.lock().unwrap(), [false]);
        assert!(!core.0.loading.load(Ordering::SeqCst));
        assert!(!radio.gate.lock().unwrap().as_ref().unwrap().is_active());
        assert!(core.0.state.lock().await.session.is_none());
        assert!(core.rides_list().await.unwrap().is_empty());
        assert!(!core.ride_status().await.unwrap().status.loaded);
        core.device_pair("other".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.ride_load(None).await.unwrap();
        core.ride_end().await.unwrap();
        assert!(core.rides_list().await.unwrap().is_empty());
    }

    /// #278: a `ride_start` that reaches the discarded session as soon as it is visible
    /// finds it already stopped, so no ride row is ever opened. The hook parks `discard`
    /// right after it released the state lock.
    #[tokio::test(start_paused = true)]
    async fn a_start_racing_a_cancelled_load_opens_no_ride() {
        let dir = tempfile::tempdir().unwrap();
        let (core, _radio) = cut_off_load(dir.path(), true).await;
        core.0.discard_hook.reached.notified().await;
        assert!(core.ride_start().await.is_err());
        core.0.discard_hook.resume.notify_one();
        tokio::time::sleep(Duration::from_secs(10)).await;
        assert!(core.rides_list().await.unwrap().is_empty());
        assert!(!core.ride_status().await.unwrap().status.loaded);
    }

    #[tokio::test(start_paused = true)]
    async fn a_panicking_load_leaves_the_core_usable() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        radio
            .panic_connect
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let load = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_load(None).await })
        };
        assert!(load.await.unwrap_err().is_panic());
        radio
            .panic_connect
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(!core.ride_status().await.unwrap().loading);
        core.ride_load(None).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_load_that_fails_to_connect_has_already_replaced_the_unstarted_ride() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.ride_load(None).await.unwrap();
        assert!(core.ride_status().await.unwrap().status.loaded);
        radio
            .fail_connect
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let e = core.ride_load(None).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::DeviceUnavailable);
        // Nothing is loaded and the devices are free again.
        let view = core.ride_status().await.unwrap();
        assert!(!view.status.loaded && !view.loading);
        radio
            .fail_connect
            .store(false, std::sync::atomic::Ordering::SeqCst);
        core.device_forget(DeviceRole::Trainer).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn an_invalid_load_leaves_the_loaded_ride_alone() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.ride_load(Some("builtin-sensor-check".into()))
            .await
            .unwrap();
        let e = core.ride_load(Some("nope".into())).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::NotFound);
        assert!(core.ride_status().await.unwrap().status.loaded);
    }

    /// A ride is under way when the application quits.
    async fn riding(radio: &FakeRadio, dir: &std::path::Path) -> Core {
        let core = open(dir, radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        radio.disconnects.lock().unwrap().clear();
        radio.commands.lock().unwrap().clear();
        core.ride_load(Some("builtin-sensor-check".into()))
            .await
            .unwrap();
        core.ride_start().await.unwrap();
        tokio::time::sleep(Duration::from_secs(20)).await;
        core
    }

    #[tokio::test(start_paused = true)]
    async fn quitting_mid_ride_ends_it_with_the_end_mode_after_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        let mut radio = fake(vec![], false);
        radio.mode_delay = Duration::from_millis(500);
        let core = riding(&radio, dir.path()).await;
        assert_eq!(core.shutdown(Duration::from_secs(3)).await, Shutdown::Ended);
        // The last thing the trainer was told is the non-torque end mode. (Whether the
        // driver has disconnected by now is not part of the contract: quit does not wait.)
        let commands = radio.commands.lock().unwrap().clone();
        assert_eq!(
            commands.last().and_then(mode_of),
            Some(ControlMode::Resistance { pct: 0.0 }),
            "{commands:?}"
        );
        assert!(!core.ride_status().await.unwrap().loading);
        assert_eq!(core.shutdown(Duration::from_secs(3)).await, Shutdown::Ended);
    }

    #[tokio::test(start_paused = true)]
    async fn quitting_waits_for_a_trainer_that_never_confirms_only_up_to_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        radio
            .swallow_modes
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let core = riding(&radio, dir.path()).await;
        // Under the cap the engine's own 2 s deadline releases the ride...
        let t = tokio::time::Instant::now();
        let outcome = core.shutdown(Duration::from_secs(3)).await;
        assert_eq!(outcome, Shutdown::Ended);
        assert!(t.elapsed() <= Duration::from_secs(3), "{:?}", t.elapsed());
        // ... and a cap below it ends the wait at the cap.
        // (quitting does not wait for the devices' release, so the first core still holds
        // the fake radio: the second ride gets its own.)
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        radio
            .swallow_modes
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let core = riding(&radio, dir.path()).await;
        let t = tokio::time::Instant::now();
        assert_eq!(
            core.shutdown(Duration::from_millis(500)).await,
            Shutdown::TimedOut
        );
        assert_eq!(t.elapsed(), Duration::from_millis(500));
    }

    #[tokio::test(start_paused = true)]
    async fn a_quit_overlapping_an_end_is_not_idle_and_the_end_still_awaits_the_release() {
        let dir = tempfile::tempdir().unwrap();
        let mut radio = fake(vec![], false);
        radio.mode_delay = Duration::from_millis(500);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        radio.disconnects.lock().unwrap().clear();
        core.ride_load(None).await.unwrap();
        // A second end of a ride that is already being ended is not a `Conflict` that
        // `shutdown` would read as "nothing loaded": it reports `Ended`; the `ride_end` still
        // returns only after the release, which is what the disconnect below shows.
        let (end, quit) = tokio::join!(core.ride_end(), core.shutdown(Duration::from_secs(3)));
        end.unwrap();
        assert_eq!(quit, Shutdown::Ended);
        assert!(!core.ride_status().await.unwrap().status.loaded);
        assert_eq!(*radio.disconnects.lock().unwrap(), [false]);
    }

    #[tokio::test(start_paused = true)]
    async fn quitting_with_nothing_loaded_is_idle() {
        let dir = tempfile::tempdir().unwrap();
        let core = open(dir.path(), &fake(vec![], false));
        assert_eq!(core.shutdown(Duration::from_secs(3)).await, Shutdown::Idle);
    }

    /// #290: quitting waits for the engine's end only. A strap that is slow to hand its
    /// peripheral back (here beyond both `RELEASE_TIMEOUT` and the cap) neither makes the
    /// quit time out nor delays it, and the trainer's last mode is still the end mode.
    #[tokio::test(start_paused = true)]
    async fn quitting_mid_ride_does_not_wait_for_a_silent_strap() {
        let dir = tempfile::tempdir().unwrap();
        let (core, radio) =
            slow_release_core(dir.path(), Duration::ZERO, Duration::from_secs(30)).await;
        radio.commands.lock().unwrap().clear();
        core.ride_load(Some("builtin-sensor-check".into()))
            .await
            .unwrap();
        core.ride_start().await.unwrap();
        tokio::time::sleep(Duration::from_secs(20)).await;
        let t = tokio::time::Instant::now();
        assert_eq!(core.shutdown(Duration::from_secs(3)).await, Shutdown::Ended);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        let commands = radio.commands.lock().unwrap().clone();
        assert_eq!(
            commands.last().and_then(mode_of),
            Some(ControlMode::Resistance { pct: 0.0 }),
            "{commands:?}"
        );
        let status = core.ride_status().await.unwrap().status;
        assert_eq!(status.state, crank_engine::StreamRideState::Finished);
        assert_eq!(
            radio.hr_released.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the strap is still being released: not reported early"
        );
    }

    /// Kill the loaded ride's engine task, as a panic would: every command answers
    /// `EngineStopped`, its device links are dropped (the core's own hold on the trainer
    /// stays), and the ride-active signal stays up.
    async fn break_engine(core: &Core) {
        core.0
            .state
            .lock()
            .await
            .session
            .as_ref()
            .unwrap()
            .task
            .abort();
        tokio::task::yield_now().await;
    }
    /// The ride was ended by force: recorded as aborted, devices released, the core free for a
    /// new ride, and the whole thing inside the deadline.
    async fn assert_forced_end(core: &Core, radio: &FakeRadio, took: Duration) {
        // a fixed number, not derived from the constants under test
        assert!(took <= Duration::from_secs(2), "{took:?}");
        let rides = core.rides_list().await.unwrap();
        assert_eq!(rides[0].outcome, RideOutcome::Aborted, "{rides:?}");
        assert!(!core.ride_status().await.unwrap().status.loaded);
        assert_eq!(radio.disconnects.lock().unwrap().len(), 1, "released");
        // the dead engine's ride-active signal is cleared for it: no reconnect loop is told
        // a ride is still on
        assert!(!core.0.gate.is_active());
        assert!(!*core.0.active.borrow());
        // the next ride loads and starts: the record writer is free, the signal cleared
        core.0.stall.release();
        core.ride_load(None).await.unwrap();
        core.ride_start().await.unwrap();
    }

    /// Suspend the loaded ride's engine task: it keeps its links and answers nothing.
    fn stall_engine(core: &Core) {
        core.0.stall.on.store(true, Ordering::SeqCst);
    }
    const END_MODE: ControlMode = ControlMode::Resistance { pct: 0.0 };
    /// The mode a command sets, if it is a `SetMode`.
    fn mode_of(command: &TrainerCommand) -> Option<ControlMode> {
        match command {
            TrainerCommand::SetMode { mode, .. } => Some(*mode),
            _ => None,
        }
    }
    /// The core sent the end mode and the trainer had it delivered (the fake takes
    /// `mode_delay` for that) before its link was dropped, which the core told it to do.
    fn assert_end_mode_before_disconnect(radio: &FakeRadio, delivery: Duration, stuck: bool) {
        let commands = radio.commands.lock().unwrap().clone();
        assert_eq!(
            commands.iter().rev().find_map(mode_of),
            Some(END_MODE),
            "stuck={stuck}: {commands:?}"
        );
        assert_eq!(
            commands.last(),
            Some(&TrainerCommand::Disconnect),
            "stuck={stuck}"
        );
        let order = radio.order.lock().unwrap().clone();
        let at = |what| order.iter().rev().find(|(w, _)| *w == what).unwrap().1;
        assert!(
            at("disconnect") >= at("set_mode") + delivery,
            "disconnected before the end mode was delivered: {order:?}"
        );
    }
    /// #331: a stuck engine that still holds its links, and a dead one, leave the trainer in
    /// the end mode: the core sends `Resistance{0}` itself and waits for its delivery before
    /// the link is dropped. Before, the last command was the ride's last ERG target.
    #[tokio::test(start_paused = true)]
    async fn a_forced_end_puts_a_healthy_trainer_into_the_end_mode_before_disconnecting() {
        for stuck in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let mut radio = fake(vec![], false);
            radio.mode_delay = Duration::from_millis(300);
            let core = riding(&radio, dir.path()).await;
            assert_ne!(
                radio.commands.lock().unwrap().last().and_then(mode_of),
                Some(END_MODE)
            );
            if stuck {
                stall_engine(&core);
            } else {
                break_engine(&core).await;
            }
            let t = tokio::time::Instant::now();
            let end = core.ride_end_outcome().await.unwrap();
            assert!(
                matches!(end, RideEnd::ForcedEnd(_)),
                "stuck={stuck}: {end:?}"
            );
            assert!(t.elapsed() <= Duration::from_secs(2), "{:?}", t.elapsed());
            assert_end_mode_before_disconnect(&radio, Duration::from_millis(300), stuck);
        }
    }
    /// A trainer that takes no write does not hold the forced end: the end mode is waited for
    /// `END_MODE_TIMEOUT` and the link is dropped after that.
    #[tokio::test(start_paused = true)]
    async fn a_forced_end_does_not_wait_for_a_stalled_trainer_write() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = riding(&radio, dir.path()).await;
        radio
            .swallow_modes
            .store(true, std::sync::atomic::Ordering::SeqCst);
        stall_engine(&core);
        let t = tokio::time::Instant::now();
        let end = core.ride_end_outcome().await.unwrap();
        assert!(matches!(end, RideEnd::ForcedEnd(_)), "{end:?}");
        let took = t.elapsed();
        assert!(
            took >= Duration::from_millis(1400),
            "waited for it: {took:?}"
        );
        assert!(took <= Duration::from_secs(2), "{took:?}");
        assert_eq!(radio.disconnects.lock().unwrap().len(), 1, "then released");
    }
    /// A `Stop` the engine takes just after the timeout finishes the ride; the record that
    /// wins decides the outcome, so End does not report a forced end of a finished ride.
    #[tokio::test(start_paused = true)]
    async fn a_stop_the_engine_takes_after_the_timeout_decides_the_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = riding(&radio, dir.path()).await;
        // the end mode stays unconfirmed, so the forced end waits long enough to be overtaken
        radio
            .swallow_modes
            .store(true, std::sync::atomic::Ordering::SeqCst);
        stall_engine(&core);
        let end = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_end_outcome().await })
        };
        tokio::time::sleep(STOP_TIMEOUT + Duration::from_millis(200)).await;
        core.0.stall.release();
        let end = end.await.unwrap().unwrap();
        let rides = core.rides_list().await.unwrap();
        match end {
            RideEnd::Ended(Some(_)) => assert_eq!(rides[0].outcome, RideOutcome::Finished),
            RideEnd::ForcedEnd(_) => assert_eq!(rides[0].outcome, RideOutcome::Aborted),
            other @ RideEnd::Ended(None) => panic!("{other:?}"),
        }
        // and a second End says the same
        assert_eq!(
            core.ride_end_outcome().await.unwrap().ride_id(),
            rides[0].id.as_str().into()
        );
    }
    /// #331: an engine blocked in synchronous code (a store write) cannot be aborted: End
    /// still answers inside the deadline, the task is detached, and a second End does not
    /// block. Real time, on threads: the stuck task occupies one of them for 4 s.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_engine_stuck_in_synchronous_code_does_not_hang_end() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.ride_load(None).await.unwrap();
        {
            let (engine, handle) =
                Engine::new(core.0.clock.clone(), None, None, Arc::new(|_: bool| {}));
            let mut st = core.0.state.lock().await;
            let session = st.session.as_mut().unwrap();
            session.task.abort();
            session.handle = handle;
            session.task = EngineTask::Tokio(tokio::spawn(async move {
                let _engine = engine;
                // after one yield, as an engine reaching a blocking store write
                tokio::task::yield_now().await;
                std::thread::sleep(Duration::from_secs(4));
            }));
        }
        let t = std::time::Instant::now();
        let (a, b) = tokio::time::timeout(
            Duration::from_secs(3),
            futures_util::future::join(core.ride_end_outcome(), core.ride_end_outcome()),
        )
        .await
        .expect("End answers, and so does a second one");
        let (a, b) = (a.unwrap(), b.unwrap());
        assert!(matches!(a, RideEnd::ForcedEnd(_)), "{a:?}");
        assert_eq!(a, b);
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
        // the state lock is free, and the next ride loads
        core.ride_load(None).await.unwrap();
    }
    /// Wait (real time) until `done`, for at most `limit`.
    async fn until(limit: Duration, done: impl Fn() -> bool) {
        let t = std::time::Instant::now();
        while !done() {
            assert!(t.elapsed() < limit, "not within {limit:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    /// #344: an engine on its own thread, woken by its timer mid-ride, blocks that thread in a
    /// record write (`std::thread::sleep`, 4 s) while it holds both links. End answers inside
    /// [`end_bound`]; the core releases both devices itself, and the release had completed when
    /// End returned (the immediate-release assumption of `end_bound`, as in [`end_beside`]); the next ride loads at once (no
    /// `driver already connected`); when the engine wakes, its late write is refused by the
    /// store's id guard, the ride stays aborted and the next ride's ride-active signal is not
    /// touched. Real time, on the production host and clock.
    async fn an_engine_thread_blocked_mid_ride(host: EngineHost) {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open_on(dir.path(), &radio, host);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.device_pair("hrm".into(), DeviceRole::HeartRate)
            .await
            .unwrap();
        // the pairings connect and drop both
        until(Duration::from_secs(5), || {
            radio.disconnects.lock().unwrap().len() == 1
                && radio.hr_released.load(Ordering::SeqCst) == 1
        })
        .await;
        radio.disconnects.lock().unwrap().clear();
        let loaded = core
            .ride_load(Some("builtin-sensor-check".into()))
            .await
            .unwrap();
        assert!(loaded.hr_linked);
        core.ride_start().await.unwrap();
        let release = core
            .0
            .state
            .lock()
            .await
            .session
            .as_ref()
            .unwrap()
            .release
            .clone();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let block = core.0.block_write.clone();
        *block.next.lock().unwrap() = Some(Duration::from_secs(4));
        until(Duration::from_secs(3), || {
            block.blocked.load(Ordering::SeqCst)
        })
        .await;
        let t = std::time::Instant::now();
        let end = tokio::time::timeout(Duration::from_secs(10), core.ride_end_outcome())
            .await
            .expect("End answers")
            .unwrap();
        let took = t.elapsed();
        assert!(took <= end_bound(), "{took:?}, bound {:?}", end_bound());
        assert!(
            block.late.lock().unwrap().is_none(),
            "End waited for the blocked engine"
        );
        assert_eq!(end, RideEnd::ForcedEnd(loaded.ride_id.clone()));
        // released by the core, with the engine still holding its links: both relays saw their
        // driver close its stream (the release finished, not just started), and the fake
        // counted the trainer's disconnect and the strap's release
        assert!(*release.trainer.borrow(), "the trainer was not released");
        assert!(
            release.hr.as_ref().is_some_and(|hr| *hr.borrow()),
            "the strap was not released"
        );
        assert_eq!(radio.disconnects.lock().unwrap().len(), 1, "trainer");
        assert_eq!(radio.hr_released.load(Ordering::SeqCst), 2, "strap");
        let next = core.ride_load(None).await.unwrap();
        assert!(*core.0.active.borrow());
        // the engine wakes
        until(Duration::from_secs(5), || {
            block.late.lock().unwrap().is_some()
        })
        .await;
        let late = block.late.lock().unwrap().clone().unwrap();
        assert!(
            late.as_ref()
                .is_err_and(|e| e.0.contains("ride is not live")),
            "{late:?}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        let rides = core.rides_list().await.unwrap();
        assert_eq!(rides[0].id, loaded.ride_id);
        assert_eq!(rides[0].outcome, RideOutcome::Aborted, "{rides:?}");
        let loaded_now = core
            .0
            .state
            .lock()
            .await
            .session
            .as_ref()
            .map(|s| s.ride_id.clone());
        assert_eq!(loaded_now, Some(next.ride_id));
        assert!(core.ride_status().await.unwrap().status.loaded);
        assert!(core.0.gate.is_active() && *core.0.active.borrow());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_engine_thread_blocked_mid_ride_does_not_hold_end_on_a_multi_thread_core() {
        an_engine_thread_blocked_mid_ride(EngineHost::Thread).await;
    }
    #[tokio::test(flavor = "current_thread")]
    async fn an_engine_thread_blocked_mid_ride_does_not_hold_end_on_a_current_thread_core() {
        an_engine_thread_blocked_mid_ride(EngineHost::Thread).await;
    }
    /// A started ride on its own engine thread (real time), with a stream of window `main` and a
    /// power subscription on it, whose engine thread is then blocked for `wedge` in a record
    /// write, as a slow store would block it.
    async fn a_wedged_ride(
        dir: &std::path::Path,
        radio: &FakeRadio,
        wedge: Duration,
    ) -> (Core, RideLoaded, StreamId, SubId) {
        let core = open_on(dir, radio, EngineHost::Thread);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        until(Duration::from_secs(5), || {
            radio.disconnects.lock().unwrap().len() == 1
        })
        .await;
        radio.disconnects.lock().unwrap().clear();
        let loaded = core
            .ride_load(Some("builtin-sensor-check".into()))
            .await
            .unwrap();
        core.ride_start().await.unwrap();
        let stream = core
            .open_stream(Some("main".into()), |_| Ok(()))
            .await
            .unwrap();
        let opts = SubscribeOpts {
            rate_hz: 1.0,
            filters: Vec::new(),
        };
        let sub = core
            .subscribe(Some("main"), stream, MetricId::Power, opts)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let block = core.0.block_write.clone();
        *block.next.lock().unwrap() = Some(wedge);
        until(Duration::from_secs(3), || {
            block.blocked.load(Ordering::SeqCst)
        })
        .await;
        (core, loaded, stream, sub)
    }
    /// Real-time slack on top of a derived bound: each bounded wait is a timer that fires at
    /// or after its deadline, and a loaded runner (macOS CI) delays every one of them.
    const SCHEDULING_SLACK: Duration = Duration::from_millis(500);
    /// The longest a forced End may take in real time **given drivers that release at once**,
    /// derived from every wait `ride_end_outcome` passes through, in order:
    /// 1. `STOP_TIMEOUT`: the wedged engine never takes `Stop` (under the state lock);
    /// 2. `END_MODE_TIMEOUT`: the trainer's confirmation of `Resistance{0}` (a started ride);
    /// 3. `ABORT_JOIN_TIMEOUT`: the aborted engine thread is blocked in a store write, so it
    ///    ends within the wait or is detached (`reap`);
    /// 4. `STORE_TIMEOUT`: closing the record on the blocking pool;
    /// 5. `RELEASE_TIMEOUT`: `end_ride` then awaits the devices' release
    ///    (`LinkRelease::wait`). The fake drivers of these tests close their streams as soon
    ///    as `Takeover::Release` arrives, so this wait is ~0 here, not 3 s: [`end_beside`] and
    ///    `an_engine_thread_blocked_mid_ride` assert that the release had completed when End
    ///    returned, and an unbounded or
    ///    slow release would exceed this bound. It is therefore not summed in.
    ///
    /// Steps 1-4 are 2.25 s (the "about 2.25 s at worst" of `END_MODE_TIMEOUT`); the true
    /// worst case with a driver that never releases is 5.25 s. `REQUEST_TIMEOUT` waits of
    /// other callers run beside End, never in front of it (no state lock, #351). A fixed
    /// 2 s was below the 2.25 s, so a runner that spends the full `END_MODE_TIMEOUT` failed
    /// it by 28 ms (#358). `SCHEDULING_SLACK` covers the timers' overshoot.
    fn end_bound() -> Duration {
        STOP_TIMEOUT + END_MODE_TIMEOUT + ABORT_JOIN_TIMEOUT + STORE_TIMEOUT + SCHEDULING_SLACK
    }
    /// Press End on a wedged ride while `pending` is under way: End answers inside
    /// [`end_bound`], by force, and the next ride loads. Returns how long End took.
    async fn end_beside(core: &Core, loaded: &RideLoaded) -> Duration {
        // the pending request has reached the engine (and, before #351, taken the state lock)
        tokio::time::sleep(Duration::from_millis(100)).await;
        let release = core
            .0
            .state
            .lock()
            .await
            .session
            .as_ref()
            .unwrap()
            .release
            .clone();
        let t = std::time::Instant::now();
        let end = tokio::time::timeout(Duration::from_secs(10), core.ride_end_outcome())
            .await
            .expect("End answers")
            .unwrap();
        let took = t.elapsed();
        // the immediate-release assumption of `end_bound`
        assert!(*release.trainer.borrow(), "the trainer was not released");
        assert!(
            release.hr.as_ref().is_none_or(|hr| *hr.borrow()),
            "the strap was not released"
        );
        assert!(
            took <= end_bound(),
            "End took {took:?}, bound {:?}",
            end_bound()
        );
        assert_eq!(end, RideEnd::ForcedEnd(loaded.ride_id.clone()));
        core.ride_load(None).await.unwrap();
        took
    }
    /// `request`, with how long it took from its first poll, for [`assert_unresponsive`].
    async fn timed<T>(
        what: &'static str,
        request: impl std::future::Future<Output = Result<T, ApiError>>,
    ) -> (&'static str, Result<(), ApiError>, Duration) {
        let t = std::time::Instant::now();
        let answer = request.await.map(|_| ());
        (what, answer, t.elapsed())
    }
    /// The request answered `EngineUnresponsive` within `REQUEST_TIMEOUT` (the one wait it
    /// passes through) plus slack, `took` being measured by the request itself.
    fn assert_unresponsive(what: &str, answer: Result<(), ApiError>, took: Duration) {
        assert!(
            took <= REQUEST_TIMEOUT + SCHEDULING_SLACK,
            "{what} took {took:?}"
        );
        let e = answer.expect_err(what);
        assert_eq!(e.code, ErrorCode::EngineUnresponsive, "{what}: {e:?}");
    }
    /// #351: a window that (re)opens its stream while the engine thread is wedged (a webview
    /// reload) is told the engine does not answer, and End is not held behind it: the state
    /// lock is not held across the engine's answer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_stream_opened_while_the_engine_is_wedged_does_not_hold_end() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let (core, loaded, _, _) = a_wedged_ride(dir.path(), &radio, Duration::from_secs(6)).await;
        let opening = tokio::spawn({
            let core = core.clone();
            async move {
                let t = std::time::Instant::now();
                let opened = core.open_stream(Some("main".into()), |_| Ok(())).await;
                (opened, t.elapsed())
            }
        });
        end_beside(&core, &loaded).await;
        let (opened, took) = tokio::time::timeout(Duration::from_secs(10), opening)
            .await
            .unwrap()
            .unwrap();
        assert_unresponsive("open_stream", opened.map(|_| ()), took);
    }
    /// #351: a destroyed window whose close finds the engine's request channel full (the
    /// engine is wedged) does not hold End either.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_window_closed_on_a_full_channel_of_a_wedged_engine_does_not_hold_end() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let (core, loaded, stream, _) =
            a_wedged_ride(dir.path(), &radio, Duration::from_secs(6)).await;
        // the engine's stream requests queue up to 64; acks are taken without an answer
        for seq in 0..64 {
            core.stream_ack(Some("main"), stream, seq).await.unwrap();
        }
        let closing = tokio::spawn({
            let core = core.clone();
            async move {
                let t = std::time::Instant::now();
                core.close_window("main".into()).await;
                t.elapsed()
            }
        });
        end_beside(&core, &loaded).await;
        let took = tokio::time::timeout(Duration::from_secs(10), closing)
            .await
            .unwrap()
            .unwrap();
        // the close waits `REQUEST_TIMEOUT` for the engine's full channel, not for End
        assert!(
            took <= REQUEST_TIMEOUT + SCHEDULING_SLACK,
            "close_window took {took:?}"
        );
    }
    /// #351: every other request the core sends a wedged engine answers `EngineUnresponsive`
    /// within its bound instead of waiting for the engine, and End still works beside them.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn every_request_to_a_wedged_engine_is_answered_within_its_bound() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let (core, loaded, stream, sub) =
            a_wedged_ride(dir.path(), &radio, Duration::from_secs(6)).await;
        for seq in 0..64 {
            core.stream_ack(Some("main"), stream, seq).await.unwrap();
        }
        let main = Some("main");
        let opts = SubscribeOpts {
            rate_hz: 1.0,
            filters: Vec::new(),
        };
        // each request measures its own wait: they run side by side, so each is one
        // `REQUEST_TIMEOUT`, not the sum
        let (pause, resume, skip, start, subscribe, unsubscribe, snapshot, ack) = tokio::join!(
            timed("ride_pause", core.ride_pause()),
            timed("ride_resume", core.ride_resume()),
            timed("ride_skip", core.ride_skip()),
            timed("ride_start", core.ride_start()),
            timed(
                "subscribe",
                core.subscribe(main, stream, MetricId::Cadence, opts)
            ),
            timed("unsubscribe", core.unsubscribe(main, stream, sub)),
            timed("stream_snapshot", core.stream_snapshot(main, stream)),
            // the request channel is full
            timed("stream_ack", core.stream_ack(main, stream, 64)),
        );
        for (what, answer, took) in [
            pause,
            resume,
            skip,
            start,
            subscribe,
            unsubscribe,
            snapshot,
            ack,
        ] {
            assert_unresponsive(what, answer, took);
        }
        end_beside(&core, &loaded).await;
    }
    /// #351: an engine that does not take its ride (`Load`) fails the load within the bound,
    /// with the devices it was built with released by the core, so the next load works.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_load_the_engine_does_not_take_fails_within_its_bound_and_releases_the_devices() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open_on(dir.path(), &radio, EngineHost::Thread);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        until(Duration::from_secs(5), || {
            radio.disconnects.lock().unwrap().len() == 1
        })
        .await;
        radio.disconnects.lock().unwrap().clear();
        stall_engine(&core);
        let t = std::time::Instant::now();
        let loaded = tokio::time::timeout(Duration::from_secs(10), core.ride_load(None))
            .await
            .expect("the load answers");
        assert_unresponsive("ride_load", loaded.map(|_| ()), t.elapsed());
        assert_eq!(radio.disconnects.lock().unwrap().len(), 1, "released");
        let status = core.ride_status().await.unwrap();
        assert!(!status.loading && !status.status.loaded);
        core.0.stall.release();
        core.ride_load(None).await.unwrap();
        core.ride_start().await.unwrap();
    }
    /// #331: a stuck engine does not wedge the replacing `ride_load` or `discard` either.
    #[tokio::test(start_paused = true)]
    async fn a_stuck_engine_does_not_hold_a_replacing_load() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.ride_load(None).await.unwrap();
        stall_engine(&core);
        let t = tokio::time::Instant::now();
        // the replacement's engine would stall too: it is let go once the old one is dealt with
        let load = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_load(None).await })
        };
        // Inside the deadline the stuck ride is gone and the lock is free (the replacement is
        // building, against an engine that stalls as well); nothing is waiting for the old one.
        tokio::time::sleep(Duration::from_millis(1600)).await;
        {
            let st = tokio::time::timeout(Duration::from_millis(100), core.0.state.lock())
                .await
                .expect("the state lock is held by an await of the stuck engine");
            assert!(st.session.is_none(), "the stuck ride is still there");
        }
        core.0.stall.release();
        tokio::time::timeout(Duration::from_secs(10), load)
            .await
            .expect("the load is not wedged")
            .unwrap()
            .unwrap();
        assert!(t.elapsed() <= Duration::from_secs(4), "{:?}", t.elapsed());
        assert!(core.ride_status().await.unwrap().status.loaded);
    }
    #[tokio::test(start_paused = true)]
    async fn a_stuck_engine_does_not_hold_a_discard() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.ride_load(None).await.unwrap();
        stall_engine(&core);
        let session = core.0.state.lock().await.session.take().unwrap();
        let t = tokio::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(10), core.discard(session))
            .await
            .expect("the discard is not wedged");
        assert!(t.elapsed() <= Duration::from_secs(3), "{:?}", t.elapsed());
        core.0.stall.release();
        core.ride_load(None).await.unwrap();
    }
    /// #331: an engine that is stuck (never answers `Stop`) does not hold `ride_end` or the
    /// state lock for ever.
    #[tokio::test(start_paused = true)]
    async fn a_stuck_engine_is_ended_by_force_within_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = riding(&radio, dir.path()).await;
        stall_engine(&core);
        let t = tokio::time::Instant::now();
        let end = tokio::time::timeout(Duration::from_secs(30), core.ride_end_outcome())
            .await
            .expect("End answers")
            .unwrap();
        let took = t.elapsed();
        assert!(matches!(end, RideEnd::ForcedEnd(_)), "{end:?}");
        assert_forced_end(&core, &radio, took).await;
    }
    /// #331: an engine whose task is gone no longer fails `ride_end` with `EngineStopped`.
    #[tokio::test(start_paused = true)]
    async fn a_dead_engine_is_ended_by_force() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = riding(&radio, dir.path()).await;
        break_engine(&core).await;
        let t = tokio::time::Instant::now();
        let end = core.ride_end_outcome().await.unwrap();
        let took = t.elapsed();
        assert!(matches!(end, RideEnd::ForcedEnd(_)), "{end:?}");
        assert_forced_end(&core, &radio, took).await;
    }
    /// #355: a forced end names the ride it ends. If a newer ride was installed meanwhile, the
    /// old ride's forced end must not cut off or end-mode the newer one: the session is looked
    /// up by id, not taken as "the" session. The test proves the takeover stays `Engine` and
    /// the newer trainer gets no command, and its ride-active signal and gate stay up (#370).
    /// A simulation: the late forced end is a direct `force_end(&old_id)` call after the old
    /// ride ended and the newer one loaded; the race through the API is
    /// `a_late_forced_end_of_a_replaced_ride_leaves_the_newer_rides_signal_up`.
    #[tokio::test(start_paused = true)]
    async fn a_forced_end_of_an_old_ride_leaves_the_ride_that_replaced_it_alone() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = riding(&radio, dir.path()).await;
        let old_id = core
            .0
            .state
            .lock()
            .await
            .session
            .as_ref()
            .unwrap()
            .ride_id
            .clone();
        // the old ride ends (a forced end that is still on its way is the late one) and a
        // newer ride takes its place
        core.ride_end_outcome().await.unwrap();
        let newer = core.ride_load(None).await.unwrap();
        assert_ne!(newer.ride_id, old_id);
        // whatever the two rides' own set-up and tear-down still send has been sent
        tokio::time::sleep(Duration::from_secs(5)).await;
        radio.commands.lock().unwrap().clear();
        core.force_end(&old_id, true).await;
        {
            let st = core.0.state.lock().await;
            let session = st.session.as_ref().expect("the newer ride is still loaded");
            assert_eq!(session.ride_id, newer.ride_id);
            assert_eq!(*session.takeover.borrow(), Takeover::Engine);
        }
        // nor clear its ride-active signal and gate (#370)
        assert!(core.0.gate.is_active() && *core.0.active.borrow());
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(
            radio.commands.lock().unwrap().is_empty(),
            "the newer ride's trainer was sent {:?}",
            radio.commands.lock().unwrap()
        );
    }
    /// #370: an End and a replacing load overlap on a dead, loaded (unstarted) ride, through the
    /// public API. Both find the engine gone and decide to end the ride by force; the End is
    /// held between that decision and its forced end (a task that is not scheduled for a
    /// while, `force_hook`), so the load's forced end runs first and the load installs the
    /// newer ride. The End's forced end then runs late, for the old id: it must not clear the
    /// newer ride's ride-active signal and gate (which would turn the newer trainer's command
    /// gate off and fire the newer ride's reaper) nor record itself as the core's last end.
    #[tokio::test(start_paused = true)]
    async fn a_late_forced_end_of_a_replaced_ride_leaves_the_newer_rides_signal_up() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        let old = core.ride_load(None).await.unwrap();
        break_engine(&core).await;
        core.0.force_hook.enabled.store(true, Ordering::SeqCst);
        let end = tokio::spawn({
            let core = core.clone();
            async move { core.ride_end_outcome().await }
        });
        core.0.force_hook.reached.notified().await;
        let newer = core.ride_load(None).await.unwrap();
        assert_ne!(newer.ride_id, old.ride_id);
        assert!(core.0.gate.is_active() && *core.0.active.borrow());
        core.0.force_hook.resume.notify_one();
        let ended = end.await.unwrap().unwrap();
        assert_eq!(ended, RideEnd::ForcedEnd(old.ride_id.clone()));
        tokio::time::sleep(Duration::from_secs(1)).await;
        let loaded = core
            .0
            .state
            .lock()
            .await
            .session
            .as_ref()
            .map(|s| s.ride_id.clone());
        assert_eq!(loaded, Some(newer.ride_id), "the newer ride was reaped");
        assert!(core.0.gate.is_active(), "the newer ride's gate was cleared");
        assert!(
            *core.0.active.borrow(),
            "the newer ride's signal was cleared"
        );
        assert!(
            core.0.state.lock().await.last_forced.is_none(),
            "the old ride's forced end was recorded as the newer ride's last end"
        );
    }
    /// #355: `open_stream` asks the engine without the state lock, so the ride may be replaced
    /// before the old engine answers. The stream then belongs to no ride: it is refused, and
    /// the newer ride does not get a stream that its own engine never opened. A simulation:
    /// the old engine is stalled, then the session's `ride_id` is renamed in place to stand
    /// for the newer ride before the engine is released; no real engine answers late.
    #[tokio::test(start_paused = true)]
    async fn a_stream_the_old_engine_opens_after_the_ride_was_replaced_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = riding(&radio, dir.path()).await;
        stall_engine(&core);
        let opening = tokio::spawn({
            let core = core.clone();
            async move { core.open_stream(Some("main".into()), |_| Ok(())).await }
        });
        // the request is with the (stalled) old engine
        tokio::time::sleep(Duration::from_millis(100)).await;
        // the ride is replaced: the session now under that name is another ride's
        core.0.state.lock().await.session.as_mut().unwrap().ride_id = "newer-ride".into();
        core.0.stall.release();
        let opened = opening.await.unwrap();
        let e = opened.expect_err("the old engine's stream is refused");
        assert_eq!(e.code, ErrorCode::Conflict, "{e:?}");
        let st = core.0.state.lock().await;
        assert!(st.session.as_ref().unwrap().streams.is_empty());
    }
    /// #331: two Ends at once on a stuck engine neither deadlock nor disagree.
    #[tokio::test(start_paused = true)]
    async fn two_concurrent_ends_of_a_stuck_engine_have_one_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = riding(&radio, dir.path()).await;
        stall_engine(&core);
        let both = tokio::time::timeout(
            Duration::from_secs(30),
            futures_util::future::join(core.ride_end_outcome(), core.ride_end_outcome()),
        )
        .await
        .expect("neither End hangs");
        let (a, b) = (both.0.unwrap(), both.1.unwrap());
        assert!(matches!(a, RideEnd::ForcedEnd(_)), "{a:?}");
        assert_eq!(a, b);
        // and a later one reports it again
        assert_eq!(core.ride_end_outcome().await.unwrap(), a);
        assert_eq!(radio.disconnects.lock().unwrap().len(), 1, "released once");
        let rides = core.rides_list().await.unwrap();
        assert_eq!(rides[0].outcome, RideOutcome::Aborted);
    }
    /// #331: quitting inherits the bound: a stuck engine is ended and recorded inside the quit
    /// cap instead of timing the quit out.
    #[tokio::test(start_paused = true)]
    async fn quitting_with_a_stuck_engine_ends_and_records_the_ride_inside_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = riding(&radio, dir.path()).await;
        stall_engine(&core);
        let t = tokio::time::Instant::now();
        assert_eq!(core.shutdown(Duration::from_secs(3)).await, Shutdown::Ended);
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        let rides = core.rides_list().await.unwrap();
        assert_eq!(rides[0].outcome, RideOutcome::Aborted);
    }

    /// A core with a trainer (releasing in `trainer`) and a strap (in `strap`) paired.
    async fn slow_release_core(
        dir: &std::path::Path,
        trainer: Duration,
        strap: Duration,
    ) -> (Core, FakeRadio) {
        let mut radio = fake(vec![], false);
        radio.disconnect_delay = trainer;
        radio.hr_delay = strap;
        let core = open(dir, &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.device_pair("hrm".into(), DeviceRole::HeartRate)
            .await
            .unwrap();
        // the pairing connects and drops both; let their releases finish
        tokio::time::sleep(Duration::from_secs(5)).await;
        radio.disconnects.lock().unwrap().clear();
        radio
            .hr_released
            .store(0, std::sync::atomic::Ordering::SeqCst);
        (core, radio)
    }

    /// #290: `ride_end` returns only once the trainer has handed its peripheral back, so a
    /// pairing right after it does not find it held.
    #[tokio::test(start_paused = true)]
    async fn ride_end_waits_for_the_trainer_release() {
        let dir = tempfile::tempdir().unwrap();
        let (core, radio) =
            slow_release_core(dir.path(), Duration::from_secs(2), Duration::ZERO).await;
        core.ride_load(None).await.unwrap();
        core.ride_end().await.unwrap();
        assert_eq!(radio.disconnects.lock().unwrap().len(), 1, "released");
    }
    /// #290: the same for the strap, on both paths that release a ride: `ride_end` and a
    /// cancelled load (`discard`).
    #[tokio::test(start_paused = true)]
    async fn ride_end_waits_for_the_strap_release() {
        let dir = tempfile::tempdir().unwrap();
        let (core, radio) =
            slow_release_core(dir.path(), Duration::ZERO, Duration::from_secs(2)).await;
        assert!(core.ride_load(None).await.unwrap().hr_linked);
        core.ride_end().await.unwrap();
        assert_eq!(
            radio.hr_released.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the strap is released when ride_end returns"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn a_discarded_load_waits_for_the_strap_release() {
        let dir = tempfile::tempdir().unwrap();
        let (core, radio) =
            slow_release_core(dir.path(), Duration::ZERO, Duration::from_secs(2)).await;
        core.ride_load(None).await.unwrap();
        let session = core.0.state.lock().await.session.take().unwrap();
        core.discard(session).await;
        assert_eq!(
            radio.hr_released.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the strap is released when discard returns"
        );
        assert_eq!(radio.disconnects.lock().unwrap().len(), 1);
    }

    /// #290: an engine that never drops the ride-active signal (its task died with it up)
    /// cannot hold a replacing `ride_load` for ever: after `REAP_TIMEOUT` the session is
    /// reaped by force and the new ride loads.
    #[tokio::test(start_paused = true)]
    async fn a_replacing_load_is_not_held_by_an_engine_that_never_releases() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.ride_load(None).await.unwrap();
        core.0.pin_active.store(true, Ordering::SeqCst);
        let replaced =
            tokio::time::timeout(REAP_TIMEOUT + Duration::from_secs(5), core.ride_load(None)).await;
        core.0.pin_active.store(false, Ordering::SeqCst);
        replaced.expect("the load is not left waiting").unwrap();
        assert!(core.ride_status().await.unwrap().status.loaded);
    }
    /// #290: a cancelled load whose engine never releases clears `loading` after
    /// `REAP_TIMEOUT` instead of leaving the core refusing loads for ever.
    #[tokio::test(start_paused = true)]
    async fn a_discarded_load_is_not_held_by_an_engine_that_never_releases() {
        let dir = tempfile::tempdir().unwrap();
        core_pin_then_cut_off(dir.path()).await;
    }
    async fn core_pin_then_cut_off(dir: &std::path::Path) {
        let mut radio = fake(vec![], false);
        radio.connect_delay = Duration::from_secs(10);
        let core = open(dir, &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        let load = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_load(None).await })
        };
        tokio::time::sleep(Duration::from_secs(1)).await;
        let held = core.0.state.lock().await;
        tokio::time::sleep(Duration::from_secs(15)).await;
        core.0.pin_active.store(true, Ordering::SeqCst);
        load.abort();
        assert!(load.await.unwrap_err().is_cancelled());
        drop(held);
        tokio::time::sleep(Duration::from_secs(9)).await;
        assert!(core.0.loading.load(Ordering::SeqCst), "still waiting");
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!core.0.loading.load(Ordering::SeqCst), "bounded");
        assert!(core.0.state.lock().await.session.is_none());
        core.0.pin_active.store(false, Ordering::SeqCst);
        core.ride_load(None).await.unwrap();
    }

    /// #290: the relay of a driver that never closes its stream ends `RELEASE_TIMEOUT`
    /// after the engine is gone, not before and not never.
    #[tokio::test(start_paused = true)]
    async fn the_relay_of_a_driver_that_never_closes_ends_at_the_release_timeout() {
        let (driver_tx, driver_rx) = tokio::sync::mpsc::channel::<u8>(4);
        let (_takeover, takeover) = watch::channel(Takeover::Engine);
        let (engine_rx, closed) = relay_events(driver_rx, false, |_| {}, takeover);
        drop(engine_rx);
        tokio::time::sleep(Duration::from_millis(2900)).await;
        assert!(!*closed.borrow(), "still listening");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(*closed.borrow(), "bounded");
        drop(driver_tx);
    }

    /// #290: a second `ride_end` (or a quit racing the UI's end) does not return before the
    /// devices of the first are released.
    #[tokio::test(start_paused = true)]
    async fn a_second_ride_end_awaits_the_same_release() {
        let dir = tempfile::tempdir().unwrap();
        let (core, radio) =
            slow_release_core(dir.path(), Duration::from_secs(2), Duration::ZERO).await;
        core.ride_load(None).await.unwrap();
        core.ride_start().await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        let first = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_end().await })
        };
        // The session is reaped; the trainer still takes 2 s to hand itself back.
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(core.0.state.lock().await.session.is_none());
        assert!(radio.disconnects.lock().unwrap().is_empty(), "releasing");
        let second = core.ride_end().await.unwrap();
        assert_eq!(
            radio.disconnects.lock().unwrap().len(),
            1,
            "the second call returned before the release"
        );
        assert_eq!(first.await.unwrap().unwrap(), second);
    }

    /// #290: an engine task that hangs (not dies) with the ride-active signal up cannot wedge
    /// the core: the forced reap aborts it instead of awaiting it under the state lock.
    #[tokio::test(start_paused = true)]
    async fn a_hung_engine_task_is_aborted_not_awaited() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let core = open(dir.path(), &radio);
        core.device_pair("neo".into(), DeviceRole::Trainer)
            .await
            .unwrap();
        core.ride_load(None).await.unwrap();
        {
            let mut st = core.0.state.lock().await;
            let session = st.session.as_mut().unwrap();
            session.task.abort();
            session.task = EngineTask::Tokio(tokio::spawn(std::future::pending()));
        }
        let replace = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_load(None).await })
        };
        // The forced reap starts at `REAP_TIMEOUT`. It aborts the hung task, so the state
        // lock is free again at once; awaiting it instead would hold the lock for a further
        // `RELEASE_TIMEOUT`.
        tokio::time::sleep(REAP_TIMEOUT + Duration::from_secs(1)).await;
        let lock = tokio::time::timeout(Duration::from_millis(500), core.0.state.lock()).await;
        assert!(
            lock.is_ok(),
            "the state lock is held by an await of the hung engine"
        );
        drop(lock);
        tokio::time::timeout(RELEASE_TIMEOUT + Duration::from_secs(5), replace)
            .await
            .expect("the load is not wedged")
            .unwrap()
            .unwrap();
        assert!(core.ride_status().await.unwrap().status.loaded);
    }

    /// #290: two overlapping ends of an unstarted ride: the loser's `Stop` finds the engine
    /// already ending (`WrongState`), and it must still wait for the same release.
    #[tokio::test(start_paused = true)]
    async fn two_concurrent_ends_of_an_unstarted_ride_both_await_the_release() {
        let dir = tempfile::tempdir().unwrap();
        let (core, radio) =
            slow_release_core(dir.path(), Duration::from_secs(2), Duration::from_secs(2)).await;
        core.ride_load(None).await.unwrap();
        let ended = |core: Core, radio: FakeRadio| async move {
            let outcome = core.ride_end().await;
            let released = (
                radio.disconnects.lock().unwrap().len(),
                radio.hr_released.load(std::sync::atomic::Ordering::SeqCst),
            );
            (outcome, released)
        };
        let (first, second) = tokio::join!(
            ended(core.clone(), radio.clone()),
            ended(core.clone(), radio.clone())
        );
        for (outcome, released) in [first, second] {
            assert_eq!(outcome.unwrap(), None);
            assert_eq!(
                released,
                (1, 1),
                "returned before the devices were released"
            );
        }
    }

    /// A trainer link whose driver side the test plays, put behind the core's relays and the
    /// command gate as `build_session` does. Returns the engine's link, the core's hold, the
    /// takeover, the driver's command receiver and its event sender.
    fn gated_link() -> (
        TrainerLink,
        TrainerHold,
        Arc<watch::Sender<Takeover>>,
        tokio::sync::mpsc::Receiver<TrainerCommand>,
        tokio::sync::mpsc::Sender<TrainerEvent>,
    ) {
        let (control, driver) = tokio::sync::mpsc::channel(64);
        let (events, driver_events) = tokio::sync::mpsc::channel(256);
        let mut link = TrainerLink {
            info: DeviceInfo::default(),
            caps: crank_devices::TrainerCaps {
                erg: true,
                sim: true,
                resistance: true,
                max_resistance_n: None,
                max_target_w: 2000,
            },
            control,
            events: driver_events,
            seq: SeqSource::default(),
        };
        let takeover = Arc::new(watch::channel(Takeover::Engine).0);
        let (hold, _release) = relay_links("ride-gate", &mut link, None, &takeover);
        (link, hold, takeover, driver, events)
    }
    fn erg_command(link: &TrainerLink, target_w: u16) -> TrainerCommand {
        TrainerCommand::SetMode {
            mode: ControlMode::Erg { target_w },
            seq: link.seq.next(),
        }
    }
    /// Let the gate and the relays run.
    async fn settle() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }
    /// #345: from the takeover on, no engine command reaches the driver, neither one queued
    /// before it (the engine woke and sent it, the gate had not forwarded it yet) nor one sent
    /// after it; each is dropped and logged. The core's end mode goes through, and it is the
    /// trainer's last mode. Before, the engine shared the driver's channel: `Erg{92}`,
    /// `Erg{109}` and `Erg{0}` followed the core's `Resistance{0}` (#334 re-review).
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn after_the_takeover_no_engine_command_reaches_the_trainer() {
        let (link, hold, takeover, mut driver, events) = gated_link();
        // the engine has the session: forwarded
        let before = erg_command(&link, 200);
        link.control.try_send(before).unwrap();
        settle().await;
        assert_eq!(driver.try_recv(), Ok(before));
        // the engine wakes from a wedge and queues two targets; the gate has not run yet
        let woken = [erg_command(&link, 92), erg_command(&link, 109)];
        for command in woken {
            link.control.try_send(command).unwrap();
        }
        takeover.send_replace(Takeover::CutOff);
        // the trainer confirms what it is given
        let trainer = tokio::spawn(async move {
            let mut seen = Vec::new();
            while let Some(command) = driver.recv().await {
                if let TrainerCommand::SetMode { mode, seq } = command {
                    let _ = events.send(TrainerEvent::Delivered { mode, seq }).await;
                }
                seen.push(command);
            }
            seen
        });
        assert!(force_end_mode("ride-gate", &hold).await, "confirmed");
        // and one more after the end mode
        let late = erg_command(&link, 0);
        link.control.try_send(late).unwrap();
        settle().await;
        let dropped = hold.dropped.lock().unwrap().clone();
        assert_eq!(dropped, [woken[0], woken[1], late]);
        release_links("ride-gate", &hold, &takeover);
        drop((link, hold));
        let seen = trainer.await.unwrap();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(mode_of(&seen[0]), Some(END_MODE), "{seen:?}");
        assert_eq!(seen[1], TrainerCommand::Disconnect);
    }
    /// #345: an engine command that waits on a full driver channel when the core takes over
    /// is dropped, not forwarded after the core's end mode, and does not hold it.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn an_engine_command_waiting_on_a_full_driver_channel_is_dropped_at_the_takeover() {
        let (link, hold, takeover, mut driver, _events) = gated_link();
        // the driver takes nothing: its channel fills, and the gate waits with the next one
        let mut sent = Vec::new();
        for target_w in 100..165 {
            let command = erg_command(&link, target_w);
            link.control.try_send(command).unwrap();
            sent.push(command);
            settle().await;
        }
        takeover.send_replace(Takeover::CutOff);
        settle().await;
        assert_eq!(*hold.dropped.lock().unwrap(), [sent[64]]);
        // the driver drains its backlog, and the core's end mode follows it
        let ending = tokio::spawn({
            let hold = hold.clone();
            async move { force_end_mode("ride-gate", &hold).await }
        });
        let mut seen = Vec::new();
        for _ in 0..65 {
            seen.push(driver.recv().await.unwrap());
        }
        assert_eq!(seen[..64], sent[..64]);
        assert_eq!(mode_of(&seen[64]), Some(END_MODE));
        assert!(!ending.await.unwrap(), "nothing confirmed it");
    }
    /// #345: the core's end mode is confirmed only by a `Delivered` of its own sequence. A
    /// late `Delivered` of an earlier `Resistance{0}` (the engine's, from a pause), the same
    /// mode, does not confirm it: the forced end waits `END_MODE_TIMEOUT` and says so. Before,
    /// a `Delivered` was matched by its mode alone.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_stale_delivered_of_an_equal_mode_does_not_confirm_the_end_mode() {
        let (link, hold, takeover, mut driver, events) = gated_link();
        let paused_seq = link.seq.next();
        let paused = TrainerCommand::SetMode {
            mode: END_MODE,
            seq: paused_seq,
        };
        link.control.try_send(paused).unwrap();
        settle().await;
        assert_eq!(driver.try_recv(), Ok(paused));
        takeover.send_replace(Takeover::CutOff);
        // the trainer reports the pause's delivery only once the core's end mode is in
        tokio::spawn(async move {
            while let Some(command) = driver.recv().await {
                if let TrainerCommand::SetMode { mode, .. } = command {
                    let stale = TrainerEvent::Delivered {
                        mode,
                        seq: paused_seq,
                    };
                    let _ = events.send(stale).await;
                }
            }
        });
        let t = tokio::time::Instant::now();
        assert!(
            !force_end_mode("ride-gate", &hold).await,
            "a stale delivery confirmed the end mode"
        );
        assert_eq!(t.elapsed(), END_MODE_TIMEOUT);
        assert_eq!(*hold.delivered.borrow(), Some(paused_seq));
    }
    /// #345 through the core: a forced end of a stalled engine, which then wakes and sends ERG
    /// targets (here through its own sender, as it would) and goes on to its own end. None of
    /// it reaches the trainer: each is dropped, and the core's end mode, then its disconnect,
    /// are the trainer's last commands.
    #[tokio::test(start_paused = true)]
    async fn an_engine_waking_after_a_forced_end_does_not_reach_the_trainer() {
        let dir = tempfile::tempdir().unwrap();
        let mut radio = fake(vec![], false);
        radio.mode_delay = Duration::from_millis(300);
        let core = riding(&radio, dir.path()).await;
        let (engine_control, takeover, hold) = {
            let st = core.0.state.lock().await;
            let s = st.session.as_ref().unwrap();
            (
                s.engine_control.clone(),
                s.takeover.subscribe(),
                s.trainer.clone(),
            )
        };
        stall_engine(&core);
        let end = {
            let core = core.clone();
            tokio::spawn(async move { core.ride_end_outcome().await })
        };
        let mut cut_off = takeover.clone();
        cut_off.wait_for(|t| *t != Takeover::Engine).await.unwrap();
        // the end mode is on its way (the fake takes 300 ms): the engine wakes
        let woken: Vec<_> = [92, 109, 0]
            .into_iter()
            .map(|target_w| TrainerCommand::SetMode {
                mode: ControlMode::Erg { target_w },
                seq: hold.seq.next(),
            })
            .collect();
        for command in &woken {
            engine_control.try_send(*command).unwrap();
        }
        core.0.stall.release();
        // (the woken engine may still take its `Stop` and finish the ride: that record wins,
        // as in `a_stop_the_engine_takes_after_the_timeout_decides_the_outcome`; the trainer's
        // commands are the same either way)
        end.await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let commands = radio.commands.lock().unwrap().clone();
        for command in &woken {
            assert!(
                !commands.contains(command),
                "{command:?} reached the trainer"
            );
        }
        let dropped = hold.dropped.lock().unwrap().clone();
        assert!(
            woken.iter().all(|c| dropped.contains(c)),
            "dropped: {dropped:?}"
        );
        assert_end_mode_before_disconnect(&radio, Duration::from_millis(300), true);
    }
    /// #345, the surviving mutant of #344: `reap` does not hold the state lock across its
    /// bounded join of the engine. Real time, the engine on its own thread, blocked in a record
    /// write so that the forced end's join waits its whole `ABORT_JOIN_TIMEOUT`: all that time
    /// the lock is free (a status read, a second End, get through).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_state_lock_is_free_while_a_forced_end_joins_the_engine() {
        let dir = tempfile::tempdir().unwrap();
        let radio = fake(vec![], false);
        let (core, loaded, _, _) = a_wedged_ride(dir.path(), &radio, Duration::from_secs(4)).await;
        let end = tokio::spawn({
            let core = core.clone();
            async move { core.ride_end_outcome().await }
        });
        until(Duration::from_secs(3), || {
            core.0.joining.load(Ordering::SeqCst)
        })
        .await;
        let mut probes = 0;
        while core.0.joining.load(Ordering::SeqCst) {
            let t = std::time::Instant::now();
            let lock = tokio::time::timeout(Duration::from_millis(50), core.0.state.lock()).await;
            // the join may have ended meanwhile, and the lock with it been taken by the end
            assert!(
                lock.is_ok() || !core.0.joining.load(Ordering::SeqCst),
                "the state lock is held across the join ({:?})",
                t.elapsed()
            );
            drop(lock);
            probes += 1;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(probes > 0);
        let end = tokio::time::timeout(Duration::from_secs(10), end)
            .await
            .expect("End answers")
            .unwrap()
            .unwrap();
        assert_eq!(end, RideEnd::ForcedEnd(loaded.ride_id));
    }
}
