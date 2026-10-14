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
/// Disconnects a half-open peripheral if setup is cancelled; shared with the HRS driver.
pub(crate) struct ConnectGuard(
    pub(crate) Option<Cleanup>,
    pub(crate) Option<tokio::sync::OwnedMutexGuard<()>>,
    pub(crate) tokio::runtime::Handle,
);
pub(crate) type Cleanup = Box<dyn FnOnce() -> BoxFuture<'static, Result<(), DeviceError>> + Send>;
impl Drop for ConnectGuard {
    fn drop(&mut self) {
        if let Some(cleanup) = self.0.take() {
            let guard = self.1.take();
            self.2.spawn(async move {
                let _guard = guard;
                if let Err(error) = cleanup().await {
                    tracing::warn!(%error, "cancelled connection cleanup failed");
                }
            });
        }
    }
}
/// FE-C trainer factory. Supply a separate transport for each live connection.
/// Cancelled setup retains admission until peripheral cleanup finishes.
/// Reconnection is a separate wrapper, [`crate::Reconnecting`] (ADR-0002 §6).
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
    /// Attach a TRACE capture. Diagnostic failures emit Fault and never stop BLE.
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
            let release = transport.clone();
            let mut cleanup = ConnectGuard(
                Some(Box::new(move || release.disconnect())),
                Some(guard),
                tokio::runtime::Handle::current(),
            );
            let (events, receiver) = mpsc::channel(256);
            let (commands, control) = mpsc::channel(64);
            let result = async {
                emit(&events, TrainerEvent::Link(LinkState::Connecting)).await?;
                write(&transport, &clock, capture.as_ref(), config, &events).await?;
                clock.sleep_until(MonoTime(clock.now().0 + 250_000)).await;
                write(&transport, &clock, capture.as_ref(), request(54)?, &events).await?;
                let deadline = MonoTime(clock.now().0 + 2_000_000);
                loop {
                    tokio::select! {
                        record = notifications.next() => {
                            let (at, bytes) = record.ok_or(DeviceError::Disconnected)??;
                            if let Err(error) = record_capture(capture.as_ref(), at, true, &bytes) {
                                events.try_send(TrainerEvent::Fault(error)).map_err(|e| DeviceError::Gatt(e.to_string()))?;
                            }
                            if let Ok(frame) = decode(&bytes)
                                && let Page::Capabilities(p) = frame.page {
                                    break Ok(TrainerCaps { erg: p.modes & 2 != 0, sim: p.modes & 4 != 0,
                                        resistance: p.modes & 1 != 0, max_resistance_n: (p.max_resistance_n != 0xffff).then_some(p.max_resistance_n), max_target_w: 16383 });
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
                    let disconnected = transport.disconnect().await;
                    cleanup.0 = None;
                    return Err(disconnected.err().map_or(error.clone(), |other| {
                        DeviceError::ConnectFailed(format!("{error}; disconnect failed: {other}"))
                    }));
                }
            };
            emit(&events, TrainerEvent::Link(LinkState::Ready)).await?;
            let guard = cleanup.1.take();
            cleanup.0 = None;
            let writer_transport = transport.clone();
            tokio::spawn(async move {
                let _guard = guard;
                let mut reader = tokio::spawn(read(notifications, events.clone(), capture.clone()));
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
    at: MonoTime,
    rx: bool,
    bytes: &[u8],
) -> Result<(), DeviceError> {
    if tracing::enabled!(tracing::Level::TRACE)
        && let Some(capture) = capture
    {
        capture
            .record(at, rx, bytes)
            .map_err(|e| DeviceError::Capture(e.to_string()))?;
    }
    Ok(())
}
async fn write(
    transport: &Arc<dyn FecTransport>,
    clock: &Arc<dyn Clock>,
    capture: Option<&Arc<Capture>>,
    bytes: [u8; 13],
    events: &mpsc::Sender<TrainerEvent>,
) -> Result<(), DeviceError> {
    if let Err(error) = record_capture(capture, clock.now(), false, &bytes) {
        emit(events, TrainerEvent::Fault(error)).await?;
    }
    transport.write(bytes).await
}
async fn read(
    mut notifications: Notifications,
    events: mpsc::Sender<TrainerEvent>,
    capture: Option<Arc<Capture>>,
) {
    while let Some(record) = notifications.next().await {
        match record {
            Ok((at, bytes)) => {
                if let Err(e) = record_capture(capture.as_ref(), at, true, &bytes) {
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
    let mut last_write = start;
    let mut next = start + 250_000;
    loop {
        clock.sleep_until(MonoTime(next)).await;
        let mut now = clock.now().0;
        if now < next {
            // An always-early clock still busy-loops, but yields cooperatively.
            // Such a misbehaving clock is a test or fault condition, not a real clock.
            tokio::task::yield_now().await;
            continue;
        }
        while now.saturating_sub(last_write) < 250_000 {
            clock
                .sleep_until(MonoTime(last_write.saturating_add(250_000)))
                .await;
            now = clock.now().0;
            tokio::task::yield_now().await;
        }
        tick = tick.max(
            now.saturating_sub(start)
                .saturating_div(250_000)
                .saturating_sub(1),
        );
        next += 250_000;
        next = next.max(start.saturating_add((tick + 2).saturating_mul(250_000)));
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
            last_write = clock.now().0;
            write(&transport, &clock, capture.as_ref(), frame, &events).await?;
            if frame[4] != 70
                && let Some(mode) = pacer.mode
            {
                emit(&events, TrainerEvent::Delivered { mode }).await?;
            }
        }
        tick += 1;
    }
}
