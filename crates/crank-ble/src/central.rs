//! The host's BLE central as the factory core needs (ADR-0002 §§5–6, ADR-0001 §2): scan
//! source, and drivers that resolve a remembered platform id to a peripheral on every
//! connect, wrapped in the reconnect supervisors.
//!
//! **One central, one shared scan.** `crank-core` holds exactly one [`BleCentral`] per
//! process. btleplug keeps one scan per adapter and stopping it ends it for everyone, so
//! all scan requests of a central (the trainer's and the strap's reconnect resolves, and
//! the UI's device scan) share one underlying scan. Each requester gets its own stream;
//! a joiner is first replayed what the running scan has already seen, and the adapter
//! scan stops only when the last requester has dropped (#265). A scan started after a
//! stop waits for that stop to complete, so a late stop never ends a newer scan.
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
use futures_util::{FutureExt, Stream, StreamExt, future::BoxFuture, stream::BoxStream};
use std::{
    collections::{HashMap, VecDeque},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::Duration,
};

/// How long a reconnect may scan for a peripheral the adapter no longer lists.
///
/// The supervisor waits `backoff(n)` and then connects, so while a device stays away the
/// gap between attempts is the backoff plus this scan: about 9 s at the steady 5 s
/// backoff (ADR-0002 §6, #206), not 5 s. 4 s is kept: the trainer and strap advertise
/// several times a second, so a device in range is seen well inside it, and the scan is
/// shared (one adapter scan serves both devices and the UI), so a longer window would
/// only lengthen the gap for an absent device. §6 states the backoff schedule, not a
/// bound on the time to reconnect.
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
    // Dropping `ads` releases this requester's share of the scan; the adapter scan itself
    // stops once the last requester is gone, possibly after we connect (the next scan
    // start waits for that stop).
    drop(ads);
    match seen {
        Ok(Some(device)) => source.find(device).await,
        _ => Err(DeviceError::NotFound),
    }
}

/// Wraps a source so that all concurrent requesters share one underlying scan.
#[derive(Clone)]
struct SharedScan<A> {
    inner: A,
    shared: Arc<Shared>,
}
#[derive(Default)]
struct Shared {
    state: Mutex<ScanState>,
    /// Woken (with a stored permit) after a requester has closed its share, so the pump
    /// re-checks whether any are left.
    wake: tokio::sync::Notify,
}
#[derive(Default)]
struct ScanState {
    /// A pump task owns the inner scan.
    running: bool,
    requesters: Vec<Arc<Mutex<Inbox>>>,
    /// Latest advertisement per device of the running scan, replayed to joiners.
    seen: Vec<Advertisement>,
}
/// One requester's pending advertisements, coalesced per device: a requester that polls
/// slowly sees every device the scan saw, each with its latest advertisement, and memory
/// is bounded by the number of distinct devices, not of advertisements.
#[derive(Default)]
struct Inbox {
    pending: VecDeque<Advertisement>,
    waker: Option<Waker>,
    /// The scan ended; no more advertisements follow.
    ended: bool,
    /// The requester dropped its share.
    closed: bool,
}
impl Inbox {
    fn push(&mut self, ad: Advertisement) {
        match self.pending.iter_mut().find(|p| p.device == ad.device) {
            Some(slot) => *slot = ad,
            None => self.pending.push_back(ad),
        }
        self.wake();
    }
    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
/// One requester's view of the shared scan; dropping it releases its share.
struct Share {
    inbox: Arc<Mutex<Inbox>>,
    shared: Arc<Shared>,
}
impl Stream for Share {
    type Item = Advertisement;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Advertisement>> {
        let mut inbox = lock(&self.inbox);
        if let Some(ad) = inbox.pending.pop_front() {
            Poll::Ready(Some(ad))
        } else if inbox.ended {
            Poll::Ready(None)
        } else {
            inbox.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
impl Drop for Share {
    fn drop(&mut self) {
        // Closed before the pump is woken: the pump's check must see it. `Notify` stores
        // the permit, so a wake that arrives while the pump is busy is not lost.
        lock(&self.inbox).closed = true;
        self.shared.wake.notify_one();
    }
}
/// Owns the inner scan while it has requesters: fans advertisements out, and ends the scan
/// when the last requester is gone. The decision to end it is taken under the state lock,
/// so a requester arriving first joins this scan and one arriving after starts a new one.
async fn pump<A: Advertisements>(inner: A, shared: Arc<Shared>) {
    // Belt and braces: the real guard is the adapter source's own scan lock
    // (`BtleplugAdvertisements`), which serialises start after stop. This also keeps any
    // source honest that reports `stopped`, and is what the fake-seam tests exercise.
    inner.stopped().await;
    {
        let mut state = lock(&shared.state);
        state.requesters.retain(|r| !lock(r).closed);
        if state.requesters.is_empty() {
            state.running = false;
            return;
        }
    }
    let mut ads = inner.advertisements();
    loop {
        tokio::select! {
            () = shared.wake.notified() => {
                let mut state = lock(&shared.state);
                state.requesters.retain(|r| !lock(r).closed);
                if state.requesters.is_empty() {
                    // Dropped under the lock: stopping is pending before a newcomer can
                    // start (and then wait for `stopped`).
                    drop(ads);
                    state.running = false;
                    state.seen.clear();
                    return;
                }
            }
            ad = ads.next() => {
                let mut state = lock(&shared.state);
                let Some(ad) = ad else {
                    for r in state.requesters.drain(..) {
                        let mut inbox = lock(&r);
                        inbox.ended = true;
                        inbox.wake();
                    }
                    state.seen.clear();
                    state.running = false;
                    return;
                };
                state.seen.retain(|s| s.device != ad.device);
                state.seen.push(ad.clone());
                state.requesters.retain(|r| {
                    let mut inbox = lock(r);
                    if inbox.closed {
                        return false;
                    }
                    inbox.push(ad.clone());
                    true
                });
            }
        }
    }
}
impl<A: Advertisements + Clone> Advertisements for SharedScan<A> {
    fn advertisements(&self) -> BoxStream<'static, Advertisement> {
        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let mut state = lock(&self.shared.state);
        // A late joiner receives every device seen so far.
        lock(&inbox).pending.extend(state.seen.iter().cloned());
        state.requesters.push(inbox.clone());
        if !state.running {
            state.running = true;
            tokio::spawn(pump(self.inner.clone(), self.shared.clone()));
        }
        Box::pin(Share {
            inbox,
            shared: self.shared.clone(),
        })
    }
    fn stopped(&self) -> BoxFuture<'static, ()> {
        self.inner.stopped()
    }
}
impl Source for SharedScan<BtleplugAdvertisements> {
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
    source: SharedScan<BtleplugAdvertisements>,
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
            source: SharedScan {
                inner: BtleplugAdvertisements::new(adapter.clone()),
                shared: Arc::default(),
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
    /// Post-filtered discovery over this adapter (shares the central's one scan with reconnects).
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
    use futures_util::stream;
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
    }
    /// A [`Fake`] that models btleplug's late stop: a dropped scan stops the adapter only
    /// after `stop_delay`, and a stop ends whatever scan runs then.
    #[derive(Clone)]
    struct Adapter {
        fake: Fake,
        scanning: Arc<AtomicUsize>,
        stops: Arc<AtomicUsize>,
        pending: Arc<AtomicUsize>,
        stop_delay: Duration,
        /// The first `find` misses, as when the adapter forgot the device after a drop.
        first_find_misses: Arc<std::sync::atomic::AtomicBool>,
    }
    struct Stopping(Adapter);
    impl Drop for Stopping {
        fn drop(&mut self) {
            let a = self.0.clone();
            a.pending.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                tokio::time::sleep(a.stop_delay).await;
                a.scanning.store(0, Ordering::SeqCst);
                a.stops.fetch_add(1, Ordering::SeqCst);
                a.pending.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }
    impl Adapter {
        fn new(script: Vec<(&'static str, Option<&'static str>, bool)>) -> Self {
            Self {
                fake: Fake {
                    script,
                    never_ends: true,
                    ..Fake::default()
                },
                scanning: Arc::default(),
                stops: Arc::default(),
                pending: Arc::default(),
                stop_delay: Duration::from_millis(500),
                first_find_misses: Arc::default(),
            }
        }
    }
    impl Advertisements for Adapter {
        fn advertisements(&self) -> BoxStream<'static, Advertisement> {
            self.scanning.store(1, Ordering::SeqCst);
            let guard = Stopping(self.clone());
            self.fake
                .advertisements()
                .map(move |ad| {
                    let _held = &guard;
                    ad
                })
                .boxed()
        }
        fn stopped(&self) -> BoxFuture<'static, ()> {
            let pending = self.pending.clone();
            async move {
                while pending.load(Ordering::SeqCst) > 0 {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
            .boxed()
        }
    }
    impl Source for Adapter {
        type Peripheral = String;
        fn find(&self, id: DeviceRef) -> BoxFuture<'static, Result<String, DeviceError>> {
            if self.first_find_misses.swap(false, Ordering::SeqCst) {
                return async { Err(DeviceError::NotFound) }.boxed();
            }
            self.fake.find(id)
        }
    }
    fn shared(adapter: &Adapter) -> SharedScan<Adapter> {
        SharedScan {
            inner: adapter.clone(),
            shared: Arc::default(),
        }
    }
    impl Source for SharedScan<Adapter> {
        type Peripheral = String;
        fn find(&self, id: DeviceRef) -> BoxFuture<'static, Result<String, DeviceError>> {
            self.inner.find(id)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn trainer_and_strap_resolve_together_and_both_find_their_device() {
        let adapter = Adapter::new(vec![("neo", Some("NEO"), false), ("hrm", Some("HR"), true)]);
        let source = shared(&adapter);
        let (neo_ref, hrm_ref) = (DeviceRef("neo".into()), DeviceRef("hrm".into()));
        let (neo, hrm) = tokio::join!(
            resolve(&source, &neo_ref, None, SCAN),
            resolve(&source, &hrm_ref, None, SCAN),
        );
        assert_eq!(neo.unwrap(), "neo");
        assert_eq!(hrm.unwrap(), "hrm");
        assert_eq!(adapter.fake.scans.load(Ordering::SeqCst), 1, "one scan");
    }
    #[tokio::test(start_paused = true)]
    async fn a_ui_scan_during_a_reconnect_resolve_gets_results_too() {
        let adapter = Adapter::new(vec![("neo", Some("NEO"), false)]);
        adapter.first_find_misses.store(true, Ordering::SeqCst);
        let source = shared(&adapter);
        let mut ui = source.advertisements();
        assert_eq!(ui.next().await.unwrap().device, DeviceRef("neo".into()));
        // The adapter does not list it (first `find` misses) and the running scan will not
        // advertise it again: the resolve joins the scan and only the replay can deliver.
        let got = resolve(&source, &DeviceRef("neo".into()), None, SCAN).await;
        assert_eq!(got.unwrap(), "neo");
        assert_eq!(
            adapter.fake.scans.load(Ordering::SeqCst),
            1,
            "joined, not restarted"
        );
        // The resolve finishing leaves the UI's scan running.
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(adapter.stops.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.scanning.load(Ordering::SeqCst), 1);
        drop(ui);
    }
    #[tokio::test(start_paused = true)]
    async fn a_burst_delivers_every_device_to_slow_and_late_requesters() {
        const IDS: [&str; 10] = ["d0", "d1", "d2", "d3", "d4", "d5", "d6", "d7", "d8", "d9"];
        let script = (0..500)
            .map(|i| (IDS[i % IDS.len()], Some("X"), false))
            .collect();
        let adapter = Adapter::new(script);
        let source = shared(&adapter);
        let (mut first, mut second) = (source.advertisements(), source.advertisements());
        // Nobody polls during the burst; the pump has fanned all 500 out by now.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut late = source.advertisements();
        for share in [&mut first, &mut second, &mut late] {
            let mut devices = std::collections::HashSet::new();
            while let Ok(Some(ad)) =
                tokio::time::timeout(Duration::from_millis(10), share.next()).await
            {
                devices.insert(ad.device.0);
            }
            assert_eq!(devices.len(), IDS.len(), "no device is lost");
        }
    }
    #[tokio::test(start_paused = true)]
    async fn the_last_requester_dropping_stops_the_scan_exactly_once() {
        let adapter = Adapter::new(vec![("neo", Some("NEO"), false)]);
        let source = shared(&adapter);
        let mut first = source.advertisements();
        let mut second = source.advertisements();
        assert!(first.next().await.is_some() && second.next().await.is_some());
        drop(first);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(
            adapter.stops.load(Ordering::SeqCst),
            0,
            "one requester left"
        );
        drop(second);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(adapter.stops.load(Ordering::SeqCst), 1);
        assert_eq!(adapter.fake.scans.load(Ordering::SeqCst), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn a_requester_arriving_during_a_pending_stop_keeps_its_scan() {
        let adapter = Adapter::new(vec![("neo", Some("NEO"), false)]);
        let source = shared(&adapter);
        let mut first = source.advertisements();
        assert!(first.next().await.is_some());
        drop(first);
        // Let the pump observe the drop: the adapter stop is now pending (`stop_delay`).
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(adapter.pending.load(Ordering::SeqCst), 1);
        let mut second = source.advertisements();
        assert!(second.next().await.is_some(), "the new scan delivers");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            adapter.scanning.load(Ordering::SeqCst),
            1,
            "not stopped late"
        );
        assert_eq!(adapter.stops.load(Ordering::SeqCst), 1, "only the old stop");
        assert_eq!(adapter.fake.scans.load(Ordering::SeqCst), 2);
    }

    /// The last requester dropping, from a plain thread, races the pump on a worker: the
    /// drop must wake the pump only after the share is closed, or the pump re-sleeps and the
    /// scan runs forever (#265 round 2). Each trial is independent, since a later requester's
    /// drop would otherwise wake and heal a leaked pump.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn the_last_requester_dropping_never_leaks_the_scan() {
        let runtime = tokio::runtime::Handle::current();
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let runtime = runtime.clone();
                std::thread::spawn(move || {
                    let _entered = runtime.enter();
                    let mut leaked = 0;
                    for _ in 0..1000 {
                        let mut adapter = Adapter::new(vec![("neo", Some("NEO"), false)]);
                        adapter.stop_delay = Duration::ZERO;
                        let source = shared(&adapter);
                        let mut share = source.advertisements();
                        runtime.block_on(share.next());
                        drop(share);
                        let stopped =
                            runtime.block_on(tokio::time::timeout(Duration::from_secs(2), async {
                                while adapter.stops.load(Ordering::SeqCst) == 0 {
                                    tokio::task::yield_now().await;
                                }
                            }));
                        leaked += usize::from(stopped.is_err());
                    }
                    leaked
                })
            })
            .collect();
        let leaked: usize = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!(leaked, 0, "scans left running after their last requester");
    }
}
