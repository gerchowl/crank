//! Thin btleplug 0.13 adapter (ADR-0002 §§4.4, 5, 6). Real-device behaviour is HIL #44.
use crate::{
    Advertisement, Advertisements, Clock, FecTransport, HrTransport, Notifications, SCAN_SERVICES,
};
use btleplug::{
    api::{
        Central as _, CentralEvent, Characteristic, Peripheral as _, ScanFilter, ValueNotification,
        WriteType,
    },
    platform::{Adapter, Peripheral, PeripheralId},
};
use crank_devices::{DeviceError, DeviceInfo, DeviceRef, MonoTime};
use futures_util::{Stream, StreamExt, future::BoxFuture, stream::BoxStream};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{sync::mpsc, task::JoinHandle};
use uuid::Uuid;
const RX: Uuid = Uuid::from_u128(0x6e40fec2_b5a3_f393_e0a9_e50e24dcca9e);
const TX: Uuid = Uuid::from_u128(0x6e40fec3_b5a3_f393_e0a9_e50e24dcca9e);
/// Heart Rate Measurement (2A37).
const HRM: Uuid = Uuid::from_u128(0x0000_2a37_0000_1000_8000_0080_5f9b_34fb);
/// Adapter for a selected peripheral and its central; never changes TCC.
#[derive(Clone)]
pub struct BtleplugTransport {
    peripheral: Peripheral,
    adapter: Adapter,
}
impl BtleplugTransport {
    /// Wrap a peripheral selected by discovery and its owning central (ADR-0002 §6).
    #[must_use]
    pub fn new(peripheral: Peripheral, adapter: Adapter) -> Self {
        Self {
            peripheral,
            adapter,
        }
    }
}
impl BtleplugTransport {
    fn write_type(
        &self,
        bytes: [u8; 13],
        kind: WriteType,
    ) -> BoxFuture<'static, Result<(), DeviceError>> {
        let p = self.peripheral.clone();
        Box::pin(async move {
            p.write(&characteristic(&p, TX)?, &bytes, kind)
                .await
                .map_err(gatt)
        })
    }
    /// HIL test hook (#44 check 2, feature `hil`, off by default): write one frame to
    /// fec3 without GATT response, which [`FecTransport::write`] never does (ADR-0002 §4.4).
    #[cfg(feature = "hil")]
    #[must_use]
    pub fn write_without_response(
        &self,
        bytes: [u8; 13],
    ) -> BoxFuture<'static, Result<(), DeviceError>> {
        self.write_type(bytes, WriteType::WithoutResponse)
    }
}
fn gatt(error: btleplug::Error) -> DeviceError {
    match error {
        btleplug::Error::PermissionDenied => DeviceError::PermissionDenied,
        other => DeviceError::Gatt(other.to_string()),
    }
}
fn characteristic(p: &Peripheral, uuid: Uuid) -> Result<Characteristic, DeviceError> {
    p.characteristics()
        .into_iter()
        .find(|c| c.uuid == uuid)
        .ok_or_else(|| DeviceError::Gatt(format!("missing characteristic {uuid}")))
}
async fn identity(p: &Peripheral) -> Result<DeviceInfo, DeviceError> {
    let mut info = DeviceInfo {
        name: p
            .properties()
            .await
            .map_err(gatt)?
            .and_then(|p| p.local_name),
        ..DeviceInfo::default()
    };
    for (uuid, field) in [
        (0x2a24, &mut info.model),
        (0x2a25, &mut info.serial),
        (0x2a26, &mut info.firmware),
    ] {
        let uuid = Uuid::from_u128((uuid << 96) | 0x0000_1000_8000_0080_5f9b_34fb);
        if let Some(c) = p.characteristics().into_iter().find(|c| c.uuid == uuid) {
            *field = Some(
                String::from_utf8(p.read(&c).await.map_err(gatt)?)
                    .map_err(|e| DeviceError::Gatt(e.to_string()))?,
            );
        }
    }
    Ok(info)
}
/// Connect, subscribe `notify` and read DIS; `required` must also exist.
fn open(
    peripheral: Peripheral,
    adapter: Adapter,
    dev: DeviceRef,
    clock: Arc<dyn Clock>,
    notify: Uuid,
    required: Option<Uuid>,
) -> BoxFuture<'static, Result<(DeviceInfo, Notifications), DeviceError>> {
    Box::pin(async move {
        let p = peripheral;
        if p.id().to_string() != dev.0 {
            return Err(DeviceError::NotFound);
        }
        let events = adapter.events().await.map_err(gatt)?;
        p.connect().await.map_err(|e| match e {
            btleplug::Error::PermissionDenied => DeviceError::PermissionDenied,
            other => DeviceError::ConnectFailed(other.to_string()),
        })?;
        let result = async {
            p.discover_services().await.map_err(gatt)?;
            let subscribed = characteristic(&p, notify)?;
            if let Some(uuid) = required {
                characteristic(&p, uuid)?;
            }
            let notifications = ingress(
                p.notifications().await.map_err(gatt)?,
                events,
                p.id(),
                clock,
                notify,
            );
            p.subscribe(&subscribed).await.map_err(gatt)?;
            Ok((identity(&p).await?, notifications))
        }
        .await;
        if result.is_err()
            && let Err(error) = crate::driver::disconnect_bounded(Box::pin({
                let p = p.clone();
                async move { p.disconnect().await.map_err(gatt) }
            }))
            .await
        {
            tracing::warn!(%error, "disconnect after a failed setup failed");
        }
        result
    })
}
impl FecTransport for BtleplugTransport {
    fn connect(
        &self,
        dev: DeviceRef,
        clock: Arc<dyn Clock>,
    ) -> BoxFuture<'static, Result<(DeviceInfo, Notifications), DeviceError>> {
        open(
            self.peripheral.clone(),
            self.adapter.clone(),
            dev,
            clock,
            RX,
            Some(TX),
        )
    }
    fn write(&self, bytes: [u8; 13]) -> BoxFuture<'static, Result<(), DeviceError>> {
        self.write_type(bytes, WriteType::WithResponse)
    }
    fn disconnect(&self) -> BoxFuture<'static, Result<(), DeviceError>> {
        let p = self.peripheral.clone();
        Box::pin(async move { p.disconnect().await.map_err(gatt) })
    }
}
/// Heart-rate strap over btleplug; same lifecycle as [`BtleplugTransport`].
#[derive(Clone)]
pub struct BtleplugHrTransport(BtleplugTransport);
impl BtleplugHrTransport {
    /// Wrap a peripheral selected by discovery and its owning central.
    #[must_use]
    pub fn new(peripheral: Peripheral, adapter: Adapter) -> Self {
        Self(BtleplugTransport::new(peripheral, adapter))
    }
}
impl HrTransport for BtleplugHrTransport {
    fn connect(
        &self,
        dev: DeviceRef,
        clock: Arc<dyn Clock>,
    ) -> BoxFuture<'static, Result<(DeviceInfo, Notifications), DeviceError>> {
        let t = &self.0;
        open(
            t.peripheral.clone(),
            t.adapter.clone(),
            dev,
            clock,
            HRM,
            None,
        )
    }
    fn disconnect(&self) -> BoxFuture<'static, Result<(), DeviceError>> {
        FecTransport::disconnect(&self.0)
    }
}
/// btleplug scan source (ADR-0002 §5): starts a filtered scan on the central and
/// reports each peripheral's name and services; [`crate::Scanner`] post-filters.
///
/// btleplug keeps one scan per adapter, so scans of one source are serialised: a scan
/// task holds `scan_lock` from before `start_scan` until after its `stop_scan` has
/// completed, and the next scan waits for it. A stop therefore never lands on a newer
/// scan (#265). This lock is the protection; `central::SharedScan` also awaits
/// [`Advertisements::stopped`] before starting (belt and braces, and what the fake-source
/// tests exercise). Concurrent requesters are shared one level up there.
#[derive(Clone)]
pub struct BtleplugAdvertisements {
    adapter: Adapter,
    scan_lock: Arc<tokio::sync::Mutex<()>>,
}
impl BtleplugAdvertisements {
    /// Scan on `adapter`.
    #[must_use]
    pub fn new(adapter: Adapter) -> Self {
        Self {
            adapter,
            scan_lock: Arc::default(),
        }
    }
    /// Resolve a discovered or remembered platform id to its peripheral.
    /// # Errors
    /// `NotFound` if the central has not seen it (scan first).
    pub async fn peripheral(&self, dev: &DeviceRef) -> Result<Peripheral, DeviceError> {
        self.adapter
            .peripherals()
            .await
            .map_err(gatt)?
            .into_iter()
            .find(|p| p.id().to_string() == dev.0)
            .ok_or(DeviceError::NotFound)
    }
}
impl Advertisements for BtleplugAdvertisements {
    fn advertisements(&self) -> BoxStream<'static, Advertisement> {
        let adapter = self.adapter.clone();
        let lock = self.scan_lock.clone();
        let (sender, receiver) = mpsc::channel(64);
        tokio::spawn(async move {
            // Held until the scan has stopped, so the next scan starts after it.
            let _slot = lock.lock_owned().await;
            if sender.is_closed() {
                return;
            }
            let Ok(mut events) = adapter.events().await else {
                return;
            };
            if let Err(error) = adapter
                .start_scan(ScanFilter {
                    services: SCAN_SERVICES.to_vec(),
                })
                .await
            {
                tracing::warn!(%error, "BLE scan failed to start");
                return;
            }
            loop {
                let event = tokio::select! {
                    () = sender.closed() => break,
                    event = events.next() => event,
                };
                let Some(
                    CentralEvent::DeviceDiscovered(id)
                    | CentralEvent::DeviceUpdated(id)
                    | CentralEvent::ServicesAdvertisement { id, .. },
                ) = event
                else {
                    if event.is_none() {
                        break;
                    }
                    continue;
                };
                let Ok(p) = adapter.peripheral(&id).await else {
                    continue;
                };
                let Ok(Some(props)) = p.properties().await else {
                    continue;
                };
                let ad = Advertisement {
                    device: DeviceRef(id.to_string()),
                    name: props.local_name,
                    services: props.services,
                };
                if sender.send(ad).await.is_err() {
                    break;
                }
            }
            if let Err(error) = adapter.stop_scan().await {
                tracing::warn!(%error, "BLE scan failed to stop");
            }
        });
        Box::pin(ScanStream { receiver })
    }
    fn stopped(&self) -> BoxFuture<'static, ()> {
        let lock = self.scan_lock.clone();
        Box::pin(async move { drop(lock.lock().await) })
    }
}
/// Dropping it closes the channel, which makes the scan task stop the adapter scan.
struct ScanStream {
    receiver: mpsc::Receiver<Advertisement>,
}
impl Stream for ScanStream {
    type Item = Advertisement;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}
// Stamp at application ingress independently of writes/DIS reads. The owning
// stream cancels the pump on drop, including failed connection setup.
fn ingress(
    mut notifications: BoxStream<'static, ValueNotification>,
    mut events: BoxStream<'static, CentralEvent>,
    id: PeripheralId,
    clock: Arc<dyn Clock>,
    notify: Uuid,
) -> Notifications {
    let (sender, receiver) = mpsc::channel(256);
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                record = notifications.next() => {
                    let Some(n) = record else { break; };
                    if n.uuid == notify && sender.send(Ok((clock.now(), n.value))).await.is_err() { break; }
                }
                event = events.next() => {
                    if event.is_none() || matches!(event, Some(CentralEvent::DeviceDisconnected(ref device)) if *device == id) {
                        let _ = sender.send(Err(DeviceError::Disconnected)).await;
                        break;
                    }
                }
            }
        }
    });
    Box::pin(Ingress { receiver, task })
}
struct Ingress {
    receiver: mpsc::Receiver<Result<(MonoTime, Vec<u8>), DeviceError>>,
    task: JoinHandle<()>,
}
impl Stream for Ingress {
    type Item = Result<(MonoTime, Vec<u8>), DeviceError>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}
impl Drop for Ingress {
    fn drop(&mut self) {
        self.task.abort();
    }
}
