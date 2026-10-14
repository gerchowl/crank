//! The [`Core`] facade (ADR-0001 §4).
//!
//! **Pairing.** Pairing remembers a peripheral: connect once to read its identity, then
//! release it (BLE allows one central, and nothing holds a device outside a ride).
//! Devices are connected when a ride is loaded (#34 B2); live readings before a ride
//! are #267.
use crate::backend::{Backend, DeviceMode};
use crate::dto::{
    CoreInfo, DeviceLinkInfo, DeviceRole, PairedDevice, ScannedDevice, WorkoutDetail,
    WorkoutSummary,
};
use crate::error::{ApiError, ErrorCode};
use crate::library::Library;
use crank_ble::{Pairing, RideGate};
use crank_devices::{Clock, DeviceInfo, DeviceRef, RiderConfig, TrainerCommand};
use crank_engine::MonotonicClock;
use crank_store::{Device, Store};
use futures_util::StreamExt;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

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

struct Inner {
    store: Arc<Store>,
    backend: Backend,
    clock: Arc<dyn Clock>,
    gate: RideGate,
    library: Library,
}

/// The application API. Cheap to clone; one per process (it owns `ride.lock`).
#[derive(Clone)]
pub struct Core(Arc<Inner>);

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
        Ok(Self(Arc::new(Inner {
            store,
            backend,
            clock,
            gate: RideGate::new(false),
            library: Library::builtin()?,
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

    /// Scan for trainers and straps for `for_ms` (default 5 s, 0.2–15 s), or until the
    /// scan source ends.
    ///
    /// A peripheral is listed once, with the union of what its advertisements showed.
    /// While another scan holds the central the list is empty; a typed busy result comes
    /// with #265 and maps to `Conflict` then.
    ///
    /// # Errors
    /// `PermissionDenied` or `DeviceUnavailable` from the adapter.
    pub async fn devices_scan(&self, for_ms: Option<u32>) -> Result<Vec<ScannedDevice>, ApiError> {
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
    /// Must not run during an active ride: the `Conflict` guard lands with the ride
    /// lifecycle (#34 B2), which `Store::forget_device` and `BleCentral` document as the
    /// caller's duty.
    ///
    /// # Errors
    /// `DeviceUnavailable` / `PermissionDenied` when the connect fails or exceeds 15 s,
    /// `InvalidInput` for an empty id or one that cannot act in `role`, `Storage`.
    pub async fn device_pair(
        &self,
        id: String,
        role: DeviceRole,
    ) -> Result<PairedDevice, ApiError> {
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
    /// Must not run during an active ride (the `Conflict` guard lands in #34 B2).
    ///
    /// # Errors
    /// `NotFound` when nothing is paired for it.
    #[expect(
        clippy::unused_async,
        reason = "every facade method is async (ADR-0001 §4)"
    )]
    pub async fn device_forget(&self, role: DeviceRole) -> Result<(), ApiError> {
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

    /// The pairing and live link of each role.
    ///
    /// # Errors
    /// `Storage`.
    #[expect(
        clippy::unused_async,
        reason = "every facade method is async (ADR-0001 §4)"
    )]
    pub async fn device_links(&self) -> Result<Vec<DeviceLinkInfo>, ApiError> {
        let devices = self.0.store.devices()?;
        Ok([DeviceRole::Trainer, DeviceRole::HeartRate]
            .into_iter()
            .map(|role| DeviceLinkInfo {
                role,
                device: devices
                    .iter()
                    .find(|d| d.kind == role.kind())
                    .cloned()
                    .and_then(paired),
                link: None,
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
            let dev = dev.clone();
            async move {
                let mut link = inner.connect(&dev, rider).await?;
                let (tx, mut rx) = tokio::sync::mpsc::channel(64);
                let real = std::mem::replace(&mut link.control, tx);
                tokio::spawn(async move {
                    while let Some(cmd) = rx.recv().await {
                        log.lock().unwrap().push(cmd);
                        if real.send(cmd).await.is_err() {
                            break;
                        }
                    }
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
        fn trainer(&self, _: Arc<dyn Clock>, _: RideGate) -> Arc<dyn TrainerDriver> {
            Arc::new(Recording(self.clone()))
        }
        fn heart(&self, _: Arc<dyn Clock>, _: RideGate) -> Arc<dyn HeartRateDriver> {
            Arc::new(self.rig.heart())
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
}
