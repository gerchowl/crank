//! The host's BLE central as the factory core needs (ADR-0002 §§5–6, ADR-0001 §2): scan
//! source, and drivers that resolve a remembered platform id to a peripheral on every
//! connect, wrapped in the reconnect supervisors.
use crate::{
    BtleplugAdvertisements, BtleplugHrTransport, BtleplugTransport, FecTrainer, HrsDriver,
    Reconnecting, ReconnectingHr, RideGate, Scanner,
};
use btleplug::{
    api::Manager as _,
    platform::{Adapter, Manager},
};
use crank_devices::{
    Clock, DeviceError, DeviceRef, DriverKind, HeartRateDriver, HeartRateLink, RiderConfig,
    TrainerDriver, TrainerLink,
};
use futures_util::future::BoxFuture;
use std::sync::Arc;

/// The first BLE adapter of the host. Acquiring it is where macOS raises the Bluetooth
/// permission prompt, so core does it on first use, not at start-up.
#[derive(Clone)]
pub struct BleCentral {
    adapter: Adapter,
    advertisements: BtleplugAdvertisements,
}
impl BleCentral {
    /// Acquire the adapter.
    ///
    /// # Errors
    /// `PermissionDenied` when the OS refuses, `AdapterUnavailable` without an adapter.
    pub async fn new() -> Result<Self, DeviceError> {
        let manager = Manager::new().await.map_err(|e| match e {
            btleplug::Error::PermissionDenied => DeviceError::PermissionDenied,
            _ => DeviceError::AdapterUnavailable,
        })?;
        let adapter = manager
            .adapters()
            .await
            .map_err(|_| DeviceError::AdapterUnavailable)?
            .into_iter()
            .next()
            .ok_or(DeviceError::AdapterUnavailable)?;
        Ok(Self {
            advertisements: BtleplugAdvertisements::new(adapter.clone()),
            adapter,
        })
    }
    /// Post-filtered discovery over this adapter.
    #[must_use]
    pub fn discovery(&self) -> Scanner<BtleplugAdvertisements> {
        Scanner(self.advertisements.clone())
    }
    /// FE-C trainer driver with the reconnect loop, retrying while `gate` is active.
    #[must_use]
    pub fn trainer_driver(&self, clock: Arc<dyn Clock>, gate: RideGate) -> impl TrainerDriver {
        Reconnecting::new(
            PeripheralTrainer {
                central: self.clone(),
                clock: clock.clone(),
            },
            clock,
            gate,
        )
    }
    /// Heart-rate strap driver with the reconnect loop, retrying while `gate` is active.
    #[must_use]
    pub fn hr_driver(&self, clock: Arc<dyn Clock>, gate: RideGate) -> impl HeartRateDriver {
        ReconnectingHr::new(
            PeripheralHr {
                central: self.clone(),
                clock: clock.clone(),
            },
            clock,
            gate,
        )
    }
}

struct PeripheralTrainer {
    central: BleCentral,
    clock: Arc<dyn Clock>,
}
impl TrainerDriver for PeripheralTrainer {
    fn kind(&self) -> DriverKind {
        DriverKind::FeC
    }
    fn connect(
        &self,
        dev: &DeviceRef,
        rider: RiderConfig,
    ) -> BoxFuture<'static, Result<TrainerLink, DeviceError>> {
        let (central, clock, dev) = (self.central.clone(), self.clock.clone(), dev.clone());
        Box::pin(async move {
            let peripheral = central.advertisements.peripheral(&dev).await?;
            let transport = BtleplugTransport::new(peripheral, central.adapter.clone());
            FecTrainer::new(Arc::new(transport), clock)
                .connect(&dev, rider)
                .await
        })
    }
}
struct PeripheralHr {
    central: BleCentral,
    clock: Arc<dyn Clock>,
}
impl HeartRateDriver for PeripheralHr {
    fn connect(&self, dev: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>> {
        let (central, clock, dev) = (self.central.clone(), self.clock.clone(), dev.clone());
        Box::pin(async move {
            let peripheral = central.advertisements.peripheral(&dev).await?;
            let transport = BtleplugHrTransport::new(peripheral, central.adapter.clone());
            HrsDriver::new(Arc::new(transport), clock)
                .connect(&dev)
                .await
        })
    }
}
