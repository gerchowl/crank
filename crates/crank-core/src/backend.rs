//! Where devices come from: the host's BLE central, or the deterministic simulator
//! (`--sim`, ADR-0002 §3), so the UI and e2e run without hardware.
use crank_ble::{BleCentral, Pairing, RideGate};
use crank_devices::{
    Clock, DeviceError, DeviceInfo, DeviceRef, DiscoveredDevice, Discovery, DriverKind,
    HeartRateDriver, TrainerDriver,
    sim::{SimConfig, SimRig},
};
use futures_util::{StreamExt, stream, stream::BoxStream};
use std::sync::Arc;
use tokio::sync::OnceCell;

/// Which devices a [`crate::Core`] drives.
#[derive(Clone, Debug)]
pub enum DeviceMode {
    /// Real peripherals over the host's Bluetooth.
    Ble,
    /// The simulator: one seeded rider with a trainer and a strap.
    Sim(SimConfig),
}

pub(crate) enum Backend {
    Ble(OnceCell<BleCentral>),
    Sim(SimRig),
}
impl Backend {
    pub(crate) fn new(mode: DeviceMode, clock: Arc<dyn Clock>) -> Self {
        match mode {
            DeviceMode::Ble => Self::Ble(OnceCell::new()),
            DeviceMode::Sim(cfg) => Self::Sim(SimRig::new(cfg, clock)),
        }
    }
    pub(crate) fn is_sim(&self) -> bool {
        matches!(self, Self::Sim(_))
    }
    /// The central is acquired on first use: that is where macOS asks for Bluetooth.
    async fn central(cell: &OnceCell<BleCentral>) -> Result<&BleCentral, DeviceError> {
        cell.get_or_try_init(BleCentral::new).await
    }
    /// Give the BLE central what is remembered of the paired devices, so a reconnect can
    /// find a peripheral by name and verify its serial (ADR-0002 §5). Nothing for the
    /// simulator.
    pub(crate) async fn remember(&self, pairings: &[Pairing]) -> Result<(), DeviceError> {
        if let Self::Ble(cell) = self {
            let central = Self::central(cell).await?;
            for pairing in pairings {
                central.remember(pairing.clone());
            }
        }
        Ok(())
    }
    #[expect(
        dead_code,
        reason = "recorded as the ride's telemetry source by B2 (#34)"
    )]
    pub(crate) fn telemetry_source(&self) -> &'static str {
        if self.is_sim() { "sim" } else { "fec" }
    }
    pub(crate) async fn scan(&self) -> Result<BoxStream<'static, DiscoveredDevice>, DeviceError> {
        match self {
            Self::Ble(cell) => Ok(Self::central(cell).await?.discovery().scan()),
            Self::Sim(_) => {
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
                    device("trainer", Some(DriverKind::Sim), false),
                    device("strap", None, true),
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
        match self {
            Self::Ble(cell) => Ok(Arc::new(
                Self::central(cell).await?.trainer_driver(clock, gate),
            )),
            Self::Sim(rig) => Ok(Arc::new(rig.trainer())),
        }
    }
    pub(crate) async fn heart(
        &self,
        clock: Arc<dyn Clock>,
        gate: RideGate,
    ) -> Result<Arc<dyn HeartRateDriver>, DeviceError> {
        match self {
            Self::Ble(cell) => Ok(Arc::new(Self::central(cell).await?.hr_driver(clock, gate))),
            Self::Sim(rig) => Ok(Arc::new(rig.heart())),
        }
    }
}
