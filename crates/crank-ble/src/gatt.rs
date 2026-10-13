//! Thin btleplug 0.13 adapter (ADR-0002 §§4.4, 6); scanning/reconnect are #147.
use crate::{Clock, FecTransport, Notifications};
use btleplug::{
    api::{
        Central as _, CentralEvent, Characteristic, Peripheral as _, ValueNotification, WriteType,
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
/// Adapter for a selected peripheral and its central; never scans or changes TCC.
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
impl FecTransport for BtleplugTransport {
    fn connect(
        &self,
        dev: DeviceRef,
        clock: Arc<dyn Clock>,
    ) -> BoxFuture<'static, Result<(DeviceInfo, Notifications), DeviceError>> {
        let (p, adapter) = (self.peripheral.clone(), self.adapter.clone());
        Box::pin(async move {
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
                let rx = characteristic(&p, RX)?;
                characteristic(&p, TX)?;
                let notifications = ingress(
                    p.notifications().await.map_err(gatt)?,
                    events,
                    p.id(),
                    clock,
                );
                p.subscribe(&rx).await.map_err(gatt)?;
                Ok((identity(&p).await?, notifications))
            }
            .await;
            if result.is_err() {
                p.disconnect().await.map_err(gatt)?;
            }
            result
        })
    }
    fn write(&self, bytes: [u8; 13]) -> BoxFuture<'static, Result<(), DeviceError>> {
        let p = self.peripheral.clone();
        Box::pin(async move {
            p.write(&characteristic(&p, TX)?, &bytes, WriteType::WithResponse)
                .await
                .map_err(gatt)
        })
    }
    fn disconnect(&self) -> BoxFuture<'static, Result<(), DeviceError>> {
        let p = self.peripheral.clone();
        Box::pin(async move { p.disconnect().await.map_err(gatt) })
    }
}
// Stamp at application ingress independently of writes/DIS reads. The owning
// stream cancels the pump on drop, including failed connection setup.
fn ingress(
    mut notifications: BoxStream<'static, ValueNotification>,
    mut events: BoxStream<'static, CentralEvent>,
    id: PeripheralId,
    clock: Arc<dyn Clock>,
) -> Notifications {
    let (sender, receiver) = mpsc::channel(256);
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                record = notifications.next() => {
                    let Some(n) = record else { break; };
                    if n.uuid == RX && sender.send(Ok((clock.now(), n.value))).await.is_err() { break; }
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
