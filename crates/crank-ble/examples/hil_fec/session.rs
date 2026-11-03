//! Live BLE session over the real `crank-ble` transport; every frame goes to a `.fecap`.
use btleplug::{
    api::{Central as _, CentralEvent, Manager as _, Peripheral as _},
    platform::{Adapter, Manager, Peripheral},
};
use crank_ble::{
    Advertisements, BtleplugAdvertisements, BtleplugTransport, Capture, FEC_SERVICE, FecTransport,
    MonotonicClock,
};
use crank_devices::{Clock, DeviceInfo, DeviceRef, MonoTime};
use futures_util::StreamExt;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use tokio::time::{Instant, sleep, timeout_at};
use uuid::Uuid;

/// Cycling Power Measurement characteristic (Bluetooth SIG 2A63).
const CPS_MEASUREMENT: Uuid = Uuid::from_u128(0x0000_2a63_0000_1000_8000_0080_5f9b_34fb);

/// Await a GATT operation for at most [`crate::rider::WRITE_WAIT`] (ADR-0002 §6: an operation
/// unacknowledged this long means the link is lost).
pub async fn bounded<T, E: std::error::Error + Send + Sync + 'static>(
    what: &str,
    op: impl std::future::Future<Output = std::result::Result<T, E>>,
) -> Result<T> {
    match tokio::time::timeout(crate::rider::WRITE_WAIT, op).await {
        Ok(r) => Ok(r?),
        Err(_) => Err(format!("{what} not completed within 2 s").into()),
    }
}

/// Boxed error: this is an operator tool, failures are printed.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// One peripheral seen during a scan, services unioned over its advertisements.
#[derive(Clone, Debug)]
pub struct Seen {
    pub name: Option<String>,
    pub services: Vec<Uuid>,
    pub adverts: u32,
}

/// A scan of `source`: `true` for each advertisement of `device` (the connected trainer), `false`
/// for any other peripheral's. It ends when the scan stops by itself.
pub fn attributed(
    source: &impl Advertisements,
    device: DeviceRef,
) -> futures_util::stream::BoxStream<'static, bool> {
    source
        .advertisements()
        .map(move |ad| ad.device == device)
        .boxed()
}

/// Resolves `true` once `events` (the adapter's events: a connected peripheral's id, `None` for
/// any other kind) reports `device` connected, `false` when they end first.
pub async fn connect_of(
    mut events: impl futures_util::Stream<Item = Option<String>> + Unpin,
    device: &DeviceRef,
) -> bool {
    while let Some(e) = events.next().await {
        if e.as_deref() == Some(device.0.as_str()) {
            return true;
        }
    }
    false
}

/// The unit reconnected is the one first connected: their DIS serials differ only for another
/// unit (a missing one is left to the page-81 check, `check11::same_trainer`).
pub fn same_unit(first: &DeviceInfo, again: &DeviceInfo) -> Result<()> {
    match (&first.serial, &again.serial) {
        (Some(a), Some(b)) if a != b => {
            Err("a different trainer answered after the reconnect (DIS serials differ)".into())
        }
        _ => Ok(()),
    }
}

/// Aborts its task when dropped, so a watcher never outlives the call that started it.
struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A reconnect apart from btleplug: `open` connects, discovers and subscribes; `events` are the
/// adapter's, as [`connect_of`] reads them. The connect event of `device` is stamped
/// `# link <us> 03` while `open` runs and up to 1 s after; the watcher is aborted when this returns
/// or is dropped (a cancelled reconnect), so no stamp lands later. On success the new link's
/// frames are pumped into `log` and `capture`, in place of the old link's pump in `slot`.
pub async fn relink(
    open: impl std::future::Future<
        Output = std::result::Result<
            (DeviceInfo, crank_ble::Notifications),
            crank_devices::DeviceError,
        >,
    >,
    events: impl futures_util::Stream<Item = Option<String>> + Send + Unpin + 'static,
    device: DeviceRef,
    clock: &Arc<MonotonicClock>,
    capture: &Arc<Capture>,
    log: &Arc<Mutex<Vec<Rx>>>,
    slot: &Mutex<tokio::task::JoinHandle<Option<String>>>,
) -> Result<DeviceInfo> {
    use crate::check11::{CONNECTED, LINK};
    let (stamp_clock, stamp_capture) = (clock.clone(), capture.clone());
    let mut watcher = AbortOnDrop(tokio::spawn(async move {
        if connect_of(events, &device).await {
            return stamp_capture
                .record_aux(stamp_clock.now(), LINK, &[CONNECTED])
                .err();
        }
        None
    }));
    let opened = open.await;
    // The connect event is already queued once the link is open; give it 1 s, then stop watching.
    let recorded = tokio::time::timeout(Duration::from_secs(1), &mut watcher.0).await;
    drop(watcher);
    let (info, notifications) = opened?;
    if let Ok(Ok(Some(e))) = recorded {
        return Err(format!("capture: {e}").into());
    }
    *slot.lock().unwrap_or_else(PoisonError::into_inner) =
        pump(notifications, log.clone(), capture.clone());
    Ok(info)
}

/// Default adapter of the first BLE manager.
pub async fn adapter() -> Result<Adapter> {
    let manager = Manager::new().await?;
    manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| "no Bluetooth adapter".into())
}

/// Scan up to `secs`, returning early once an FE-C service is seen when `stop_on_fec`.
pub async fn scan(
    adv: &BtleplugAdvertisements,
    secs: u64,
    stop_on_fec: bool,
) -> BTreeMap<String, Seen> {
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut stream = adv.advertisements();
    while let Ok(Some(ad)) = timeout_at(deadline, stream.next()).await {
        let entry = seen.entry(ad.device.0.clone()).or_insert(Seen {
            name: None,
            services: Vec::new(),
            adverts: 0,
        });
        entry.adverts += 1;
        entry.name = ad.name.or(entry.name.take());
        for s in ad.services {
            if !entry.services.contains(&s) {
                entry.services.push(s);
            }
        }
        if stop_on_fec && entry.services.contains(&FEC_SERVICE) {
            break;
        }
    }
    seen
}

/// A received notification with its arrival stamp.
#[derive(Clone, Debug)]
pub struct Rx {
    pub at: MonoTime,
    pub bytes: Vec<u8>,
}

/// Result of one write and its awaited reply.
#[derive(Debug)]
pub struct Exchange {
    pub sent: [u8; 13],
    pub write_error: Option<String>,
    pub reply: Option<Rx>,
    pub latency_us: Option<u64>,
}

/// Connected FE-C peripheral with a recording rx pump.
pub struct Session {
    pub transport: BtleplugTransport,
    pub clock: Arc<MonotonicClock>,
    pub capture: Arc<Capture>,
    pub capture_path: std::path::PathBuf,
    pub device: DeviceRef,
    pub info: DeviceInfo,
    adv: BtleplugAdvertisements,
    adapter: Adapter,
    started: MonoTime,
    peripheral: Peripheral,
    cps_pump: Option<tokio::task::JoinHandle<Option<String>>>,
    log: Arc<Mutex<Vec<Rx>>>,
    /// The rx pump of the current connection; a reconnect starts a new one.
    pump: Mutex<tokio::task::JoinHandle<Option<String>>>,
}

/// Record each notification in `capture` and keep it in `log`; ends with the link.
fn pump(
    mut notifications: crank_ble::Notifications,
    log: Arc<Mutex<Vec<Rx>>>,
    capture: Arc<Capture>,
) -> tokio::task::JoinHandle<Option<String>> {
    tokio::spawn(async move {
        while let Some(item) = notifications.next().await {
            match item {
                Ok((at, bytes)) => {
                    if let Err(e) = capture.record(at, true, &bytes) {
                        return Some(format!("capture: {e}"));
                    }
                    log.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(Rx { at, bytes });
                }
                Err(e) => return Some(e.to_string()),
            }
        }
        None
    })
}

/// Locate the NEO (scan up to `secs`); `None` when no FE-C device advertises.
pub async fn find_trainer(
    adv: &BtleplugAdvertisements,
    secs: u64,
) -> Option<(DeviceRef, BTreeMap<String, Seen>)> {
    let seen = scan(adv, secs, true).await;
    let id = seen
        .iter()
        .find(|(_, s)| s.services.contains(&FEC_SERVICE))
        .map(|(id, _)| DeviceRef(id.clone()))?;
    Some((id, seen))
}

impl Session {
    /// Resolve `device` on the scanned central, connect and subscribe; rx frames are
    /// logged to `capture` and kept for [`Session::wait_page`].
    pub async fn open(
        adv: &BtleplugAdvertisements,
        adapter: Adapter,
        device: DeviceRef,
        capture_path: &Path,
    ) -> Result<Self> {
        let clock = Arc::new(MonotonicClock::default());
        let peripheral = adv.peripheral(&device).await?;
        let transport = BtleplugTransport::new(peripheral.clone(), adapter.clone());
        let started = clock.now();
        let capture = Arc::new(Capture::create(capture_path, started)?);
        let (info, notifications) = transport.connect(device.clone(), clock.clone()).await?;
        let log = Arc::new(Mutex::new(Vec::new()));
        let pump = Mutex::new(pump(notifications, log.clone(), capture.clone()));
        Ok(Self {
            transport,
            clock,
            capture,
            capture_path: capture_path.to_path_buf(),
            device,
            info,
            adv: adv.clone(),
            adapter,
            started,
            peripheral,
            cps_pump: None,
            log,
            pump,
        })
    }

    /// Subscribe (read-only) to the Cycling Power measurement 2A63 on this connection and record
    /// each notification in the capture as a `# cps` line (check 5). Fails when the device has
    /// no such characteristic; nothing is written to the trainer.
    pub async fn subscribe_cps(&mut self) -> Result<()> {
        let p = self.peripheral.clone();
        let characteristic = p
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == CPS_MEASUREMENT)
            .ok_or("the device has no Cycling Power measurement (2A63)")?;
        let mut notifications = p.notifications().await?;
        bounded("subscribe to 2A63", p.subscribe(&characteristic)).await?;
        let (clock, capture) = (self.clock.clone(), self.capture.clone());
        self.cps_pump = Some(tokio::spawn(async move {
            while let Some(n) = notifications.next().await {
                if n.uuid != CPS_MEASUREMENT {
                    continue;
                }
                if let Err(e) = capture.record_aux(clock.now(), "cps", &n.value) {
                    return Some(format!("capture: {e}"));
                }
            }
            None
        }));
        Ok(())
    }

    /// A scan beside the connection (check 10), attributed by [`attributed`].
    pub fn adverts(&self) -> futures_util::stream::BoxStream<'static, bool> {
        attributed(&self.adv, self.device.clone())
    }

    /// After a drop: connect again, discover and subscribe, and pump the new link's frames into
    /// the same log and capture ([`relink`]). A unit whose DIS serial differs from the one first
    /// connected is refused (its link is up, so the caller's release still reaches it).
    pub async fn reconnect(&self) -> Result<()> {
        let events = self.adapter.events().await?.map(|e| match e {
            CentralEvent::DeviceConnected(p) => Some(p.to_string()),
            _ => None,
        });
        let open = self
            .transport
            .connect(self.device.clone(), self.clock.clone());
        let info = relink(
            open,
            events,
            self.device.clone(),
            &self.clock,
            &self.capture,
            &self.log,
            &self.pump,
        )
        .await?;
        same_unit(&self.info, &info)
    }

    /// Number of frames received so far (use as a `since` mark).
    pub fn mark(&self) -> usize {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Frames received since `mark`.
    pub fn since(&self, mark: usize) -> Vec<Rx> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)[mark..].to_vec()
    }

    /// Microseconds since the capture started.
    pub fn elapsed_us(&self) -> u64 {
        self.capture_us(self.clock.now())
    }

    /// A clock stamp on the capture's time base.
    pub fn capture_us(&self, at: MonoTime) -> u64 {
        at.0.saturating_sub(self.started.0)
    }

    /// Why the rx pump ended, if it has (disconnect, capture failure).
    pub fn pump_ended(&self) -> bool {
        self.pump
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_finished()
            || self
                .cps_pump
                .as_ref()
                .is_some_and(tokio::task::JoinHandle::is_finished)
    }

    /// First rx frame carrying `page` after `mark`, waiting up to `wait`.
    pub async fn wait_page(&self, mark: usize, page: u8, wait: Duration) -> Option<Rx> {
        let deadline = Instant::now() + wait;
        loop {
            if let Some(rx) = self
                .since(mark)
                .into_iter()
                .find(|r| crate::frames::page_of(&r.bytes) == Some(page))
            {
                return Some(rx);
            }
            if Instant::now() >= deadline {
                return None;
            }
            sleep(Duration::from_millis(10)).await;
        }
    }

    /// Record and write one frame; returns the host-side stamp and any write error.
    pub async fn send(&self, frame: [u8; 13], with_response: bool) -> (MonoTime, Option<String>) {
        let at = self.clock.now();
        if let Err(e) = self.capture.record(at, false, &frame) {
            return (at, Some(format!("capture: {e}")));
        }
        let result = if with_response {
            FecTransport::write(&self.transport, frame).await
        } else {
            self.transport.write_without_response(frame).await
        };
        (at, result.err().map(|e| e.to_string()))
    }

    /// Write `frame` and wait up to `wait` for the first reply carrying `reply_page`.
    pub async fn exchange(
        &self,
        frame: [u8; 13],
        with_response: bool,
        reply_page: u8,
        wait: Duration,
    ) -> Exchange {
        let mark = self.mark();
        let (at, write_error) = self.send(frame, with_response).await;
        let reply = self.wait_page(mark, reply_page, wait).await;
        let latency_us = reply.as_ref().map(|r| r.at.0.saturating_sub(at.0));
        Exchange {
            sent: frame,
            write_error,
            reply,
            latency_us,
        }
    }

    /// Release the peripheral; once this returns the pump no longer writes the capture.
    pub async fn close(self) {
        let _ = FecTransport::disconnect(&self.transport).await;
        let pump = self
            .pump
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        pump.abort();
        let _ = pump.await;
        if let Some(cps) = self.cps_pump {
            cps.abort();
            let _ = cps.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Rx, attributed, bounded, connect_of, relink, same_unit};
    use crank_ble::{Advertisement, Advertisements};
    use crank_ble::{Capture, MonotonicClock};
    use crank_devices::DeviceRef;
    use crank_devices::{Clock, DeviceInfo, MonoTime};
    use futures_util::{StreamExt, stream::BoxStream};
    use std::sync::{Arc, Mutex};

    /// A scan that reports these devices' advertisements, in order, then ends.
    struct Scan(&'static [&'static str]);

    impl Advertisements for Scan {
        fn advertisements(&self) -> BoxStream<'static, Advertisement> {
            futures_util::stream::iter(self.0.iter().map(|d| Advertisement {
                device: DeviceRef((*d).into()),
                name: None,
                services: Vec::new(),
            }))
            .boxed()
        }
    }

    #[tokio::test]
    async fn only_the_connected_trainers_adverts_count_as_its_own() {
        let scan = Scan(&["phone", "neo", "watch", "neo-2", "neo", "phone"]);
        let seen: Vec<bool> = attributed(&scan, DeviceRef("neo".into())).collect().await;
        assert_eq!(seen, [false, true, false, false, true, false]);
    }

    /// What a reconnect feeds: a capture in `dir`, an empty log and the old link's ended pump.
    struct Parts {
        clock: Arc<MonotonicClock>,
        capture: Arc<Capture>,
        path: std::path::PathBuf,
        log: Arc<Mutex<Vec<Rx>>>,
        slot: Mutex<tokio::task::JoinHandle<Option<String>>>,
    }

    impl Parts {
        fn new(dir: &tempfile::TempDir) -> Self {
            let clock = Arc::new(MonotonicClock::default());
            let path = dir.path().join("check11b.fecap");
            let capture = Arc::new(Capture::create(&path, clock.now()).expect("capture"));
            Self {
                clock,
                capture,
                path,
                log: Arc::new(Mutex::new(Vec::new())),
                slot: Mutex::new(tokio::spawn(async { None })),
            }
        }

        fn link_lines(&self) -> usize {
            std::fs::read_to_string(&self.path)
                .expect("read")
                .lines()
                .filter(|l| l.starts_with("# link "))
                .count()
        }

        async fn relink(
            &self,
            open: impl std::future::Future<
                Output = Result<(DeviceInfo, crank_ble::Notifications), crank_devices::DeviceError>,
            >,
            events: BoxStream<'static, Option<String>>,
        ) -> super::Result<DeviceInfo> {
            let (clock, capture, log) = (&self.clock, &self.capture, &self.log);
            relink(
                open,
                events,
                DeviceRef("neo".into()),
                clock,
                capture,
                log,
                &self.slot,
            )
            .await
        }
    }

    /// Adapter events sent through the returned channel.
    fn events() -> (
        tokio::sync::mpsc::UnboundedSender<Option<String>>,
        BoxStream<'static, Option<String>>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let stream =
            futures_util::stream::unfold(
                rx,
                |mut rx| async move { rx.recv().await.map(|e| (e, rx)) },
            );
        (tx, stream.boxed())
    }

    /// A link that delivers one page-25 frame.
    fn opened() -> (DeviceInfo, crank_ble::Notifications) {
        let frame = vec![0xa4, 9, 0x4e, 5, 25, 0, 90, 0, 0, 100, 0, 0x30, 0];
        (
            DeviceInfo::default(),
            futures_util::stream::iter(vec![Ok((MonoTime(1), frame))]).boxed(),
        )
    }

    #[test]
    fn a_reconnect_to_another_units_serial_is_refused() {
        let unit = |serial: Option<&str>| DeviceInfo {
            serial: serial.map(String::from),
            ..DeviceInfo::default()
        };
        assert!(same_unit(&unit(Some("1")), &unit(Some("1"))).is_ok());
        assert!(same_unit(&unit(Some("1")), &unit(None)).is_ok());
        assert!(same_unit(&unit(None), &unit(Some("2"))).is_ok());
        assert_eq!(
            same_unit(&unit(Some("1")), &unit(Some("2"))).map_err(|e| e.to_string()),
            Err("a different trainer answered after the reconnect (DIS serials differ)".into())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_reconnect_pumps_the_new_link_and_stamps_only_the_trainers_connect() {
        let dir = tempfile::tempdir().expect("tmp");
        let parts = Parts::new(&dir);
        let (tx, ev) = events();
        for e in [None, Some("phone"), Some("neo")] {
            tx.send(e.map(String::from)).expect("send");
        }
        parts
            .relink(async { Ok(opened()) }, ev)
            .await
            .expect("relinked");
        for _ in 0..100 {
            if !parts.log.lock().expect("log").is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            parts.log.lock().expect("log").len(),
            1,
            "the new pump feeds the log"
        );
        assert_eq!(parts.link_lines(), 1);
        // A failed connect installs nothing and stamps nothing without the event.
        let parts = Parts::new(&tempfile::tempdir().expect("tmp"));
        let (_tx, ev) = events();
        let failed = parts
            .relink(async { Err(crank_devices::DeviceError::NotFound) }, ev)
            .await;
        assert!(failed.is_err());
        assert!(parts.slot.lock().expect("slot").is_finished());
    }

    #[tokio::test(start_paused = true)]
    async fn no_connect_stamp_lands_after_a_reconnect_returned_or_was_cancelled() {
        let dir = tempfile::tempdir().expect("tmp");
        let parts = Parts::new(&dir);
        let (tx, ev) = events();
        parts
            .relink(async { Ok(opened()) }, ev)
            .await
            .expect("relinked");
        tx.send(Some("neo".into())).expect("send");
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert_eq!(parts.link_lines(), 0, "late event after the return");
        let (tx, ev) = events();
        let cancelled = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            parts.relink(std::future::pending(), ev),
        )
        .await;
        assert!(cancelled.is_err());
        // The send fails or is never read: the watcher is gone with the cancelled call.
        let _ = tx.send(Some("neo".into()));
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert_eq!(parts.link_lines(), 0, "event after the cancel");
    }

    #[tokio::test]
    async fn only_the_trainers_connect_event_counts() {
        let neo = DeviceRef("neo".into());
        let events = |e: &[Option<&str>]| {
            futures_util::stream::iter(e.iter().map(|x| x.map(String::from)).collect::<Vec<_>>())
        };
        assert!(
            connect_of(
                events(&[None, Some("phone"), Some("neo-2"), Some("neo")]),
                &neo
            )
            .await
        );
        assert!(!connect_of(events(&[None, Some("phone"), Some("neo-2")]), &neo).await);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_gatt_operation_fails_within_the_bound() {
        let started = tokio::time::Instant::now();
        let hung =
            bounded::<(), btleplug::Error>("subscribe to 2A63", std::future::pending()).await;
        assert_eq!(
            hung.err().map(|e| e.to_string()),
            Some("subscribe to 2A63 not completed within 2 s".into())
        );
        assert_eq!(started.elapsed(), crate::rider::WRITE_WAIT);
        let failed = bounded::<(), _>("x", async { Err(btleplug::Error::NotConnected) }).await;
        assert!(failed.is_err());
        assert_eq!(
            bounded("x", async { Ok::<_, btleplug::Error>(7) })
                .await
                .ok(),
            Some(7)
        );
    }
}
