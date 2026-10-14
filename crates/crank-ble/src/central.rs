//! The host's BLE central as the factory core needs (ADR-0002 §§5–6, ADR-0001 §2): scan
//! source, and drivers that resolve a remembered platform id to a peripheral on every
//! connect, wrapped in the reconnect supervisors.
//!
//! **One central, one scan.** `crank-core` holds exactly one [`BleCentral`] per process.
//! btleplug keeps one scan per adapter and `ScanStream::drop` stops it, so a second
//! scan would silently end the first. Every scan of a central therefore takes one slot:
//! while it is held, another scan request yields an empty stream (and a reconnect
//! resolve fails with `NotFound`, to be retried by the supervisor's next backoff).
use crate::{
    Advertisement, Advertisements, BtleplugAdvertisements, BtleplugHrTransport, BtleplugTransport,
    FecTrainer, HrsDriver, Pairing, Reconnecting, ReconnectingHr, RideGate, Scanner, classify,
};
use btleplug::{
    api::Manager as _,
    platform::{Adapter, Manager, Peripheral},
};
use crank_devices::{
    Clock, DeviceError, DeviceInfo, DeviceRef, DiscoveredDevice, DriverKind, HeartRateDriver,
    HeartRateLink, RiderConfig, TrainerCommand, TrainerDriver, TrainerLink,
};
use futures_util::{
    FutureExt, StreamExt,
    future::BoxFuture,
    stream::{self, BoxStream},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

/// How long a reconnect may scan for a peripheral the adapter no longer lists. It fits
/// inside the supervisor's steady 5 s backoff (ADR-0002 §6, #206) so attempts do not
/// pile up.
pub const RESOLVE_SCAN: Duration = Duration::from_secs(4);

/// Map a btleplug adapter or manager error. On macOS `Manager::new` cannot fail and the
/// OS's *unauthorized* state surfaces from `adapters()` (btleplug `Adapter::new`), so
/// both calls go through here.
#[must_use]
pub fn classify_adapter_error(error: &btleplug::Error) -> DeviceError {
    match error {
        btleplug::Error::PermissionDenied => DeviceError::PermissionDenied,
        _ => DeviceError::AdapterUnavailable,
    }
}

/// A scan source that can also turn an id into a connectable peripheral.
trait Source: Advertisements {
    type Peripheral: Send + 'static;
    /// The peripheral if the adapter currently lists it.
    fn find(&self, id: DeviceRef) -> BoxFuture<'static, Result<Self::Peripheral, DeviceError>>;
}

/// Resolve `dev` to a peripheral: the stored id first (listed by the adapter only while
/// it is connected or was seen by a running scan), else a bounded scan until the id is
/// seen or an advertisement matches the remembered `pairing` by name (ADR-0002 §5).
async fn resolve<S: Source>(
    source: &S,
    dev: &DeviceRef,
    pairing: Option<&Pairing>,
    scan_for: Duration,
) -> Result<S::Peripheral, DeviceError> {
    match source.find(dev.clone()).await {
        Err(DeviceError::NotFound) => {}
        other => return other,
    }
    let mut ads = source.advertisements();
    let seen = tokio::time::timeout(scan_for, async {
        while let Some(ad) = ads.next().await {
            let Some((trainer, heart_rate)) = classify(&ad.services) else {
                continue;
            };
            let found = DiscoveredDevice {
                device: ad.device.clone(),
                info: DeviceInfo {
                    name: ad.name,
                    ..DeviceInfo::default()
                },
                trainer,
                heart_rate,
            };
            if found.device == *dev || pairing.is_some_and(|p| p.matches(&found)) {
                return Some(found.device);
            }
        }
        None
    })
    .await;
    // Dropping `ads` ends the scan before connecting.
    drop(ads);
    match seen {
        Ok(Some(device)) => source.find(device).await,
        _ => Err(DeviceError::NotFound),
    }
}

/// Wraps a source so that at most one scan runs at a time.
#[derive(Clone)]
struct OneScan<A> {
    inner: A,
    slot: Arc<Mutex<bool>>,
}
struct Slot(Arc<Mutex<bool>>);
impl Drop for Slot {
    fn drop(&mut self) {
        if let Ok(mut busy) = self.0.lock() {
            *busy = false;
        }
    }
}
impl<A: Advertisements> Advertisements for OneScan<A> {
    fn advertisements(&self) -> BoxStream<'static, Advertisement> {
        let acquired = self.slot.lock().is_ok_and(|mut busy| {
            let free = !*busy;
            *busy = true;
            free
        });
        if !acquired {
            return stream::empty().boxed();
        }
        let guard = Slot(self.slot.clone());
        self.inner
            .advertisements()
            .map(move |ad| {
                let _held = &guard;
                ad
            })
            .boxed()
    }
}
impl Source for OneScan<BtleplugAdvertisements> {
    type Peripheral = Peripheral;
    fn find(&self, id: DeviceRef) -> BoxFuture<'static, Result<Peripheral, DeviceError>> {
        let inner = self.inner.clone();
        async move { inner.peripheral(&id).await }.boxed()
    }
}

/// The first BLE adapter of the host. Acquiring it is where macOS raises the Bluetooth
/// permission prompt, so core does it on first use, not at start-up. Core holds exactly
/// one per process and does not forget a device during a ride.
#[derive(Clone)]
pub struct BleCentral {
    adapter: Adapter,
    source: OneScan<BtleplugAdvertisements>,
    pairings: Arc<Pairings>,
}

/// What is remembered of the paired devices, by platform id.
#[derive(Default)]
struct Pairings(Mutex<HashMap<String, Pairing>>);
impl Pairings {
    fn remember(&self, pairing: Pairing) {
        if let Ok(mut all) = self.0.lock() {
            all.insert(pairing.platform_id.clone(), pairing);
        }
    }
    fn forget(&self, platform_id: &str) {
        if let Ok(mut all) = self.0.lock() {
            all.remove(platform_id);
        }
    }
    fn get(&self, platform_id: &str) -> Option<Pairing> {
        self.0.lock().ok()?.get(platform_id).cloned()
    }
}
impl BleCentral {
    /// Acquire the adapter.
    ///
    /// # Errors
    /// `PermissionDenied` when the OS refuses Bluetooth, `AdapterUnavailable` for any
    /// other failure or without an adapter.
    pub async fn new() -> Result<Self, DeviceError> {
        let manager = Manager::new()
            .await
            .map_err(|e| classify_adapter_error(&e))?;
        let adapter = manager
            .adapters()
            .await
            .map_err(|e| classify_adapter_error(&e))?
            .into_iter()
            .next()
            .ok_or(DeviceError::AdapterUnavailable)?;
        Ok(Self {
            source: OneScan {
                inner: BtleplugAdvertisements::new(adapter.clone()),
                slot: Arc::default(),
            },
            adapter,
            pairings: Arc::default(),
        })
    }
    /// Remember what is known of a paired peripheral so a reconnect can find it by name
    /// when its platform id is no longer listed, and verify serial and model after
    /// connecting (ADR-0002 §5).
    pub fn remember(&self, pairing: Pairing) {
        self.pairings.remember(pairing);
    }
    /// Drop what [`BleCentral::remember`] stored for a platform id (the pairing was
    /// forgotten): it no longer matches by name nor verifies serial and model.
    pub fn forget(&self, platform_id: &str) {
        self.pairings.forget(platform_id);
    }
    fn pairing(&self, dev: &DeviceRef) -> Option<Pairing> {
        self.pairings.get(&dev.0)
    }
    /// Post-filtered discovery over this adapter (one scan at a time).
    #[must_use]
    pub fn discovery(&self) -> Scanner<impl Advertisements> {
        Scanner(self.source.clone())
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
            let pairing = central.pairing(&dev);
            let peripheral = resolve(&central.source, &dev, pairing.as_ref(), RESOLVE_SCAN).await?;
            let transport = BtleplugTransport::new(peripheral, central.adapter.clone());
            let link = FecTrainer::new(Arc::new(transport), clock)
                .connect(&dev, rider)
                .await?;
            if pairing.is_some_and(|p| !p.verify(&link.info)) {
                let _ = link.control.send(TrainerCommand::Disconnect).await;
                return Err(DeviceError::NotFound);
            }
            Ok(link)
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
            let pairing = central.pairing(&dev);
            let peripheral = resolve(&central.source, &dev, pairing.as_ref(), RESOLVE_SCAN).await?;
            let transport = BtleplugHrTransport::new(peripheral, central.adapter.clone());
            let link = HrsDriver::new(Arc::new(transport), clock)
                .connect(&dev)
                .await?;
            if pairing.is_some_and(|p| !p.verify(&link.info)) {
                // Dropping the link releases the strap.
                return Err(DeviceError::NotFound);
            }
            Ok(link)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FEC_SERVICE, HRS_SERVICE};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An adapter that lists a peripheral only after a scan has shown it.
    #[derive(Clone, Default)]
    struct Fake {
        listed: Arc<Mutex<Vec<String>>>,
        script: Vec<(&'static str, Option<&'static str>, bool)>,
        scans: Arc<AtomicUsize>,
        never_ends: bool,
    }
    impl Advertisements for Fake {
        fn advertisements(&self) -> BoxStream<'static, Advertisement> {
            self.scans.fetch_add(1, Ordering::SeqCst);
            let listed = self.listed.clone();
            let ads: Vec<_> = self
                .script
                .iter()
                .map(|(id, name, hr)| {
                    (
                        (*id).to_owned(),
                        name.map(str::to_owned),
                        if *hr { HRS_SERVICE } else { FEC_SERVICE },
                    )
                })
                .collect();
            let s = stream::iter(ads).map(move |(id, name, service)| {
                listed.lock().unwrap().push(id.clone());
                Advertisement {
                    device: DeviceRef(id),
                    name,
                    services: vec![service],
                }
            });
            if self.never_ends {
                s.chain(stream::pending()).boxed()
            } else {
                s.boxed()
            }
        }
    }
    impl Source for Fake {
        type Peripheral = String;
        fn find(&self, id: DeviceRef) -> BoxFuture<'static, Result<String, DeviceError>> {
            let found = self.listed.lock().unwrap().contains(&id.0);
            async move { found.then_some(id.0).ok_or(DeviceError::NotFound) }.boxed()
        }
    }
    fn pairing(id: &str, name: &str) -> Pairing {
        Pairing {
            platform_id: id.into(),
            name: Some(name.into()),
            serial: None,
            model: None,
        }
    }
    const SCAN: Duration = Duration::from_secs(4);

    #[test]
    fn forgotten_pairings_no_longer_match() {
        let all = Pairings::default();
        all.remember(pairing("neo", "NEO"));
        assert!(all.get("neo").is_some());
        all.forget("neo");
        assert!(all.get("neo").is_none());
    }

    #[test]
    fn denied_bluetooth_is_permission_denied_not_no_adapter() {
        assert_eq!(
            classify_adapter_error(&btleplug::Error::PermissionDenied),
            DeviceError::PermissionDenied
        );
        for other in [
            btleplug::Error::DeviceNotFound,
            btleplug::Error::NotConnected,
            btleplug::Error::NotSupported("x".into()),
        ] {
            assert_eq!(
                classify_adapter_error(&other),
                DeviceError::AdapterUnavailable
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_listed_peripheral_connects_without_scanning() {
        let fake = Fake::default();
        fake.listed.lock().unwrap().push("neo".into());
        let got = resolve(&fake, &DeviceRef("neo".into()), None, SCAN).await;
        assert_eq!(got.unwrap(), "neo");
        assert_eq!(fake.scans.load(Ordering::SeqCst), 0);
    }
    #[tokio::test(start_paused = true)]
    async fn after_a_drop_the_stored_id_is_found_by_a_bounded_scan() {
        // The adapter forgot the disconnected device: listing fails until a scan runs.
        let fake = Fake {
            script: vec![("other", Some("Wahoo"), false), ("neo", Some("NEO"), false)],
            ..Fake::default()
        };
        let got = resolve(&fake, &DeviceRef("neo".into()), None, SCAN).await;
        assert_eq!(got.unwrap(), "neo");
        assert_eq!(fake.scans.load(Ordering::SeqCst), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn a_reassigned_id_is_matched_by_the_remembered_name() {
        let fake = Fake {
            script: vec![
                ("other", Some("Wahoo"), false),
                ("new-id", Some("NEO"), false),
            ],
            ..Fake::default()
        };
        let p = pairing("old-id", "NEO");
        let got = resolve(&fake, &DeviceRef("old-id".into()), Some(&p), SCAN).await;
        assert_eq!(got.unwrap(), "new-id");
        // Without the remembered pairing the same advertisement does not match.
        let fake = Fake {
            script: vec![("new-id", Some("NEO"), false)],
            ..Fake::default()
        };
        let got = resolve(&fake, &DeviceRef("old-id".into()), None, SCAN).await;
        assert_eq!(got.unwrap_err(), DeviceError::NotFound);
    }
    #[tokio::test(start_paused = true)]
    async fn the_scan_is_bounded_and_ends_not_found() {
        let fake = Fake {
            script: vec![("other", Some("Wahoo"), false)],
            never_ends: true,
            ..Fake::default()
        };
        let started = tokio::time::Instant::now();
        let got = resolve(&fake, &DeviceRef("neo".into()), None, SCAN).await;
        assert_eq!(got.unwrap_err(), DeviceError::NotFound);
        assert_eq!(started.elapsed(), SCAN);
        assert!(
            RESOLVE_SCAN <= Duration::from_secs(5),
            "fits the steady backoff"
        );
    }
    #[tokio::test(start_paused = true)]
    async fn a_second_scan_does_not_run_while_one_is_active() {
        let one = OneScan {
            inner: Fake {
                script: vec![("neo", Some("NEO"), false)],
                never_ends: true,
                ..Fake::default()
            },
            slot: Arc::default(),
        };
        let mut first = one.advertisements();
        assert!(first.next().await.is_some());
        // The slot is held: the second request is empty, and the first scan is untouched.
        assert!(one.advertisements().next().await.is_none());
        assert_eq!(one.inner.scans.load(Ordering::SeqCst), 1);
        drop(first);
        assert!(one.advertisements().next().await.is_some(), "slot released");
    }
}
