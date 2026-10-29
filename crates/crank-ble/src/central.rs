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
    ConnectCleanup, FecTrainer, HrsDriver, Pairing, Reconnecting, ReconnectingHr, RideGate,
    Scanner, classify,
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
        supervised_trainer(
            |cleanup| PeripheralTrainer {
                central: self.clone(),
                clock: clock.clone(),
                cleanup,
            },
            clock.clone(),
            gate,
        )
    }
    /// Heart-rate strap driver with the reconnect loop, retrying while `gate` is active.
    #[must_use]
    pub fn hr_driver(&self, clock: Arc<dyn Clock>, gate: RideGate) -> impl HeartRateDriver {
        supervised_hr(
            |cleanup| PeripheralHr {
                central: self.clone(),
                clock: clock.clone(),
                cleanup,
            },
            clock.clone(),
            gate,
        )
    }
}

/// The production supervisor over the driver `build` makes from the cleanup tracker. The one
/// tracker is shared with the driver (which reports its cancelled connects to it) and the
/// supervisor (which waits for them before it reports the link released, #304); this is the
/// only place that wires the two, and a test builds it with a fake driver.
fn supervised_trainer<D: TrainerDriver>(
    build: impl FnOnce(ConnectCleanup) -> D,
    clock: Arc<dyn Clock>,
    gate: RideGate,
) -> Reconnecting<D> {
    let cleanup = ConnectCleanup::new();
    Reconnecting::new(build(cleanup.clone()), clock, gate).with_cleanup(cleanup)
}
/// [`supervised_trainer`] for a heart-rate strap.
fn supervised_hr<D: HeartRateDriver>(
    build: impl FnOnce(ConnectCleanup) -> D,
    clock: Arc<dyn Clock>,
    gate: RideGate,
) -> ReconnectingHr<D> {
    let cleanup = ConnectCleanup::new();
    ReconnectingHr::new(build(cleanup.clone()), clock, gate).with_cleanup(cleanup)
}

struct PeripheralTrainer {
    central: BleCentral,
    clock: Arc<dyn Clock>,
    cleanup: ConnectCleanup,
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
        let (central, clock, dev, cleanup) = (
            self.central.clone(),
            self.clock.clone(),
            dev.clone(),
            self.cleanup.clone(),
        );
        Box::pin(async move {
            let pairing = central.pairing(&dev);
            let peripheral = resolve(&central.source, &dev, pairing.as_ref(), RESOLVE_SCAN).await?;
            let transport = BtleplugTransport::new(peripheral, central.adapter.clone());
            let link = FecTrainer::new(Arc::new(transport), clock)
                .with_cleanup(cleanup)
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
    cleanup: ConnectCleanup,
}
impl HeartRateDriver for PeripheralHr {
    fn connect(&self, dev: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>> {
        let (central, clock, dev, cleanup) = (
            self.central.clone(),
            self.clock.clone(),
            dev.clone(),
            self.cleanup.clone(),
        );
        Box::pin(async move {
            let pairing = central.pairing(&dev);
            let peripheral = resolve(&central.source, &dev, pairing.as_ref(), RESOLVE_SCAN).await?;
            let transport = BtleplugHrTransport::new(peripheral, central.adapter.clone());
            let link = HrsDriver::new(Arc::new(transport), clock)
                .with_cleanup(cleanup)
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
    /// 400 adverts from one chatty device, then 100 new devices, while nobody polls: a bounded
    /// lossy channel (64 slots) is full of `d0` by the time the new devices arrive and drops
    /// them; an inbox without coalescing (M5) holds 500 adverts, not 101.
    #[tokio::test(start_paused = true)]
    async fn a_burst_delivers_every_device_once_to_slow_and_late_requesters() {
        let fresh: Vec<&'static str> = (0..100)
            .map(|i| &*Box::leak(format!("n{i}").into_boxed_str()))
            .collect();
        let script = std::iter::repeat_n("d0", 400)
            .chain(fresh)
            .map(|id| (id, Some("X"), false))
            .collect();
        let adapter = Adapter::new(script);
        let source = shared(&adapter);
        let (mut first, mut second) = (source.advertisements(), source.advertisements());
        // Nobody polls during the burst; the pump has fanned all 500 out by now.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut late = source.advertisements();
        for share in [&mut first, &mut second, &mut late] {
            let mut devices = Vec::new();
            while let Ok(Some(ad)) =
                tokio::time::timeout(Duration::from_millis(10), share.next()).await
            {
                devices.push(ad.device.0);
            }
            let distinct: std::collections::HashSet<_> = devices.iter().collect();
            assert_eq!(distinct.len(), 101, "no device is lost");
            assert_eq!(devices.len(), 101, "each device once, coalesced");
        }
    }
    /// A device that advertises again updates its pending advert instead of keeping the old
    /// one (M6), for a requester that polls late and for one that joins afterwards.
    #[tokio::test(start_paused = true)]
    async fn a_re_advertised_device_is_delivered_with_its_latest_advert() {
        let adapter = Adapter::new(vec![
            ("d0", Some("old"), false),
            ("d1", Some("other"), false),
            ("d0", Some("new"), false),
        ]);
        let source = shared(&adapter);
        let mut slow = source.advertisements();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut late = source.advertisements();
        for share in [&mut slow, &mut late] {
            let mut names = std::collections::BTreeMap::new();
            while let Ok(Some(ad)) =
                tokio::time::timeout(Duration::from_millis(10), share.next()).await
            {
                names.insert(ad.device.0, ad.name);
            }
            assert_eq!(names.len(), 2);
            assert_eq!(names["d0"].as_deref(), Some("new"), "the latest advert");
        }
    }
    /// An inner scan that ends ends every share (M7): a requester polling it sees the end
    /// instead of waiting for ever.
    #[tokio::test(start_paused = true)]
    async fn an_ended_scan_ends_every_share() {
        let mut adapter = Adapter::new(vec![("neo", Some("NEO"), false)]);
        adapter.fake.never_ends = false;
        let source = shared(&adapter);
        let (mut first, mut second) = (source.advertisements(), source.advertisements());
        for share in [&mut first, &mut second] {
            assert!(share.next().await.is_some());
            let end = tokio::time::timeout(Duration::from_secs(5), share.next()).await;
            assert_eq!(
                end.expect("the share is not left waiting")
                    .map(|a| a.device),
                None
            );
        }
    }
    /// What the scan saw is replayed only to joiners of that scan: the record is cleared
    /// when the last requester stops it and when the inner scan ends (M8), so a later scan
    /// cannot start from the previous one's devices.
    #[tokio::test(start_paused = true)]
    async fn the_replay_record_is_cleared_when_a_scan_stops_or_ends() {
        let adapter = Adapter::new(vec![("neo", Some("NEO"), false)]);
        let source = shared(&adapter);
        let mut share = source.advertisements();
        assert!(share.next().await.is_some());
        assert_eq!(lock(&source.shared.state).seen.len(), 1);
        drop(share);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(
            lock(&source.shared.state).seen.is_empty(),
            "cleared on stop"
        );

        let mut adapter = Adapter::new(vec![("neo", Some("NEO"), false)]);
        adapter.fake.never_ends = false;
        let source = shared(&adapter);
        let mut share = source.advertisements();
        assert!(share.next().await.is_some());
        let end = tokio::time::timeout(Duration::from_secs(5), share.next()).await;
        assert!(end.expect("the share is not left waiting").is_none());
        assert!(lock(&source.shared.state).seen.is_empty(), "cleared on end");
        assert!(!lock(&source.shared.state).running);
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
    /// Two requesters dropping at the same moment: the pump must see both closes. A wake
    /// that is not stored (`notify_waiters`, M2) is lost when the pump is between two waits
    /// (it handled the first drop and has not yet re-armed), and the pump then re-sleeps with
    /// the second requester already gone. Four trial tasks run concurrently, each dropping
    /// its two requesters from two tasks released by one barrier (no OS threads, no blocking
    /// a worker: #293). Trials stop at the first leak and the run is bounded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_requesters_dropping_together_never_leak_the_scan() {
        const TRIALS: usize = 3000;
        let leaked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let trials: Vec<_> = (0..4)
            .map(|_| {
                let leaked = leaked.clone();
                tokio::spawn(async move {
                    for _ in 0..TRIALS {
                        if leaked.load(Ordering::SeqCst) {
                            return;
                        }
                        let mut adapter = Adapter::new(vec![("neo", Some("NEO"), false)]);
                        adapter.stop_delay = Duration::ZERO;
                        let source = shared(&adapter);
                        let (mut a, mut b) = (source.advertisements(), source.advertisements());
                        a.next().await;
                        b.next().await;
                        let start = Arc::new(tokio::sync::Barrier::new(2));
                        let droppers: Vec<_> = [a, b]
                            .into_iter()
                            .map(|requester| {
                                let start = start.clone();
                                tokio::spawn(async move {
                                    start.wait().await;
                                    drop(requester);
                                })
                            })
                            .collect();
                        for dropper in droppers {
                            dropper.await.unwrap();
                        }
                        let stopped = tokio::time::timeout(Duration::from_secs(2), async {
                            while adapter.stops.load(Ordering::SeqCst) == 0 {
                                tokio::task::yield_now().await;
                            }
                        })
                        .await;
                        if stopped.is_err() {
                            leaked.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                })
            })
            .collect();
        for t in trials {
            t.await.unwrap();
        }
        assert!(
            !leaked.load(Ordering::SeqCst),
            "a scan was left running after both requesters dropped"
        );
    }

    /// A strap driver whose first connect succeeds and whose later connects hang, after
    /// starting a cleanup (reported to `cleanup`) that takes `hold`, for ever if `None`.
    struct HangingHr {
        cleanup: ConnectCleanup,
        hold: Option<Duration>,
        calls: Arc<AtomicUsize>,
        first: Arc<Mutex<Option<tokio::sync::mpsc::Sender<crank_devices::HrEvent>>>>,
    }
    impl HeartRateDriver for HangingHr {
        fn connect(&self, _: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                let (tx, events) = tokio::sync::mpsc::channel(8);
                *self.first.lock().unwrap() = Some(tx);
                return Box::pin(async {
                    Ok(HeartRateLink {
                        info: crank_devices::DeviceInfo::default(),
                        events,
                    })
                });
            }
            let (token, hold) = (self.cleanup.begin(), self.hold);
            tokio::spawn(async move {
                match hold {
                    Some(hold) => tokio::time::sleep(hold).await,
                    None => std::future::pending::<()>().await,
                }
                drop(token);
            });
            Box::pin(std::future::pending())
        }
    }
    /// Release a supervised strap whose reconnect attempt is stuck mid-connect and report
    /// how long the release took (`None`: not within 10 s).
    async fn time_to_release(hold: Option<Duration>) -> Option<Duration> {
        let clock: Arc<dyn Clock> = Arc::new(crate::MonotonicClock::default());
        let (calls, first) = (Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::default()));
        let driver = supervised_hr(
            |cleanup| HangingHr {
                cleanup,
                hold,
                calls: calls.clone(),
                first: first.clone(),
            },
            clock,
            RideGate::new(true),
        );
        let mut link = driver.connect(&DeviceRef("strap".into())).await.unwrap();
        // The link drops out; the reconnect attempt (after its 0.5 s backoff) hangs.
        let lost = first.lock().unwrap().take();
        drop(lost);
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let start = tokio::time::Instant::now();
        tokio::time::timeout(
            Duration::from_secs(10),
            crank_devices::release_events(&mut link.events),
        )
        .await
        .ok()
        .map(|()| start.elapsed())
    }
    /// #304: the production wiring: the supervisor built by `supervised_hr` (which
    /// `hr_driver` uses) waits for the cleanup of the connect it cancelled. Without
    /// `with_cleanup` it is released at once.
    #[tokio::test(start_paused = true)]
    async fn the_production_strap_supervisor_waits_for_the_cleanup_of_a_cancelled_connect() {
        let took = time_to_release(Some(Duration::from_millis(1500)))
            .await
            .unwrap();
        // the cleanup, started at 0.5 s, ends at 2.0 s; the release started at 0.7 s
        assert!(took >= Duration::from_secs(1), "{took:?}");
        assert!(took < Duration::from_millis(1500), "{took:?}");
    }
    /// #304: a cleanup that never settles holds the release for the supervisor's cap
    /// (the 2 s disconnect bound plus 1 s) and no longer.
    #[tokio::test(start_paused = true)]
    async fn a_cleanup_that_never_settles_holds_the_release_only_up_to_the_cap() {
        let took = time_to_release(None).await.expect("released within 10 s");
        assert!(took >= Duration::from_secs(3), "{took:?}");
        assert!(took < Duration::from_millis(3500), "{took:?}");
    }
    /// The trainer supervisor is wired the same way.
    #[tokio::test(start_paused = true)]
    async fn the_production_trainer_supervisor_waits_for_the_cleanup_of_a_cancelled_connect() {
        struct HangingTrainer {
            cleanup: ConnectCleanup,
            calls: Arc<AtomicUsize>,
            first: Arc<Mutex<Option<tokio::sync::mpsc::Sender<crank_devices::TrainerEvent>>>>,
        }
        impl TrainerDriver for HangingTrainer {
            fn kind(&self) -> DriverKind {
                DriverKind::FeC
            }
            fn connect(
                &self,
                _: &DeviceRef,
                _: RiderConfig,
            ) -> BoxFuture<'static, Result<TrainerLink, DeviceError>> {
                if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    let (tx, events) = tokio::sync::mpsc::channel(8);
                    *self.first.lock().unwrap() = Some(tx);
                    let (control, _) = tokio::sync::mpsc::channel(8);
                    return Box::pin(async {
                        Ok(TrainerLink {
                            info: crank_devices::DeviceInfo::default(),
                            caps: crank_devices::TrainerCaps {
                                erg: true,
                                sim: false,
                                resistance: true,
                                max_resistance_n: None,
                                max_target_w: 1000,
                            },
                            control,
                            events,
                        })
                    });
                }
                let token = self.cleanup.begin();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    drop(token);
                });
                Box::pin(std::future::pending())
            }
        }
        let clock: Arc<dyn Clock> = Arc::new(crate::MonotonicClock::default());
        let (calls, first) = (Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::default()));
        let driver = supervised_trainer(
            |cleanup| HangingTrainer {
                cleanup,
                calls: calls.clone(),
                first: first.clone(),
            },
            clock,
            RideGate::new(true),
        );
        let rider = RiderConfig {
            rider_kg: 75.0,
            bike_kg: 10.0,
            wheel_m: 0.7,
        };
        let mut link = driver
            .connect(&DeviceRef("neo".into()), rider)
            .await
            .unwrap();
        let lost = first.lock().unwrap().take();
        drop(lost);
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let start = tokio::time::Instant::now();
        drop(link.control);
        while link.events.recv().await.is_some() {}
        let took = start.elapsed();
        // the cleanup, started at 0.5 s, ends at 2.0 s; the release started at 0.7 s
        assert!(took >= Duration::from_secs(1), "{took:?}");
        assert!(took < Duration::from_millis(1500), "{took:?}");
    }
}
