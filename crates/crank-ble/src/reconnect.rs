//! Reconnect supervisors (ADR-0002 §6): backoff 0.5, 1, 2, 4 s then every 5 s
//! until the consumer releases the device or drops the link.
//!
//! The supervisor owns the consumer-facing channels, so they survive a loss.
//! It forwards the inner driver's events (including `Link(Lost)`, then
//! `Reconnecting`, `Link(Connecting)`, `Link(Ready)` for each attempt) and
//! never sends a control mode itself: the engine re-issues `SetMode` on
//! `Ready` (one owner of control state). Rider configuration is re-applied
//! because every attempt calls the inner driver's `connect` with it.
use crate::Clock;
use crank_devices::{
    DeviceError, DeviceRef, HeartRateDriver, HeartRateLink, HrEvent, LinkState, MonoTime,
    RiderConfig, TrainerCommand, TrainerDriver, TrainerEvent, TrainerLink,
};
use futures_util::future::BoxFuture;
use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc;

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
/// After a loss the loop runs until `Disconnect`, or until the link is dropped.
pub struct Reconnecting<D> {
    inner: Arc<D>,
    clock: Arc<dyn Clock>,
}
impl<D: TrainerDriver> Reconnecting<D> {
    /// Wrap `inner`; `clock` times the backoff and must be the engine's clock.
    pub fn new(inner: D, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::new(inner),
            clock,
        }
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
        let (inner, clock, dev) = (self.inner.clone(), self.clock.clone(), dev.clone());
        Box::pin(async move {
            let link = inner.connect(&dev, rider).await?;
            let (info, caps) = (link.info.clone(), link.caps.clone());
            let (control, commands) = mpsc::channel(64);
            let (events_tx, events) = mpsc::channel(256);
            tokio::spawn(supervise_trainer(
                inner, clock, dev, rider, link, commands, events_tx,
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
async fn supervise_trainer<D: TrainerDriver>(
    inner: Arc<D>,
    clock: Arc<dyn Clock>,
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
                    if events.send(event).await.is_err() {
                        let _ = link.control.send(TrainerCommand::Disconnect).await;
                        return;
                    }
                    if lost { break }
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
        if releasing {
            return;
        }
        let mut attempt = 0;
        link = loop {
            attempt += 1;
            tokio::select! {
                () = pause(&clock, attempt) => {}
                () = released(&mut commands) => return,
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
                result = inner.connect(&dev, rider) => result,
                () = released(&mut commands) => return,
                () = events.closed() => return,
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
pub struct ReconnectingHr<D> {
    inner: Arc<D>,
    clock: Arc<dyn Clock>,
}
impl<D: HeartRateDriver> ReconnectingHr<D> {
    /// Wrap `inner`; `clock` times the backoff and must be the engine's clock.
    pub fn new(inner: D, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::new(inner),
            clock,
        }
    }
}
impl<D: HeartRateDriver> HeartRateDriver for ReconnectingHr<D> {
    fn connect(&self, dev: &DeviceRef) -> BoxFuture<'static, Result<HeartRateLink, DeviceError>> {
        let (inner, clock, dev) = (self.inner.clone(), self.clock.clone(), dev.clone());
        Box::pin(async move {
            let link = inner.connect(&dev).await?;
            let info = link.info.clone();
            let (events_tx, events) = mpsc::channel(64);
            tokio::spawn(supervise_hr(inner, clock, dev, link, events_tx));
            Ok(HeartRateLink { info, events })
        })
    }
}
async fn supervise_hr<D: HeartRateDriver>(
    inner: Arc<D>,
    clock: Arc<dyn Clock>,
    dev: DeviceRef,
    mut link: HeartRateLink,
    events: mpsc::Sender<HrEvent>,
) {
    loop {
        while let Some(event) = link.events.recv().await {
            let lost = event == HrEvent::Link(LinkState::Lost);
            if events.send(event).await.is_err() {
                return;
            }
            if lost {
                break;
            }
        }
        let mut attempt = 0;
        link = loop {
            attempt += 1;
            tokio::select! {
                () = pause(&clock, attempt) => {}
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
                result = inner.connect(&dev) => result,
                () = events.closed() => return,
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
