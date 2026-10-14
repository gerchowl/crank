//! Pairing memory matching (ADR-0002 §5). Persistence is `crank-store`'s `device` table.
use crank_devices::{DeviceInfo, DeviceRef, DiscoveredDevice};

/// What is remembered about a paired peripheral: the platform id (`CoreBluetooth`
/// UUID on macOS, stable per host), advertised name and DIS serial. No BLE bonding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    /// Platform peripheral id; the first thing a reconnect tries.
    pub platform_id: String,
    /// Advertised name.
    pub name: Option<String>,
    /// DIS serial (2A25), known only after a connection.
    pub serial: Option<String>,
    /// DIS model number (2A24), known only after a connection.
    pub model: Option<String>,
}
impl Pairing {
    /// Remember a connected device from its reference and DIS identity.
    #[must_use]
    pub fn from_connection(dev: &DeviceRef, info: &DeviceInfo) -> Self {
        Self {
            platform_id: dev.0.clone(),
            name: info.name.clone(),
            serial: info.serial.clone(),
            model: info.model.clone(),
        }
    }
    /// Reference used to reconnect by stored platform id without scanning.
    #[must_use]
    pub fn device_ref(&self) -> DeviceRef {
        DeviceRef(self.platform_id.clone())
    }
    /// Scan fallback: same platform id, or same advertised name. The serial is
    /// not advertised; confirm it after connecting with [`Self::verify`].
    #[must_use]
    pub fn matches(&self, found: &DiscoveredDevice) -> bool {
        found.device.0 == self.platform_id || self.name.is_some() && self.name == found.info.name
    }
    /// After connecting: false only when both serials (or both models) are
    /// known and differ, i.e. a different unit that merely shares the name.
    #[must_use]
    pub fn verify(&self, info: &DeviceInfo) -> bool {
        let agrees = |stored: &Option<String>, read: &Option<String>| match (stored, read) {
            (Some(stored), Some(read)) => stored == read,
            _ => true,
        };
        agrees(&self.serial, &info.serial) && agrees(&self.model, &info.model)
    }
}
