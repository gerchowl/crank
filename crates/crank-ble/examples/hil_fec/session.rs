//! Live BLE session over the real `crank-ble` transport; every frame goes to a `.fecap`.
use btleplug::{
    api::{Manager as _, Peripheral as _},
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
    started: MonoTime,
    peripheral: Peripheral,
    cps_pump: Option<tokio::task::JoinHandle<Option<String>>>,
    log: Arc<Mutex<Vec<Rx>>>,
    pump: tokio::task::JoinHandle<Option<String>>,
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
        let transport = BtleplugTransport::new(peripheral.clone(), adapter);
        let started = clock.now();
        let capture = Arc::new(Capture::create(capture_path, started)?);
        let (info, mut notifications) = transport.connect(device.clone(), clock.clone()).await?;
        let log = Arc::new(Mutex::new(Vec::new()));
        let pump = tokio::spawn({
            let (log, capture) = (log.clone(), capture.clone());
            async move {
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
            }
        });
        Ok(Self {
            transport,
            clock,
            capture,
            capture_path: capture_path.to_path_buf(),
            device,
            info,
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
        self.pump.is_finished()
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
        self.pump.abort();
        let _ = self.pump.await;
        if let Some(cps) = self.cps_pump {
            cps.abort();
            let _ = cps.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::bounded;

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
