//! The [`Core`] facade (ADR-0001 §4).
//!
//! **Lifecycle.** Pairing remembers a peripheral (connect once to read its identity,
//! then release: BLE allows one central, and nothing holds a device outside a ride).
//! `ride_load` connects the paired trainer (and strap, if any), builds a ride engine
//! over the links and loads the workout; `ride_end` finishes the ride and drops the
//! engine, which releases the devices. Streams belong to the loaded ride's engine and
//! to a window label; they end with the ride.
use crate::backend::{Backend, DeviceMode};
use crate::dto::{
    CoreInfo, DeviceLinkInfo, DeviceRole, PairedDevice, RideLoaded, RideOutcome, RideRecord,
    RideSummary, ScannedDevice, WorkoutDetail, WorkoutSummary,
};
use crate::error::{ApiError, ErrorCode};
use crate::library::Library;
use crate::recorder::{RideMeta, StoreRecorder, unix_now};
use crank_ble::RideGate;
use crank_devices::{Clock, DeviceRef, RiderConfig, TrainerCommand};
use crank_domain::workout::{CompiledWorkout, Severity};
use crank_engine::{
    Engine, EngineCmd, EngineHandle, MetricId, MonotonicClock, RideSetup, StreamFrame, StreamId,
    StreamRideState, StreamSnapshot, StreamStatus, SubId, SubscribeOpts,
};
use crank_store::{Device, Ride, RideState, Store};
use futures_util::StreamExt;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::{sync::Mutex, task::JoinHandle};

/// Longest a connect may take before the command fails, so the UI never waits on a
/// device indefinitely.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a released link may take to close before pairing stops waiting.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(3);
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

struct Session {
    handle: EngineHandle,
    /// The engine task: ending it drops the device links.
    task: JoinHandle<()>,
    ride_id: String,
}
impl Session {
    /// Drop the handle and wait until the engine has ended and released its links, so
    /// the next load can connect at once.
    async fn release(self) {
        drop(self.handle);
        let _ = self.task.await;
    }
}
#[derive(Default)]
struct State {
    session: Option<Session>,
    /// Final status of the last ride that finished, until the next load.
    last: Option<StreamStatus>,
}
struct Inner {
    store: Arc<Store>,
    backend: Backend,
    clock: Arc<dyn Clock>,
    gate: RideGate,
    library: Library,
    state: Mutex<State>,
}

/// The application API. Cheap to clone; one per process (it owns `ride.lock`).
#[derive(Clone)]
pub struct Core(Arc<Inner>);

fn conflict(message: &str) -> ApiError {
    ApiError::new(ErrorCode::Conflict, message)
}
fn idle_status() -> StreamStatus {
    StreamStatus {
        state: StreamRideState::Idle,
        loaded: false,
        session_s: 0.0,
        workout_s: 0.0,
        block_index: None,
        block_elapsed_s: 0.0,
        late_samples: 0,
        record_error: None,
    }
}
#[expect(clippy::cast_precision_loss, reason = "UTC seconds are far below 2^53")]
fn secs(unix: i64) -> f64 {
    unix as f64
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

impl Core {
    /// Open the store as its owner and build the core. Must run inside a tokio runtime
    /// when devices are used.
    ///
    /// # Errors
    /// `Conflict` when another process holds `ride.lock`; `Storage` for any store failure.
    pub fn open(config: CoreConfig) -> Result<Self, ApiError> {
        let store = Arc::new(Store::open_owner(&config.data_dir)?);
        let clock = config
            .clock
            .unwrap_or_else(|| Arc::new(MonotonicClock::new()));
        Ok(Self(Arc::new(Inner {
            store,
            backend: Backend::new(config.devices, clock.clone()),
            clock,
            gate: RideGate::new(false),
            library: Library::builtin()?,
            state: Mutex::default(),
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

    async fn require_no_ride(&self) -> Result<(), ApiError> {
        if self.0.state.lock().await.session.is_some() {
            return Err(conflict("a ride is loaded: end it first"));
        }
        Ok(())
    }

    // ---- devices ----------------------------------------------------------------

    /// Scan for trainers and straps for `for_ms` (default 5 s, 0.2–15 s), or until the
    /// scan source ends.
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
        let mut found = Vec::new();
        let _ = tokio::time::timeout(limit, async {
            while let Some(d) = scan.next().await {
                found.push(ScannedDevice {
                    paired: known.contains(&d.device.0),
                    id: d.device.0,
                    name: d.info.name,
                    trainer: d.trainer.is_some(),
                    heart_rate: d.heart_rate,
                });
            }
        })
        .await;
        Ok(found)
    }

    /// Connect to `id` once to read its identity, remember it for `role` (replacing the
    /// previous pairing of that role) and release it.
    ///
    /// # Errors
    /// `Conflict` while a ride is loaded, `DeviceUnavailable` / `PermissionDenied` when
    /// the connect fails or exceeds 15 s, `Storage`.
    pub async fn device_pair(
        &self,
        id: String,
        role: DeviceRole,
    ) -> Result<PairedDevice, ApiError> {
        self.require_no_ride().await?;
        if id.is_empty() {
            return Err(ApiError::new(ErrorCode::InvalidInput, "empty device id"));
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
        }
        devices
            .into_iter()
            .find(|d| d.id == db_id)
            .and_then(paired)
            .ok_or_else(|| ApiError::new(ErrorCode::Internal, "pairing vanished"))
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
        }
        any.then_some(())
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, "nothing paired for that role"))
    }

    /// The pairing and live link of each role.
    ///
    /// # Errors
    /// `Storage`.
    pub async fn device_links(&self) -> Result<Vec<DeviceLinkInfo>, ApiError> {
        let devices = self.0.store.devices()?;
        let live = match &self.0.state.lock().await.session {
            Some(s) if s.handle.status().state != crank_engine::RideState::Finished => {
                let st = s.handle.status();
                Some((st.trainer_link, st.hr_link))
            }
            _ => None,
        };
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

    // ---- workouts ---------------------------------------------------------------

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
    ) -> Result<
        (
            crank_devices::TrainerLink,
            Option<crank_devices::HeartRateLink>,
            [Option<String>; 2],
        ),
        ApiError,
    > {
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
        if let Some(strap) = find("hr") {
            let driver = self.0.backend.heart(clock, gate).await?;
            match tokio::time::timeout(
                CONNECT_TIMEOUT,
                driver.connect(&DeviceRef(strap.platform_id.clone())),
            )
            .await
            {
                Ok(Ok(l)) => hr = Some(l),
                Ok(Err(e)) => tracing::warn!(%e, "strap unavailable: riding without heart rate"),
                Err(_) => tracing::warn!("strap connect timed out: riding without heart rate"),
            }
        }
        let ids = [
            Some(trainer.id.clone()),
            hr.is_some()
                .then(|| find("hr").map(|d| d.id.clone()))
                .flatten(),
        ];
        Ok((link, hr, ids))
    }

    /// Load a workout (or a free ride for `None`): connect the paired devices, build
    /// the engine and open the ride record's metadata. The ride starts with `ride_start`.
    ///
    /// # Errors
    /// `Conflict` if a ride is in progress; `NotFound` for an unknown workout;
    /// `DeviceUnavailable` without a paired or reachable trainer.
    pub async fn ride_load(&self, workout_id: Option<String>) -> Result<RideLoaded, ApiError> {
        let mut st = self.0.state.lock().await;
        if let Some(s) = &st.session {
            if StreamStatus::from(&s.handle.status()).in_progress() {
                return Err(conflict("a ride is in progress"));
            }
            // Loaded but not started, or finished: replace it. `Stop` unloads a loaded
            // ride and is refused (harmlessly) on a finished one.
            let _ = s.handle.send(EngineCmd::Stop).await;
            if let Some(old) = st.session.take() {
                old.release().await;
            }
        }
        st.last = None;
        let (workout, compiled) = match &workout_id {
            Some(id) => {
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
                (e.workout.clone(), e.compiled.clone())
            }
            None => (
                crank_domain::workout::Workout {
                    id: "free-ride".into(),
                    name: "Free ride".into(),
                    ftp: 0.0,
                    nodes: Vec::new(),
                },
                CompiledWorkout {
                    blocks: Vec::new(),
                    total_s: 0.0,
                    total_range_s: [0.0, 0.0],
                    tss: 0.0,
                    intensity_factor: 0.0,
                    time_in_zone_s: [0; 7],
                    issues: Vec::new(),
                },
            ),
        };
        self.ensure_start_allowed()?;
        let (trainer, hr, [trainer_device_id, hr_device_id]) = self.connect_links().await?;
        let hr_linked = hr.is_some();
        let gate = self.0.gate.clone();
        let (engine, handle) = Engine::new(
            self.0.clock.clone(),
            Some(trainer),
            hr,
            Arc::new(move |active: bool| gate.set_active(active)),
        );
        let task = tokio::spawn(engine.run());
        let ride_id = format!(
            "ride-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
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
            }))
            .await?;
        st.session = Some(Session {
            handle,
            task,
            ride_id: ride_id.clone(),
        });
        Ok(RideLoaded {
            ride_id,
            workout_id,
            hr_linked,
        })
    }

    async fn engine(&self) -> Result<EngineHandle, ApiError> {
        self.0
            .state
            .lock()
            .await
            .session
            .as_ref()
            .map(|s| s.handle.clone())
            .ok_or_else(|| conflict("no ride loaded"))
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
    /// Finish the ride (or unload one that never started) and release the devices.
    /// Returns the id of the finished ride, `None` for an unloaded one.
    ///
    /// # Errors
    /// `Conflict` without a loaded ride.
    pub async fn ride_end(&self) -> Result<Option<String>, ApiError> {
        let mut st = self.0.state.lock().await;
        let s = st
            .session
            .as_ref()
            .ok_or_else(|| conflict("no ride loaded"))?;
        let state = s.handle.status().state;
        let mut status = s.handle.subscribe();
        let ended = match state {
            // The workout ended by itself; the record is closed already.
            crank_engine::RideState::Finished => Some(StreamStatus::from(&s.handle.status())),
            crank_engine::RideState::Riding | crank_engine::RideState::Paused(_) => {
                s.handle.send(EngineCmd::Stop).await?;
                // The engine answers before it publishes the new status.
                let finished = status
                    .wait_for(|s| s.state == crank_engine::RideState::Finished)
                    .await
                    .map_err(|_| ApiError::new(ErrorCode::Internal, "engine stopped"))?;
                Some(StreamStatus::from(&*finished))
            }
            crank_engine::RideState::Idle => {
                s.handle.send(EngineCmd::Stop).await?;
                None
            }
        };
        let ride_id = ended.is_some().then(|| s.ride_id.clone());
        if let Some(session) = st.session.take() {
            session.release().await;
        }
        st.last = ended;
        Ok(ride_id)
    }
    /// The ride's state; durable truth behind `resync` (ADR-0001 §4.3).
    ///
    /// # Errors
    /// Never fails today.
    pub async fn ride_status(&self) -> Result<StreamStatus, ApiError> {
        let st = self.0.state.lock().await;
        Ok(match (&st.session, &st.last) {
            (Some(s), _) => StreamStatus::from(&s.handle.status()),
            (None, Some(last)) => last.clone(),
            (None, None) => idle_status(),
        })
    }

    // ---- streams ----------------------------------------------------------------

    /// Open the window's multiplexed stream on the loaded ride (dropping that window's
    /// previous streams). Frames go to `sink` until the ride ends or `sink` fails.
    ///
    /// # Errors
    /// `Conflict` without a loaded ride, or with 16 streams open.
    pub async fn open_stream(
        &self,
        window: Option<String>,
        sink: impl FrameSink,
    ) -> Result<StreamId, ApiError> {
        let (id, mut frames) = self.engine().await?.open_stream(window).await?;
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
    /// Subscribe to a metric on a stream.
    ///
    /// # Errors
    /// `NotFound` stream, `Conflict` beyond 64 subscriptions, `InvalidInput` for a
    /// rate or filter out of range.
    pub async fn subscribe(
        &self,
        stream: StreamId,
        metric: MetricId,
        opts: SubscribeOpts,
    ) -> Result<SubId, ApiError> {
        Ok(self
            .engine()
            .await?
            .stream_subscribe(stream, metric, opts)
            .await?)
    }
    /// End a subscription.
    ///
    /// # Errors
    /// `NotFound`.
    pub async fn unsubscribe(&self, stream: StreamId, sub: SubId) -> Result<(), ApiError> {
        Ok(self.engine().await?.stream_unsubscribe(stream, sub).await?)
    }
    /// The window has applied every frame up to `seq`.
    ///
    /// # Errors
    /// `Conflict` without a loaded ride.
    pub async fn stream_ack(&self, stream: StreamId, seq: u64) -> Result<(), ApiError> {
        Ok(self.engine().await?.stream_ack(stream, seq).await?)
    }
    /// Latest sample of every subscription, the status and the links.
    ///
    /// # Errors
    /// `NotFound` stream.
    pub async fn stream_snapshot(&self, stream: StreamId) -> Result<StreamSnapshot, ApiError> {
        Ok(self.engine().await?.stream_snapshot(stream).await?)
    }
    /// A window was destroyed: drop its streams.
    pub async fn close_window(&self, window: String) {
        if let Ok(engine) = self.engine().await {
            let _ = engine.close_window(window).await;
        }
    }

    // ---- history ----------------------------------------------------------------

    fn record(ride: Ride) -> RideRecord {
        let summary = ride.summary.and_then(|s| {
            serde_json::from_str::<RideSummary>(crank_store::JsonColumn::as_str(&s)).ok()
        });
        let workout = ride.workout.workout();
        RideRecord {
            id: ride.id,
            started_at: secs(ride.started_at),
            ended_at: ride.ended_at.map(secs),
            outcome: match ride.state {
                RideState::Open => RideOutcome::Open,
                RideState::Finished => RideOutcome::Finished,
                RideState::Aborted => RideOutcome::Aborted,
            },
            workout_id: (workout.id != "free-ride").then(|| workout.id.clone()),
            workout_name: workout.name.clone(),
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
    #[expect(
        clippy::unused_async,
        reason = "every facade method is async (ADR-0001 §4)"
    )]
    pub async fn ride_summary(&self, id: String) -> Result<RideRecord, ApiError> {
        self.0
            .store
            .ride(&id)?
            .map(Self::record)
            .ok_or_else(|| ApiError::new(ErrorCode::NotFound, format!("no ride {id:?}")))
    }
}
