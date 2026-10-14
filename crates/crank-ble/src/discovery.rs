//! Scan classification (ADR-0002 §5). The OS filter is advisory (Linux ignores
//! it), so every advertisement is classified here regardless of platform.
use crank_devices::{DeviceInfo, DeviceRef, DiscoveredDevice, Discovery, DriverKind};
use futures_util::{StreamExt, future::ready, stream::BoxStream};
use std::collections::HashMap;
use uuid::Uuid;

/// FE-C over BLE primary service (ADR-0002 §4).
pub const FEC_SERVICE: Uuid = Uuid::from_u128(0x6e40fec1_b5a3_f393_e0a9_e50e24dcca9e);
/// Fitness Machine Service.
pub const FTMS_SERVICE: Uuid = Uuid::from_u128(0x0000_1826_0000_1000_8000_0080_5f9b_34fb);
/// Cycling Power Service; advertised by trainers but ignored in v1.
pub const CPS_SERVICE: Uuid = Uuid::from_u128(0x0000_1818_0000_1000_8000_0080_5f9b_34fb);
/// Heart Rate Service.
pub const HRS_SERVICE: Uuid = Uuid::from_u128(0x0000_180d_0000_1000_8000_0080_5f9b_34fb);
/// Service filter handed to the OS scan.
pub const SCAN_SERVICES: [Uuid; 4] = [FEC_SERVICE, FTMS_SERVICE, CPS_SERVICE, HRS_SERVICE];

/// One advertisement or service-update observation for a peripheral.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advertisement {
    /// Host-local peripheral identity.
    pub device: DeviceRef,
    /// Advertised local name, when seen.
    pub name: Option<String>,
    /// Advertised service UUIDs.
    pub services: Vec<Uuid>,
}
/// Source of advertisements: btleplug in production, a fake in tests.
pub trait Advertisements: Send + Sync + 'static {
    /// Start scanning; dropping the stream stops the scan.
    fn advertisements(&self) -> BoxStream<'static, Advertisement>;
}

/// FE-C → `FeC`; FTMS without FE-C → `Ftms`; HRS sets `heart_rate`.
/// Cycling-Power-only (or unrelated) peripherals yield `None`.
#[must_use]
pub fn classify(services: &[Uuid]) -> Option<(Option<DriverKind>, bool)> {
    let has = |s| services.contains(&s);
    let trainer = if has(FEC_SERVICE) {
        Some(DriverKind::FeC)
    } else if has(FTMS_SERVICE) {
        Some(DriverKind::Ftms)
    } else {
        None
    };
    let heart_rate = has(HRS_SERVICE);
    (trainer.is_some() || heart_rate).then_some((trainer, heart_rate))
}

/// Post-filtering [`Discovery`] over an [`Advertisements`] source. A device is
/// reported once, and again only if its classification or name changes.
pub struct Scanner<A>(pub A);
impl<A: Advertisements> Discovery for Scanner<A> {
    fn scan(&self) -> BoxStream<'static, DiscoveredDevice> {
        self.0
            .advertisements()
            .scan(HashMap::new(), |seen, ad| {
                let reported = classify(&ad.services).and_then(|(trainer, heart_rate)| {
                    let key = (ad.name.clone(), trainer, heart_rate);
                    (seen.insert(ad.device.0.clone(), key.clone()) != Some(key)).then(|| {
                        DiscoveredDevice {
                            device: ad.device,
                            info: DeviceInfo {
                                name: ad.name,
                                ..DeviceInfo::default()
                            },
                            trainer,
                            heart_rate,
                        }
                    })
                });
                ready(Some(reported))
            })
            .filter_map(ready)
            .boxed()
    }
}
