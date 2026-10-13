//! Connection lifecycle and a single paced writer (ADR-0002 §§4.4, 6).
use crate::{
    Capture,
    pacing::{Pacer, request},
    telemetry::receive,
};
use crank_devices::{
    Clock, DeviceError, DeviceInfo, DeviceRef, DriverKind, LinkState, MonoTime, ProtocolErrorKind,
    RiderConfig, TrainerCaps, TrainerCommand, TrainerDriver, TrainerEvent, TrainerLink,
};
use crank_fec::{ControlPage, Page, decode, encode};
use futures_util::{StreamExt, future::BoxFuture, stream::BoxStream};
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

/// Tokio monotonic clock; clone the same epoch into every driver and engine.
#[derive(Clone)]
pub struct MonotonicClock(tokio::time::Instant);
impl Default for MonotonicClock {
    fn default() -> Self {
        Self(tokio::time::Instant::now())
    }
}
impl Clock for MonotonicClock {
    fn now(&self) -> MonoTime {
        MonoTime(u64::try_from(self.0.elapsed().as_micros()).unwrap_or(u64::MAX))
    }
    fn sleep_until(&self, at: MonoTime) -> BoxFuture<'static, ()> {
        let deadline = self.0 + Duration::from_micros(at.0);
        Box::pin(tokio::time::sleep_until(deadline))
    }
}
/// Timestamped, complete GATT notification payloads, preserving arrival order.
pub type Notifications = BoxStream<'static, Result<(MonoTime, Vec<u8>), DeviceError>>;
/// Injectable FE-C transport; connect must subscribe before returning.
pub trait FecTransport: Send + Sync + 'static {
    /// Connect, discover, subscribe fec2, and read identity.
    fn connect(
        &self,
        dev: DeviceRef,
        clock: Arc<dyn Clock>,
    ) -> BoxFuture<'static, Result<(DeviceInfo, Notifications), DeviceError>>;
    /// Write one complete frame to fec3 with response.
    fn write(&self, bytes: [u8; 13]) -> BoxFuture<'static, Result<(), DeviceError>>;
    /// Release the peripheral.
    fn disconnect(&self) -> BoxFuture<'static, Result<(), DeviceError>>;
}
/// FE-C trainer factory. Supply a separate transport for each live connection.
/// No discovery/reconnect loop is implemented here (#147).
pub struct FecTrainer {
    transport: Arc<dyn FecTransport>,
    clock: Arc<dyn Clock>,
    capture: Option<Arc<Capture>>,
    connecting: Arc<tokio::sync::Mutex<()>>,
}
impl FecTrainer {
    /// Build a driver over btleplug, a test fake, or replay.
    pub fn new(transport: Arc<dyn FecTransport>, clock: Arc<dyn Clock>) -> Self {
        Self {
            transport,
            clock,
            capture: None,
            connecting: Arc::default(),
        }
    }
    /// Attach a per-ride capture opened by the ride owner; writes occur at TRACE.
    #[must_use]
    pub fn with_capture(mut self, capture: Arc<Capture>) -> Self {
        self.capture = Some(capture);
        self
    }
}
impl TrainerDriver for FecTrainer {
    fn kind(&self) -> DriverKind {
        DriverKind::FeC
    }
    fn connect(
        &self,
        dev: &DeviceRef,
        rider: RiderConfig,
    ) -> BoxFuture<'static, Result<TrainerLink, DeviceError>> {
        let (transport, clock, capture, guard, dev) = (
            self.transport.clone(),
            self.clock.clone(),
            self.capture.clone(),
            self.connecting.clone(),
            dev.clone(),
        );
        Box::pin(async move {
            let guard = guard
                .try_lock_owned()
                .map_err(|_| DeviceError::ConnectFailed("driver already connected".into()))?;
            let config = encode(&ControlPage::UserConfig {
                rider_kg: rider.rider_kg,
                bike_kg: rider.bike_kg,
                wheel_m: rider.wheel_m,
            })
            .map_err(|e| crate::protocol(e, ProtocolErrorKind::Encode))?;
            let (info, mut notifications) = transport.connect(dev, clock.clone()).await?;
            let (events, receiver) = mpsc::channel(256);
            let (commands, control) = mpsc::channel(64);
            let result = async {
                emit(&events, TrainerEvent::Link(LinkState::Connecting)).await?;
                write(&transport, &clock, capture.as_ref(), config).await?;
                clock.sleep_until(MonoTime(clock.now().0 + 250_000)).await;
                write(&transport, &clock, capture.as_ref(), request(54)?).await?;
                let deadline = MonoTime(clock.now().0 + 2_000_000);
                loop {
                    tokio::select! {
                        record = notifications.next() => {
                            let (at, bytes) = record.ok_or(DeviceError::Disconnected)??;
                            record_capture(capture.as_ref(), &clock, true, &bytes)?;
                            if let Ok(frame) = decode(&bytes)
                                && let Page::Capabilities(p) = frame.page {
                                    break Ok(TrainerCaps { erg: p.modes & 2 != 0, sim: p.modes & 4 != 0,
                                        resistance: p.modes & 1 != 0, max_resistance_n: p.max_resistance_n, max_target_w: 16383 });
                                }
                            for event in receive(at, &bytes) { events.try_send(event).map_err(|e| DeviceError::Gatt(e.to_string()))?; }
                        }
                        () = clock.sleep_until(deadline) => break Err(crate::protocol("page 54 timed out", ProtocolErrorKind::UnexpectedPage)),
                    }
                }
            }.await;
            let caps = match result {
                Ok(caps) => caps,
                Err(error) => {
                    transport.disconnect().await?;
                    return Err(error);
                }
            };
            emit(&events, TrainerEvent::Link(LinkState::Ready)).await?;
            let writer_transport = transport.clone();
            tokio::spawn(async move {
                let _guard = guard;
                let mut reader = tokio::spawn(read(
                    notifications,
                    events.clone(),
                    capture.clone(),
                    clock.clone(),
                ));
                let result = tokio::select! {
                    result = run(control, events.clone(), transport.clone(), clock, capture) => result,
                    result = &mut reader => result.map_err(|e| DeviceError::Gatt(e.to_string())),
                };
                if !reader.is_finished() {
                    reader.abort();
                    let _ = reader.await;
                }
                if let Err(error) = result {
                    let _ = emit(&events, TrainerEvent::Fault(error)).await;
                }
                if let Err(error) = writer_transport.disconnect().await {
                    let _ = emit(&events, TrainerEvent::Fault(error)).await;
                }
                let _ = emit(&events, TrainerEvent::Link(LinkState::Lost)).await;
            });
            Ok(TrainerLink {
                info,
                caps,
                control: commands,
                events: receiver,
            })
        })
    }
}
async fn emit(events: &mpsc::Sender<TrainerEvent>, event: TrainerEvent) -> Result<(), DeviceError> {
    events
        .send(event)
        .await
        .map_err(|_| DeviceError::Disconnected)
}
fn record_capture(
    capture: Option<&Arc<Capture>>,
    clock: &Arc<dyn Clock>,
    rx: bool,
    bytes: &[u8],
) -> Result<(), DeviceError> {
    if tracing::enabled!(tracing::Level::TRACE)
        && let Some(capture) = capture
    {
        capture
            .record_current(clock.as_ref(), rx, bytes)
            .map_err(|e| DeviceError::Gatt(e.to_string()))?;
    }
    Ok(())
}
async fn write(
    transport: &Arc<dyn FecTransport>,
    clock: &Arc<dyn Clock>,
    capture: Option<&Arc<Capture>>,
    bytes: [u8; 13],
) -> Result<(), DeviceError> {
    record_capture(capture, clock, false, &bytes)?;
    transport.write(bytes).await
}
async fn read(
    mut notifications: Notifications,
    events: mpsc::Sender<TrainerEvent>,
    capture: Option<Arc<Capture>>,
    clock: Arc<dyn Clock>,
) {
    while let Some(record) = notifications.next().await {
        match record {
            Ok((at, bytes)) => {
                if let Err(e) = record_capture(capture.as_ref(), &clock, true, &bytes) {
                    let _ = emit(&events, TrainerEvent::Fault(e)).await;
                }
                for event in receive(at, &bytes) {
                    if emit(&events, event).await.is_err() {
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = emit(&events, TrainerEvent::Fault(error)).await;
                break;
            }
        }
    }
}
async fn run(
    mut commands: mpsc::Receiver<TrainerCommand>,
    events: mpsc::Sender<TrainerEvent>,
    transport: Arc<dyn FecTransport>,
    clock: Arc<dyn Clock>,
    capture: Option<Arc<Capture>>,
) -> Result<(), DeviceError> {
    let mut pacer = Pacer::default();
    let mut tick = 0;
    let start = clock.now().0;
    let mut next = start + 250_000;
    loop {
        clock.sleep_until(MonoTime(next)).await;
        tick = tick.max((clock.now().0 - start) / 250_000 - 1);
        next = clock.now().0 + 250_000;
        for _ in 0..commands.len() {
            let Ok(command) = commands.try_recv() else {
                break;
            };
            match command {
                TrainerCommand::SetMode(mode) => {
                    if let Err(error) = pacer.set(mode, tick) {
                        emit(&events, TrainerEvent::Fault(error)).await?;
                    }
                }
                TrainerCommand::SetRider(_) => {
                    emit(
                        &events,
                        TrainerEvent::Fault(crate::protocol(
                            "rider changes require a new connection (#43)",
                            ProtocolErrorKind::UnexpectedPage,
                        )),
                    )
                    .await?;
                }
                TrainerCommand::Disconnect => return Ok(()),
            }
        }
        if commands.is_closed() || events.is_closed() {
            return Ok(());
        }
        if let Some(frame) = pacer.tick(tick)? {
            write(&transport, &clock, capture.as_ref(), frame).await?;
            if frame[4] != 70
                && let Some(mode) = pacer.mode
            {
                emit(&events, TrainerEvent::Delivered { mode }).await?;
            }
        }
        tick += 1;
    }
}
