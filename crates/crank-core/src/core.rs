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
    Clock, DeviceInfo, DeviceRef, HeartRateLink, RiderConfig, TrainerCommand, TrainerLink,
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
    /// The ride was ended and its devices released (the end mode delivered, the link lost
    /// or the engine's own deadline passed).
    Released,
    /// The devices were not released within the cap; the caller exits anyway.
    TimedOut,
    /// Ending the ride failed.
    Failed(ApiError),
}

struct Session {
    handle: EngineHandle,
    /// Core stream id → (the engine's id of it, owning window). Core ids are unique for
    /// the process lifetime; the engine's restart with every ride.
    streams: BTreeMap<StreamId, (StreamId, Option<String>)>,
    /// The engine task: ending it drops the device links.
    task: JoinHandle<()>,
    ride_id: String,
    /// Set once a driver has closed its event stream, i.e. handed the peripheral back
    /// (the same confirmation pairing waits for).
    release: LinkRelease,
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
/// (see [`driver_closed`]).
fn relay_events<E: Send + 'static>(
    mut driver_events: tokio::sync::mpsc::Receiver<E>,
    close_on_gone: bool,
) -> (tokio::sync::mpsc::Receiver<E>, watch::Receiver<bool>) {
    let (relay, events) = tokio::sync::mpsc::channel(256);
    let (closed_tx, closed) = watch::channel(false);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                event = driver_events.recv() => match event {
                    // the driver closed first
                    None => break,
                    Some(event) => {
                        if relay.send(event).await.is_err() {
                            break;
                        }
                    }
                },
                () = relay.closed() => break,
            }
        }
        // The engine is gone (or the driver ended): see the driver close.
        if tokio::time::timeout(
            RELEASE_TIMEOUT,
            driver_closed(&mut driver_events, close_on_gone),
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
/// Resolve once the driver has dropped its sender; with `close` the channel is closed towards
/// it first so that a strap's driver, which has no control channel, learns that its consumer
/// is gone (see [`crank_devices::release_events`]).
async fn driver_closed<E>(events: &mut tokio::sync::mpsc::Receiver<E>, close: bool) {
    if close {
        crank_devices::release_events(events).await;
    } else {
        while events.recv().await.is_some() {}
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
    gate: RideGate,
    /// Mirrors the gate: the engine's ride-active signal, awaitable.
    active: watch::Sender<bool>,
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
    discard_hook: DiscardHook,
    /// While set, the engine's callback ignores "inactive": an engine task that died with
    /// the ride-active signal up (#290).
    #[cfg(test)]
    pin_active: Arc<AtomicBool>,
}
/// A rendezvous between a test and `Core::discard`: when enabled, `discard` announces that
/// the lock is released and waits to be resumed.
#[cfg(test)]
#[derive(Default)]
struct DiscardHook {
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
        Self::open_with(&config.data_dir, config.clock, move |pairings, clock| {
            Backend::new(devices, clock, pairings)
        })
    }
    fn open_with(
        data_dir: &std::path::Path,
        clock: Option<Arc<dyn Clock>>,
        backend: impl FnOnce(Vec<Pairing>, Arc<dyn Clock>) -> Backend,
    ) -> Result<Self, ApiError> {
        let store = Arc::new(Store::open_owner(data_dir)?);
        let clock = clock.unwrap_or_else(|| Arc::new(MonotonicClock::new()));
        // A restarted process starts with an empty adapter memory: the stored pairings
        // are handed to the radio as soon as it exists (ADR-0002 §5).
        let pairings = store.devices()?.iter().map(pairing).collect();
        Self::assemble(store, backend(pairings, clock.clone()), clock)
    }
    fn assemble(
        store: Arc<Store>,
        backend: Backend,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ApiError> {
        let clock_for_idle = clock.clone();
        Ok(Self(Arc::new(Inner {
            store,
            backend,
            clock,
            gate: RideGate::new(false),
            active: watch::channel(false).0,
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
            discard_hook: DiscardHook::default(),
            #[cfg(test)]
            pin_active: Arc::default(),
        })))
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
    /// `Conflict` while a ride is loaded, `DeviceUnavailable` / `PermissionDenied` when the connect fails or exceeds 15 s,
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
    /// The engine task is awaited under the state lock, so never unboundedly: with `force`
    /// (the engine did not release the ride in time) it is aborted first, otherwise it gets
    /// `RELEASE_TIMEOUT` to end after its handle is gone and is aborted after that.
    async fn reap(&self, ride_id: &str, force: bool) {
        let mut st = self.0.state.lock().await;
        if st.session.as_ref().is_some_and(|s| s.ride_id == ride_id)
            && let Some(mut session) = st.session.take()
        {
            let status = session.handle.status();
            st.last = (status.state == RideState::Finished)
                .then(|| (session.ride_id.clone(), StreamStatus::from(&status)));
            st.last_release = Some(session.release.clone());
            drop(session.handle);
            if force {
                session.task.abort();
            }
            if tokio::time::timeout(RELEASE_TIMEOUT, &mut session.task)
                .await
                .is_err()
            {
                tracing::warn!(ride_id, "the engine task did not end: aborting it");
                session.task.abort();
                let _ = session.task.await;
            }
        }
        drop(st);
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
    /// `DeviceUnavailable` without a paired or reachable trainer.
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
                        let _ = s.handle.send(EngineCmd::Stop).await;
                        Some((s.ride_id.clone(), s.release.clone()))
                    }
                }
            };
            match ending {
                Some((id, release)) => {
                    self.settle(&id).await;
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
        {
            let mut st = self.0.state.lock().await;
            let _ = session.handle.send(EngineCmd::Stop).await;
            st.session = Some(session);
        }
        #[cfg(test)]
        if self.0.discard_hook.enabled.load(Ordering::SeqCst) {
            self.0.discard_hook.reached.notify_one();
            self.0.discard_hook.resume.notified().await;
        }
        self.spawn_reaper(&ride_id);
        self.settle(&ride_id).await;
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
            self.0.gate.set_active(false);
            self.0.active.send_replace(false);
        }
    }

    /// Connect the devices, build the engine and load the workout into it.
    async fn build_session(
        &self,
        workout: Workout,
        compiled: CompiledWorkout,
    ) -> Result<(Session, bool), ApiError> {
        let (mut trainer, hr, [trainer_device_id, hr_device_id]) = self.connect_links().await?;
        // The engine owns the links and drops them when it ends, so the core watches the
        // drivers' side of the event streams through relays (see `relay_events`): a
        // closed stream is the driver's confirmation that the peripheral is handed back.
        let (events, trainer_closed) = relay_events(
            std::mem::replace(&mut trainer.events, tokio::sync::mpsc::channel(1).1),
            false,
        );
        trainer.events = events;
        let mut hr = hr;
        let hr_closed = hr.as_mut().map(|link| {
            let (events, closed) = relay_events(
                std::mem::replace(&mut link.events, tokio::sync::mpsc::channel(1).1),
                true,
            );
            link.events = events;
            closed
        });
        let hr_linked = hr.is_some();
        let (gate, active) = (self.0.gate.clone(), self.0.active.clone());
        #[cfg(test)]
        let pin = self.0.pin_active.clone();
        let (engine, handle) = Engine::new(
            self.0.clock.clone(),
            Some(trainer),
            hr,
            Arc::new(move |on: bool| {
                #[cfg(test)]
                if !on && pin.load(Ordering::SeqCst) {
                    return;
                }
                gate.set_active(on);
                active.send_replace(on);
            }),
        );
        let task = tokio::spawn(engine.run());
        let ride_id = format!(
            "ride-{:x}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
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
        handle
            .send(EngineCmd::Load(RideSetup {
                blocks: compiled.blocks,
                recorder: Box::new(recorder),
                // The health cap is derived from the athlete profile (#244).
                cap_w: None,
            }))
            .await?;
        Ok((
            Session {
                handle,
                streams: BTreeMap::new(),
                task,
                ride_id,
                release: LinkRelease {
                    trainer: trainer_closed,
                    hr: hr_closed,
                },
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
        Ok(self.engine().await?.send(cmd).await?)
    }
    /// Start the loaded ride.
    ///
    /// # Errors
    /// `Conflict` without a loaded ride or in the wrong state; `DeviceUnavailable` while
    /// the trainer link is not ready; `Storage` if the record cannot be opened.
    pub async fn ride_start(&self) -> Result<(), ApiError> {
        self.ensure_start_allowed()?;
        self.ride_cmd(EngineCmd::Start).await
    }
    /// Pause the ride (the rider's pause).
    ///
    /// # Errors
    /// `Conflict` without a ride in progress.
    pub async fn ride_pause(&self) -> Result<(), ApiError> {
        self.ride_cmd(EngineCmd::Pause).await
    }
    /// End the rider's pause.
    ///
    /// # Errors
    /// `Conflict` unless paused by the rider.
    pub async fn ride_resume(&self) -> Result<(), ApiError> {
        self.ride_cmd(EngineCmd::Resume).await
    }
    /// Jump to the next block now.
    ///
    /// # Errors
    /// `Conflict` without a ride in progress.
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
        self.end_ride(true).await
    }
    /// `ride_end`; without `release` it returns once the engine has ended (the end mode
    /// delivered, the link lost or its deadline passed) and does not wait for the devices'
    /// drivers to hand their peripherals back. That is for a process that is exiting, which
    /// releases the radio anyway and must not spend its quit cap on `RELEASE_TIMEOUT`.
    async fn end_ride(&self, release: bool) -> Result<Option<String>, ApiError> {
        let (id, finishes, link_release) = {
            let st = self.0.state.lock().await;
            let Some(s) = &st.session else {
                // Already reaped, possibly by an end that is still waiting for the devices:
                // this call waits for the same release.
                let ended = st
                    .last
                    .as_ref()
                    .map(|(id, _)| Some(id.clone()))
                    .ok_or_else(|| ApiError::new(ErrorCode::Conflict, "no ride loaded"));
                let pending = st.last_release.clone().filter(|_| release);
                drop(st);
                if let Some(pending) = pending {
                    pending.wait().await;
                }
                return ended;
            };
            let state = s.handle.status().state;
            if state != RideState::Finished {
                match s.handle.send(EngineCmd::Stop).await {
                    // `WrongState`: the ride is already ending (it finished on its own, or
                    // an overlapping end got in first, also on an unstarted ride). That is
                    // not an error, and the waits below must still run.
                    Ok(()) | Err(CommandError::WrongState) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            (
                s.ride_id.clone(),
                state != RideState::Idle,
                s.release.clone(),
            )
        };
        self.settle(&id).await;
        if release {
            link_release.wait().await;
        }
        Ok(finishes.then_some(id))
    }
    /// The application is quitting: end the loaded ride like `ride_end` and wait, for at
    /// most `cap`, until its devices are released.
    ///
    /// Quitting is a ride end (ADR-0002 §7.9): the engine puts the trainer into its end
    /// mode (`Resistance{0}`) and holds the ride-active gate until that was delivered, the
    /// link was lost or its own 2 s deadline passed. A process that exits without waiting
    /// can leave ERG torque on the trainer (S1), and on macOS exiting does not drop `Core`.
    /// The cap bounds a trainer that never answers; it should exceed the engine's 2 s.
    pub async fn shutdown(&self, cap: Duration) -> Shutdown {
        match tokio::time::timeout(cap, self.ride_end()).await {
            Err(_) => Shutdown::TimedOut,
            Ok(Ok(_)) => Shutdown::Released,
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
    /// `Conflict` without a loaded ride, or with 16 streams open.
    pub async fn open_stream(
        &self,
        window: Option<String>,
        sink: impl FrameSink,
    ) -> Result<StreamId, ApiError> {
        let mut st = self.0.state.lock().await;
        let session = st
            .session
            .as_mut()
            .ok_or_else(|| ApiError::new(ErrorCode::Conflict, "no ride loaded"))?;
        let (engine_id, mut frames) = session.handle.open_stream(window.clone()).await?;
        let id = loop {
            let n = self.0.next_stream.fetch_add(1, Ordering::Relaxed);
            if n != 0 {
                break StreamId(n);
            }
        };
        session.streams.insert(id, (engine_id, window));
        drop(st);
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
    /// subscriptions, `InvalidInput` for a rate or filter out of range.
    pub async fn subscribe(
        &self,
        window: Option<&str>,
        stream: StreamId,
        metric: MetricId,
        opts: SubscribeOpts,
    ) -> Result<SubId, ApiError> {
        let (engine, id) = self.owned(window, stream).await?;
        Ok(engine.stream_subscribe(id, metric, opts).await?)
    }
    /// End a subscription of a stream of `window`.
    ///
    /// # Errors
    /// `NotFound`.
    pub async fn unsubscribe(
        &self,
        window: Option<&str>,
        stream: StreamId,
        sub: SubId,
    ) -> Result<(), ApiError> {
        let (engine, id) = self.owned(window, stream).await?;
        Ok(engine.stream_unsubscribe(id, sub).await?)
    }
    /// The window has applied every frame up to `seq`.
    ///
    /// # Errors
    /// `NotFound` for a stream `window` does not own.
    pub async fn stream_ack(
        &self,
        window: Option<&str>,
        stream: StreamId,
        seq: u32,
    ) -> Result<(), ApiError> {
        let (engine, id) = self.owned(window, stream).await?;
        Ok(engine.stream_ack(id, seq).await?)
    }
    /// Latest sample of every subscription, the status and the links.
    ///
    /// # Errors
    /// `NotFound` for a stream `window` does not own.
    pub async fn stream_snapshot(
        &self,
        window: Option<&str>,
        stream: StreamId,
    ) -> Result<StreamSnapshot, ApiError> {
        let (engine, id) = self.owned(window, stream).await?;
        Ok(engine.stream_snapshot(id).await?)
    }
    /// A window was destroyed: drop its streams.
    pub async fn close_window(&self, window: String) {
        let mut st = self.0.state.lock().await;
        if let Some(session) = st.session.as_mut() {
            session
                .streams
                .retain(|_, (_, owner)| owner.as_deref() != Some(window.as_str()));
            let _ = session.handle.close_window(window).await;
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
                        }
                    };
                    while let Some(cmd) = rx.recv().await {
                        if cmd == TrainerCommand::Disconnect {
                            tokio::time::sleep(slow).await;
                            release(&radio);
                        }
                        if matches!(cmd, TrainerCommand::SetMode(_)) {
                            let active = radio
                                .gate
                                .lock()
                                .unwrap()
                                .as_ref()
                                .is_some_and(RideGate::is_active);
                            radio.mode_gate.lock().unwrap().push(active);
                        }
                        log.lock().unwrap().push(cmd);
                        if matches!(cmd, TrainerCommand::SetMode(_)) {
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
        Core::open_with(dir, Some(Arc::new(VirtualClock::new(1))), move |p, _| {
            Backend::with_radio(factory, p)
        })
        .unwrap()
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
                .any(|c| matches!(c, TrainerCommand::SetMode(_)))
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
            commands.last(),
            Some(&TrainerCommand::SetMode(
                crank_devices::ControlMode::Resistance { pct: 0.0 }
            )),
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
    async fn quitting_mid_ride_ends_it_with_the_end_mode_and_releases_after_the_gate() {
        let dir = tempfile::tempdir().unwrap();
        let mut radio = fake(vec![], false);
        radio.mode_delay = Duration::from_millis(500);
        let core = riding(&radio, dir.path()).await;
        assert_eq!(
            core.shutdown(Duration::from_secs(3)).await,
            Shutdown::Released
        );
        // Released before `shutdown` returned, after the gate dropped, and the last thing
        // the trainer was told is the non-torque end mode.
        assert_eq!(*radio.disconnects.lock().unwrap(), [false]);
        let commands = radio.commands.lock().unwrap().clone();
        assert_eq!(
            commands.last(),
            Some(&TrainerCommand::SetMode(
                crank_devices::ControlMode::Resistance { pct: 0.0 }
            )),
            "{commands:?}"
        );
        assert!(!core.ride_status().await.unwrap().loading);
        assert_eq!(
            core.shutdown(Duration::from_secs(3)).await,
            Shutdown::Released
        );
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
        assert_eq!(outcome, Shutdown::Released);
        assert!(t.elapsed() <= Duration::from_secs(3), "{:?}", t.elapsed());
        // ... and a cap below it ends the wait at the cap.
        let dir = tempfile::tempdir().unwrap();
        let core = riding(&radio, dir.path()).await;
        let t = tokio::time::Instant::now();
        assert_eq!(
            core.shutdown(Duration::from_millis(500)).await,
            Shutdown::TimedOut
        );
        assert_eq!(t.elapsed(), Duration::from_millis(500));
    }

    #[tokio::test(start_paused = true)]
    async fn overlapping_ends_of_a_loaded_ride_both_wait_for_the_release() {
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
        // `shutdown` would read as "nothing loaded": it waits for the release like the first.
        let (end, quit) = tokio::join!(core.ride_end(), core.shutdown(Duration::from_secs(3)));
        end.unwrap();
        assert_eq!(quit, Shutdown::Released);
        assert!(!core.ride_status().await.unwrap().status.loaded);
        assert_eq!(*radio.disconnects.lock().unwrap(), [false]);
    }

    #[tokio::test(start_paused = true)]
    async fn quitting_with_nothing_loaded_is_idle() {
        let dir = tempfile::tempdir().unwrap();
        let core = open(dir.path(), &fake(vec![], false));
        assert_eq!(core.shutdown(Duration::from_secs(3)).await, Shutdown::Idle);
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
        let (engine_rx, closed) = relay_events(driver_rx, false);
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
            session.task = tokio::spawn(std::future::pending());
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
}
