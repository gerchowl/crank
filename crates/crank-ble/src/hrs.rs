//! Heart Rate Service driver: 2A37 parsing over the shared notification seam (ADR-0002 §5).
use crate::{
    Clock, Notifications,
    driver::{Cleanup, ConnectGuard},
};
use crank_devices::{
    DeviceError, DeviceInfo, DeviceRef, HeartRateDriver, HeartRateLink, HrEvent, HrSample,
    LinkState, MonoTime, ProtocolErrorKind,
};
use futures_util::{StreamExt, future::BoxFuture};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Injectable HRS transport; connect must subscribe 2A37 before returning.
pub trait HrTransport: Send + Sync + 'static {
    /// Connect, discover, subscribe 2A37 and read identity.
    fn connect(
        &self,
        dev: DeviceRef,
        clock: Arc<dyn Clock>,
    ) -> BoxFuture<'static, Result<(DeviceInfo, Notifications), DeviceError>>;
    /// Release the peripheral.
    fn disconnect(&self) -> BoxFuture<'static, Result<(), DeviceError>>;
}

/// Parse one Heart Rate Measurement (2A37) payload. Never panics.
///
/// Flags: bit 0 = 16-bit value, bit 1 = contact detected, bit 2 = contact
/// supported, bit 3 = energy expended present (skipped), bit 4 = RR present.
/// `bpm` is `None` for the spec's "no value" 0 and for a 16-bit value above
/// 255. RR intervals (1/1024 s) are converted to rounded milliseconds.
///
/// # Errors
/// A payload shorter than its flags promise, or an RR field of odd length,
/// is a `Framing` protocol error.
pub fn parse_heart_rate(at: MonoTime, bytes: &[u8]) -> Result<HrSample, DeviceError> {
    let framing = |what: &str| crate::protocol(what, ProtocolErrorKind::Framing);
    let (&flags, mut rest) = bytes.split_first().ok_or_else(|| framing("empty 2A37"))?;
    let raw = if flags & 1 == 0 {
        let (&v, tail) = rest
            .split_first()
            .ok_or_else(|| framing("2A37 missing bpm"))?;
        rest = tail;
        u16::from(v)
    } else {
        let (v, tail) = rest
            .split_first_chunk::<2>()
            .ok_or_else(|| framing("2A37 missing bpm"))?;
        rest = tail;
        u16::from_le_bytes(*v)
    };
    if flags & 0x08 != 0 {
        rest = rest
            .get(2..)
            .ok_or_else(|| framing("2A37 missing energy expended"))?;
    }
    let rr = if flags & 0x10 != 0 {
        if rest.len() % 2 != 0 {
            return Err(framing("2A37 RR field has odd length"));
        }
        rest
    } else {
        &[]
    };
    Ok(HrSample {
        at,
        bpm: u8::try_from(raw).ok().filter(|bpm| *bpm != 0),
        rr_ms: rr
            .chunks_exact(2)
            .map(|c| {
                let raw = u32::from(u16::from_le_bytes([c[0], c[1]]));
                u16::try_from((raw * 1000 + 512) / 1024).unwrap_or(u16::MAX)
            })
            .collect(),
        contact: (flags & 0x04 != 0).then_some(flags & 0x02 != 0),
    })
}

/// Heart-rate strap factory over an [`HrTransport`]. One live connection at a
/// time; wrap in [`crate::ReconnectingHr`] for the ADR-0002 §6 reconnect loop.
pub struct HrsDriver {
    transport: Arc<dyn HrTransport>,
    clock: Arc<dyn Clock>,
    connecting: Arc<tokio::sync::Mutex<()>>,
}
impl HrsDriver {
    /// Build a driver over btleplug or a test fake.
    pub fn new(transport: Arc<dyn HrTransport>, clock: Arc<dyn Clock>) -> Self {
        Self {
            transport,
            clock,
            connecting: Arc::default(),
        }
    }
}
impl HeartRateDriver for HrsDriver {
    fn connect(&self, dev: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>> {
        let (transport, clock, admission, dev) = (
            self.transport.clone(),
            self.clock.clone(),
            self.connecting.clone(),
            dev.clone(),
        );
        Box::pin(async move {
            let guard = admission
                .try_lock_owned()
                .map_err(|_| DeviceError::ConnectFailed("driver already connected".into()))?;
            let (info, mut notifications) = transport.connect(dev, clock).await?;
            let release = transport.clone();
            let cleanup: Cleanup = Box::new(move || release.disconnect());
            let mut guard = ConnectGuard(
                Some(cleanup),
                Some(guard),
                tokio::runtime::Handle::current(),
            );
            let (events, receiver) = mpsc::channel(64);
            let send = |event| async {
                events
                    .send(event)
                    .await
                    .map_err(|_| DeviceError::Disconnected)
            };
            // On failure `guard` disconnects the half-open peripheral.
            send(HrEvent::Link(LinkState::Connecting)).await?;
            send(HrEvent::Link(LinkState::Ready)).await?;
            let held = guard.1.take();
            guard.0 = None;
            tokio::spawn(async move {
                let _held = held;
                while let Some(record) = notifications.next().await {
                    let event = match record {
                        Ok((at, bytes)) => parse_heart_rate(at, &bytes)
                            .map_or_else(HrEvent::Fault, HrEvent::Sample),
                        Err(error) => {
                            let _ = events.send(HrEvent::Fault(error)).await;
                            break;
                        }
                    };
                    if events.send(event).await.is_err() {
                        break;
                    }
                }
                if let Err(error) = transport.disconnect().await {
                    let _ = events.send(HrEvent::Fault(error)).await;
                }
                let _ = events.send(HrEvent::Link(LinkState::Lost)).await;
            });
            Ok(HeartRateLink {
                info,
                events: receiver,
            })
        })
    }
}
