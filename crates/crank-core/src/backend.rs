//! Where devices come from: the host's BLE central, or the deterministic simulator
//! (`--sim`, ADR-0002 §3), so the UI and e2e run without hardware.
use crate::dto::DeviceRole;
use crank_ble::{BleCentral, Pairing, RideGate};
use crank_devices::{
    Clock, DeviceError, DeviceInfo, DeviceRef, DiscoveredDevice, Discovery, DriverKind,
    HeartRateDriver, TrainerDriver,
    sim::{SimConfig, SimRig},
};
use futures_util::{FutureExt, StreamExt, future::BoxFuture, stream, stream::BoxStream};
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;

/// Which devices a [`crate::Core`] drives.
#[derive(Clone, Debug)]
pub enum DeviceMode {
    /// Real peripherals over the host's Bluetooth.
    Ble,
    /// The simulator: one seeded rider with a trainer and a strap.
    Sim(SimConfig),
}

/// What the core needs from a BLE central. [`BleCentral`] is the production
/// implementation; tests inject a recording one.
pub(crate) trait Radio: Send + Sync + 'static {
    /// Post-filtered discovery.
    fn scan(&self) -> BoxStream<'static, DiscoveredDevice>;
    /// Trainer driver with reconnect.
    fn trainer(&self, clock: Arc<dyn Clock>, gate: RideGate) -> Arc<dyn TrainerDriver>;
    /// Strap driver with reconnect.
    fn heart(&self, clock: Arc<dyn Clock>, gate: RideGate) -> Arc<dyn HeartRateDriver>;
    /// A paired device the radio should be able to find again by name and verify.
    fn remember(&self, pairing: Pairing);
    /// A pairing was forgotten.
    fn forget(&self, platform_id: &str);
}
impl Radio for BleCentral {
    fn scan(&self) -> BoxStream<'static, DiscoveredDevice> {
        self.discovery().scan()
    }
    fn trainer(&self, clock: Arc<dyn Clock>, gate: RideGate) -> Arc<dyn TrainerDriver> {
        Arc::new(self.trainer_driver(clock, gate))
    }
    fn heart(&self, clock: Arc<dyn Clock>, gate: RideGate) -> Arc<dyn HeartRateDriver> {
        Arc::new(self.hr_driver(clock, gate))
    }
    fn remember(&self, pairing: Pairing) {
        BleCentral::remember(self, pairing);
    }
    fn forget(&self, platform_id: &str) {
        BleCentral::forget(self, platform_id);
    }
}
/// Acquires the radio on first use.
pub(crate) type RadioFactory =
    Arc<dyn Fn() -> BoxFuture<'static, Result<Arc<dyn Radio>, DeviceError>> + Send + Sync>;

enum Kind {
    Radio {
        cell: OnceCell<Arc<dyn Radio>>,
        factory: RadioFactory,
    },
    Sim(SimRig),
}
pub(crate) struct Backend {
    kind: Kind,
    /// Everything remembered of the paired devices: handed to the radio the moment it
    /// is created (a restarted process starts with an empty adapter memory) and kept
    /// current afterwards.
    pairings: Mutex<Vec<Pairing>>,
}

/// The ids the simulator's scan offers, and the role each can act in.
const SIM_TRAINER: &str = "trainer";
const SIM_STRAP: &str = "strap";

impl Backend {
    pub(crate) fn new(mode: DeviceMode, clock: Arc<dyn Clock>, pairings: Vec<Pairing>) -> Self {
        let kind = match mode {
            DeviceMode::Ble => Kind::Radio {
                cell: OnceCell::new(),
                factory: Arc::new(|| {
                    async {
                        let central: Arc<dyn Radio> = Arc::new(BleCentral::new().await?);
                        Ok(central)
                    }
                    .boxed()
                }),
            },
            DeviceMode::Sim(cfg) => Kind::Sim(SimRig::new(cfg, clock)),
        };
        Self {
            kind,
            pairings: Mutex::new(pairings),
        }
    }
    #[cfg(test)]
    pub(crate) fn with_radio(factory: RadioFactory, pairings: Vec<Pairing>) -> Self {
        Self {
            kind: Kind::Radio {
                cell: OnceCell::new(),
                factory,
            },
            pairings: Mutex::new(pairings),
        }
    }
    pub(crate) fn is_sim(&self) -> bool {
        matches!(self.kind, Kind::Sim(_))
    }
    /// The radio is acquired on first use (that is where macOS asks for Bluetooth) and
    /// is given every stored pairing before anyone uses it.
    async fn radio<'a>(
        cell: &'a OnceCell<Arc<dyn Radio>>,
        factory: &RadioFactory,
        pairings: &Mutex<Vec<Pairing>>,
    ) -> Result<&'a Arc<dyn Radio>, DeviceError> {
        cell.get_or_try_init(|| async {
            let radio = factory().await?;
            for p in pairings.lock().map(|p| p.clone()).unwrap_or_default() {
                radio.remember(p);
            }
            Ok(radio)
        })
        .await
    }
    /// A pairing was made or refreshed.
    pub(crate) fn remember(&self, pairing: Pairing) {
        if let Ok(mut all) = self.pairings.lock() {
            all.retain(|p| p.platform_id != pairing.platform_id);
            all.push(pairing.clone());
        }
        if let Kind::Radio { cell, .. } = &self.kind
            && let Some(radio) = cell.get()
        {
            radio.remember(pairing);
        }
    }
    /// A pairing was forgotten.
    pub(crate) fn forget(&self, platform_id: &str) {
        if let Ok(mut all) = self.pairings.lock() {
            all.retain(|p| p.platform_id != platform_id);
        }
        if let Kind::Radio { cell, .. } = &self.kind
            && let Some(radio) = cell.get()
        {
            radio.forget(platform_id);
        }
    }
    /// The simulator only has one device per role; a real radio cannot know the role of
    /// an id it has not scanned, so its connect is the check.
    pub(crate) fn accepts(&self, id: &str, role: DeviceRole) -> bool {
        match (&self.kind, role) {
            (Kind::Sim(_), DeviceRole::Trainer) => id == SIM_TRAINER,
            (Kind::Sim(_), DeviceRole::HeartRate) => id == SIM_STRAP,
            (Kind::Radio { .. }, _) => true,
        }
    }
    #[expect(
        dead_code,
        reason = "recorded as the ride's telemetry source by B2 (#34)"
    )]
    pub(crate) fn telemetry_source(&self) -> &'static str {
        if self.is_sim() { "sim" } else { "fec" }
    }
    pub(crate) async fn scan(&self) -> Result<BoxStream<'static, DiscoveredDevice>, DeviceError> {
        match &self.kind {
            Kind::Radio { cell, factory } => {
                Ok(Self::radio(cell, factory, &self.pairings).await?.scan())
            }
            Kind::Sim(_) => {
                let device = |id: &str, trainer, heart_rate| DiscoveredDevice {
                    device: DeviceRef(id.into()),
                    info: DeviceInfo {
                        name: Some(format!("crank sim {id}")),
                        ..DeviceInfo::default()
                    },
                    trainer,
                    heart_rate,
                };
                Ok(stream::iter([
                    device(SIM_TRAINER, Some(DriverKind::Sim), false),
                    device(SIM_STRAP, None, true),
                ])
                .boxed())
            }
        }
    }
    pub(crate) async fn trainer(
        &self,
        clock: Arc<dyn Clock>,
        gate: RideGate,
    ) -> Result<Arc<dyn TrainerDriver>, DeviceError> {
        match &self.kind {
            Kind::Radio { cell, factory } => Ok(Self::radio(cell, factory, &self.pairings)
                .await?
                .trainer(clock, gate)),
            Kind::Sim(rig) => Ok(Arc::new(rig.trainer())),
        }
    }
    pub(crate) async fn heart(
        &self,
        clock: Arc<dyn Clock>,
        gate: RideGate,
    ) -> Result<Arc<dyn HeartRateDriver>, DeviceError> {
        match &self.kind {
            Kind::Radio { cell, factory } => Ok(Self::radio(cell, factory, &self.pairings)
                .await?
                .heart(clock, gate)),
            Kind::Sim(rig) => Ok(Arc::new(rig.heart())),
        }
    }
}
