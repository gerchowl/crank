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
        let mut found = Vec::new();
        // The scan source may never end; the deadline is the normal way out.
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
    /// `DeviceUnavailable` / `PermissionDenied` when the connect fails or exceeds 15 s,
    /// `InvalidInput` for an empty id, `Storage`.
    pub async fn device_pair(
        &self,
        id: String,
        role: DeviceRole,
    ) -> Result<PairedDevice, ApiError> {
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
        let kept: Vec<Device> = devices
            .into_iter()
            .filter(|d| d.kind != kind || d.id == db_id)
            .collect();
        self.sync_pairings(&kept).await?;
        kept.into_iter()
            .find(|d| d.id == db_id)
            .and_then(paired)
            .ok_or_else(|| ApiError::new(ErrorCode::Internal, "pairing vanished"))
    }

    /// Hand the remembered pairings to the BLE central (the caller of
    /// `BleCentral::remember`): a reconnect can then match a re-assigned id by name and
    /// verify serial and model.
    async fn sync_pairings(&self, devices: &[Device]) -> Result<(), ApiError> {
        let pairings: Vec<Pairing> = devices.iter().map(pairing).collect();
        Ok(self.0.backend.remember(&pairings).await?)
    }

    /// Forget the pairing of `role`.
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
