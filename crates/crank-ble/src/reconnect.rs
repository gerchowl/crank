//! Reconnect supervisors (ADR-0002 §6): backoff 0.5, 1, 2, 4 s then every 5 s
//! until the consumer releases the device or drops the link.
//!
//! The supervisor owns the consumer-facing channels, so they survive a loss.
//! It forwards the inner driver's events (including `Link(Lost)`, then
//! `Reconnecting`, `Link(Connecting)`, `Link(Ready)` for each attempt) and
//! never sends a control mode itself: the engine re-issues `SetMode` on
//! `Ready` (one owner of control state). Rider configuration is re-applied
//! because every attempt calls the inner driver's `connect` with it.
use crate::{Clock, ConnectCleanup};
use crank_devices::{
    DeviceError, DeviceRef, HeartRateDriver, HeartRateLink, HrEvent, LinkState, MonoTime,
    RiderConfig, TrainerCommand, TrainerDriver, TrainerEvent, TrainerLink,
};
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

/// Explicit "ride is active" input of the reconnect supervisors (ADR-0002 §6).
///
/// The link may be held while the app is open (§5), but the backoff loop runs
/// only while the gate is active. Outside a ride a loss is surfaced
/// (`Link(Lost)`, then the event stream closes) and normal rediscovery
/// reconnects when the device is seen again. If the gate goes inactive during
/// backoff the loop stops the same way. Clones share one state. Driving it
/// from the engine/core is tracked in #208 (#32/#34).
#[derive(Clone)]
pub struct RideGate(Arc<watch::Sender<bool>>);
impl RideGate {
    /// A gate that starts as given; share clones with every wrapper.
    #[must_use]
    pub fn new(active: bool) -> Self {
        Self(Arc::new(watch::channel(active).0))
    }
    /// Set whether a ride is loaded / the ride view is open.
    pub fn set_active(&self, active: bool) {
        self.0.send_replace(active);
    }
    /// Current state.
    #[must_use]
    pub fn is_active(&self) -> bool {
        *self.0.borrow()
    }
    /// Resolves once the gate is (or becomes) inactive.
    async fn inactive(&self) {
        let mut rx = self.0.subscribe();
        // `wait_for` only errs if the sender drops, which `self` prevents.
        let _ = rx.wait_for(|active| !*active).await;
    }
}

/// Delay before reconnect attempt `attempt` (1-based): 0.5, 1, 2, 4, then 5 s.
#[must_use]
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(match attempt {
        0 | 1 => 500,
        2 => 1_000,
        3 => 2_000,
        4 => 4_000,
        _ => 5_000,
    })
}
async fn pause(clock: &Arc<dyn Clock>, attempt: u32) {
    let micros = u64::try_from(backoff(attempt).as_micros()).unwrap_or(u64::MAX);
    clock
        .sleep_until(MonoTime(clock.now().0.saturating_add(micros)))
        .await;
}

/// Adds the ADR-0002 §6 reconnect loop to any [`TrainerDriver`]. The first
/// connect is not retried: its error reaches the caller (UI "not found").
/// After a loss the loop runs while the [`RideGate`] is active, until
/// `Disconnect`, or until the link is dropped.
pub struct Reconnecting<D> {
    inner: Arc<D>,
    clock: Arc<dyn Clock>,
    gate: RideGate,
    cleanup: ConnectCleanup,
}
impl<D: TrainerDriver> Reconnecting<D> {
    /// Wrap `inner`; `clock` times the backoff and must be the engine's clock.
    pub fn new(inner: D, clock: Arc<dyn Clock>, gate: RideGate) -> Self {
        Self {
            inner: Arc::new(inner),
            clock,
            gate,
            cleanup: ConnectCleanup::default(),
        }
    }
    /// Wait for `cleanup` before a shutdown that cancelled a reconnect attempt reports the
    /// link as released: pass the tracker the inner driver reports its cancelled connects
    /// to, so "released" means the peripheral was disconnected (ADR-0002 §7.9).
    #[must_use]
    pub fn with_cleanup(mut self, cleanup: ConnectCleanup) -> Self {
        self.cleanup = cleanup;
        self
    }
}
impl<D: TrainerDriver> TrainerDriver for Reconnecting<D> {
    fn kind(&self) -> crank_devices::DriverKind {
        self.inner.kind()
    }
    fn connect(
        &self,
        dev: &DeviceRef,
        rider: RiderConfig,
    ) -> BoxFuture<'static, Result<TrainerLink, DeviceError>> {
        let (inner, clock, dev, gate, cleanup) = (
            self.inner.clone(),
            self.clock.clone(),
            dev.clone(),
            self.gate.clone(),
            self.cleanup.clone(),
        );
        Box::pin(async move {
            let link = inner.connect(&dev, rider).await?;
            let (info, caps) = (link.info.clone(), link.caps.clone());
            let (control, commands) = mpsc::channel(64);
            let (events_tx, events) = mpsc::channel(256);
            tokio::spawn(supervise_trainer(
                inner, clock, gate, cleanup, dev, rider, link, commands, events_tx,
            ));
            Ok(TrainerLink {
                info,
                caps,
                control,
                events,
            })
        })
    }
}
#[allow(clippy::too_many_arguments)]
async fn supervise_trainer<D: TrainerDriver>(
    inner: Arc<D>,
    clock: Arc<dyn Clock>,
    gate: RideGate,
    cleanup: ConnectCleanup,
    dev: DeviceRef,
    rider: RiderConfig,
    mut link: TrainerLink,
    mut commands: mpsc::Receiver<TrainerCommand>,
    events: mpsc::Sender<TrainerEvent>,
) {
    loop {
        // Pump until the link is lost; `releasing` means the consumer released it.
        let mut releasing = false;
        loop {
            tokio::select! {
                event = link.events.recv() => {
                    let Some(event) = event else { break };
                    let lost = event == TrainerEvent::Link(LinkState::Lost);
                    // A consumer that is gone releases the inner link like `Disconnect`;
                    // the pump then runs on until the inner stream ends, i.e. until the
                    // inner link has actually disconnected, so that the consumer's end of
                    // this stream means "released".
                    if !releasing && events.send(event).await.is_err() {
                        releasing = true;
                        let _ = link.control.send(TrainerCommand::Disconnect).await;
                    }
                    if lost { break }
                }
                // The consumer closed or dropped the event stream without releasing.
                () = events.closed(), if !releasing => {
                    releasing = true;
                    let _ = link.control.send(TrainerCommand::Disconnect).await;
                }
                command = commands.recv(), if !releasing => match command {
                    None | Some(TrainerCommand::Disconnect) => {
                        releasing = true;
                        let _ = link.control.send(TrainerCommand::Disconnect).await;
                    }
                    Some(command) => {
                        let _ = link.control.send(command).await;
                    }
                },
            }
        }
        if releasing || !gate.is_active() {
            return; // Outside a ride the loss is surfaced; rediscovery reconnects.
        }
        let mut attempt = 0;
        link = loop {
            attempt += 1;
            tokio::select! {
                () = pause(&clock, attempt) => {}
                () = released(&mut commands) => return,
                () = gate.inactive() => return,
                () = events.closed() => return,
            }
            if events
                .send(TrainerEvent::Reconnecting { attempt })
                .await
                .is_err()
            {
                return;
            }
            tracing::info!(attempt, "reconnecting trainer");
            let connected = tokio::select! {
                result = inner.connect(&dev, rider) => Some(result),
                () = released(&mut commands) => None,
                () = gate.inactive() => None,
                () = events.closed() => None,
            };
            // Cancelled mid-connect: the dropped attempt disconnects its peripheral in the
            // background; "released" (this task ending) waits for that.
            let Some(connected) = connected else {
                cleanup.settled().await;
                return;
            };
            match connected {
                Ok(link) => break link,
                Err(error) => {
                    tracing::warn!(attempt, %error, "trainer reconnect failed");
                    if events.send(TrainerEvent::Fault(error)).await.is_err() {
                        return;
                    }
                }
            }
        };
    }
}
/// Resolves when the consumer releases or drops the link. Other commands that
/// arrive while disconnected are discarded: the engine re-issues its mode (and
/// any rider change) after the next `Ready`, and a stale `SetMode` must not
/// reach a trainer the engine has not re-activated (ADR-0002 §6).
async fn released(commands: &mut mpsc::Receiver<TrainerCommand>) {
    loop {
        match commands.recv().await {
            None | Some(TrainerCommand::Disconnect) => return,
            Some(discarded) => tracing::debug!(?discarded, "command discarded while disconnected"),
        }
    }
}

/// Adds the ADR-0002 §6 reconnect loop to any [`HeartRateDriver`]. The ride
/// does not pause on strap dropout; the consumer sees `Link(Lost)`, then
/// `Reconnecting`, and drops the link to stop (there is no command channel).
/// Like [`Reconnecting`] it retries only while the [`RideGate`] is active.
pub struct ReconnectingHr<D> {
    inner: Arc<D>,
    clock: Arc<dyn Clock>,
    gate: RideGate,
    cleanup: ConnectCleanup,
}
impl<D: HeartRateDriver> ReconnectingHr<D> {
    /// Wrap `inner`; `clock` times the backoff and must be the engine's clock.
    pub fn new(inner: D, clock: Arc<dyn Clock>, gate: RideGate) -> Self {
        Self {
            inner: Arc::new(inner),
            clock,
            gate,
            cleanup: ConnectCleanup::default(),
        }
    }
    /// Wait for `cleanup` before a shutdown that cancelled a reconnect attempt reports the
    /// link as released: pass the tracker the inner driver reports its cancelled connects
    /// to, so "released" means the peripheral was disconnected (ADR-0002 §7.9).
    #[must_use]
    pub fn with_cleanup(mut self, cleanup: ConnectCleanup) -> Self {
        self.cleanup = cleanup;
        self
    }
}
impl<D: HeartRateDriver> HeartRateDriver for ReconnectingHr<D> {
    fn connect(&self, dev: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>> {
        let (inner, clock, dev, gate, cleanup) = (
            self.inner.clone(),
            self.clock.clone(),
            dev.clone(),
            self.gate.clone(),
            self.cleanup.clone(),
        );
        Box::pin(async move {
            let link = inner.connect(&dev).await?;
            let info = link.info.clone();
            let (events_tx, events) = mpsc::channel(64);
            tokio::spawn(supervise_hr(
                inner, clock, gate, cleanup, dev, link, events_tx,
            ));
            Ok(HeartRateLink { info, events })
        })
    }
}
/// The consumer of the strap is gone: the inner driver disconnects when it learns so, and
/// this supervisor keeps its own stream open until it has (dropping `events` is what the
/// consumer reads as "the strap is released").
async fn release_inner(link: &mut HeartRateLink) {
    crank_devices::release_events(&mut link.events).await;
}
async fn supervise_hr<D: HeartRateDriver>(
    inner: Arc<D>,
    clock: Arc<dyn Clock>,
    gate: RideGate,
    cleanup: ConnectCleanup,
    dev: DeviceRef,
    mut link: HeartRateLink,
    events: mpsc::Sender<HrEvent>,
) {
    loop {
        loop {
            let event = tokio::select! {
                event = link.events.recv() => event,
                () = events.closed() => {
                    release_inner(&mut link).await;
                    return;
                }
            };
            let Some(event) = event else { break };
            let lost = event == HrEvent::Link(LinkState::Lost);
            if events.send(event).await.is_err() {
                release_inner(&mut link).await;
                return;
            }
            if lost {
                break;
            }
        }
        if !gate.is_active() {
            return;
        }
        let mut attempt = 0;
        link = loop {
            attempt += 1;
            tokio::select! {
                () = pause(&clock, attempt) => {}
                () = gate.inactive() => return,
                () = events.closed() => return,
            }
            if events
                .send(HrEvent::Reconnecting { attempt })
                .await
                .is_err()
            {
                return;
            }
            tracing::info!(attempt, "reconnecting heart-rate strap");
            let connected = tokio::select! {
                result = inner.connect(&dev) => Some(result),
                () = gate.inactive() => None,
                () = events.closed() => None,
            };
            let Some(connected) = connected else {
                cleanup.settled().await;
                return;
            };
            match connected {
                Ok(link) => break link,
                Err(error) => {
                    tracing::warn!(attempt, %error, "strap reconnect failed");
                    if events.send(HrEvent::Fault(error)).await.is_err() {
                        return;
                    }
                }
            }
        };
    }
}
