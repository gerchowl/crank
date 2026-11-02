//! The desktop file log (#411): the reconnect events of `crank-ble` reach `crank.log`,
//! with the capture directory and without it.
use crank_ble::{MonotonicClock, Reconnecting, RideGate};
use crank_desktop::logging::{LOG_FILE, LogPlan};
use crank_devices::{
    DeviceError, DeviceInfo, DeviceRef, DriverKind, LinkState, RiderConfig, SeqSource, TrainerCaps,
    TrainerDriver, TrainerEvent, TrainerLink,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use std::{future::Future, pin::Pin};
use tokio::sync::mpsc;

/// Hands out its scripted links, then refuses every connect.
struct Script(Mutex<VecDeque<TrainerLink>>);
impl TrainerDriver for Script {
    fn kind(&self) -> DriverKind {
        DriverKind::FeC
    }
    fn connect(
        &self,
        _: &DeviceRef,
        _: RiderConfig,
    ) -> Pin<Box<dyn Future<Output = Result<TrainerLink, DeviceError>> + Send>> {
        let next = self.0.lock().expect("script").pop_front();
        Box::pin(async move { next.ok_or(DeviceError::NotFound) })
    }
}

/// A trainer that connects, loses its link, and is then not found while the ride runs.
async fn lose_the_link_during_a_ride() {
    let (events_tx, events) = mpsc::channel(8);
    let (control, _commands) = mpsc::channel(8);
    let link = TrainerLink {
        info: DeviceInfo::default(),
        caps: TrainerCaps {
            erg: true,
            sim: true,
            resistance: true,
            max_resistance_n: None,
            max_target_w: 2000,
        },
        control,
        events,
        seq: SeqSource::default(),
    };
    let gate = RideGate::new(true);
    let driver = Reconnecting::new(
        Script(Mutex::new([link].into())),
        Arc::new(MonotonicClock::default()),
        gate.clone(),
    );
    let rider = RiderConfig {
        rider_kg: 75.0,
        bike_kg: 10.0,
        wheel_m: 0.7,
    };
    let mut link = driver
        .connect(&DeviceRef("t".into()), rider)
        .await
        .expect("first connect");
    events_tx
        .send(TrainerEvent::Link(LinkState::Lost))
        .await
        .expect("send");
    // The supervisor reports each attempt and each failure; the first failure is enough.
    while !matches!(link.events.recv().await, Some(TrainerEvent::Fault(_))) {}
    gate.set_active(false);
}

fn log_after_a_lost_link(plan: &LogPlan) -> String {
    let subscriber = plan.subscriber().expect("subscriber");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    let _installed = tracing::subscriber::set_default(subscriber);
    rt.block_on(lose_the_link_during_a_ride());
    std::fs::read_to_string(plan.log_dir.join(LOG_FILE)).expect("log file")
}

#[test]
fn reconnect_events_reach_the_log_file_of_a_capture_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = log_after_a_lost_link(&LogPlan::new(Some(dir.path()), dir.path().join("unused")));
    assert!(log.contains("reconnecting trainer"), "{log}");
    assert!(log.contains("trainer reconnect failed"), "{log}");
}

#[test]
fn reconnect_events_reach_the_default_log_file_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let default = dir.path().join("Logs/crank");
    let log = log_after_a_lost_link(&LogPlan::new(None, default.clone()));
    assert!(log.contains("reconnecting trainer"), "{log}");
    assert!(default.join(LOG_FILE).exists());
}
