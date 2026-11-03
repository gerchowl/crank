//! The desktop log (#411, ADR-0001 §7.1): what reaches vigil's `logs.jsonl`, with the
//! capture directory and without it. vigil allows one pipeline per process, so each test
//! installs its own: run them with nextest (one process per test), as `just test` and CI do.
use crank_ble::{MonotonicClock, Reconnecting, RideGate};
use crank_desktop::logging::{LOG_FILE, LogError, LogPlan, Logging};
use crank_devices::{
    DeviceError, DeviceInfo, DeviceRef, DriverKind, LinkState, RiderConfig, SeqSource, TrainerCaps,
    TrainerDriver, TrainerEvent, TrainerLink,
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
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

/// Every log line in `dir`, rotated segments included.
fn read(dir: &Path) -> String {
    vigil::rotate::read_since(dir, "logs", None)
        .expect("logs")
        .join("\n")
}

fn log_after_a_lost_link(plan: &LogPlan) -> String {
    let logging = Logging::new(plan.install(None).expect("logging starts"));
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("runtime");
    rt.block_on(lose_the_link_during_a_ride());
    logging.shut_down(Duration::from_secs(5));
    read(&plan.log_dir)
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

/// A capture run with `CRANK_LOG=crank_log`: what a TRACE record of the driver and a DEBUG
/// record of the core leave in the log.
fn capture_with(crank_log: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = LogPlan::new(Some(dir.path()), dir.path().join("unused"));
    let logging = Logging::new(plan.install(Some(crank_log)).expect("logging starts"));
    tracing::trace!(target: "crank_ble::driver", "driver-trace-marker");
    tracing::debug!(target: "crank_core", "core-debug-marker");
    logging.shut_down(Duration::from_secs(5));
    read(dir.path())
}

#[test]
fn crank_log_warn_keeps_the_capture_at_trace() {
    let log = capture_with("warn");
    assert!(log.contains("driver-trace-marker"), "{log}");
    assert!(!log.contains("core-debug-marker"), "{log}");
}

#[test]
fn crank_log_cannot_lower_a_crank_ble_module_and_says_so() {
    let log = capture_with("crank_core=debug,crank_ble::driver=warn");
    assert!(log.contains("driver-trace-marker"), "{log}");
    assert!(log.contains("core-debug-marker"), "{log}");
    assert!(
        log.contains("crank_ble::driver=warn"),
        "the dropped directive is reported: {log}"
    );
}

/// A rotated `logs` segment last written 30 days ago, in vigil's layout.
fn old_segment(dir: &Path) -> PathBuf {
    let path = dir.join("logs-20200101T000000.000000000Z.jsonl");
    let file = std::fs::File::create(&path).expect("segment");
    file.set_modified(SystemTime::now() - Duration::from_hours(30 * 24))
        .expect("mtime");
    path
}

#[test]
fn a_capture_directory_keeps_every_segment_however_old() {
    let dir = tempfile::tempdir().expect("tempdir");
    let old = old_segment(dir.path());
    let plan = LogPlan::new(Some(dir.path()), dir.path().join("unused"));
    let logging = Logging::new(plan.install(None).expect("logging starts"));
    tracing::info!("capture-marker");
    logging.shut_down(Duration::from_secs(5));
    assert!(old.exists(), "a capture prunes nothing");
    assert!(read(dir.path()).contains("capture-marker"));
}

#[test]
fn a_normal_launch_prunes_segments_past_seven_days() {
    let dir = tempfile::tempdir().expect("tempdir");
    let old = old_segment(dir.path());
    let plan = LogPlan::new(None, dir.path().into());
    let logging = Logging::new(plan.install(None).expect("logging starts"));
    tracing::info!("normal-marker");
    logging.shut_down(Duration::from_secs(5));
    assert!(!old.exists(), "retention removed the 30-day-old segment");
}

#[test]
fn a_capture_directory_that_cannot_be_written_refuses_to_start() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("a-file");
    std::fs::write(&file, "").expect("file");
    let plan = LogPlan::new(Some(&file.join("capture")), dir.path().join("unused"));
    assert!(matches!(plan.install(None), Err(LogError::NoDirectory(_))));
}

#[test]
fn records_flushed_when_a_ride_ends_survive_a_quit_with_no_budget_left() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = LogPlan::new(None, dir.path().into());
    let logging = Logging::new(plan.install(None).expect("logging starts"));
    tracing::info!("ride-marker");
    logging.flush();
    // What is logged after the flush may be lost: the quit used up `QUIT_CAP`.
    tracing::info!("after-the-flush");
    logging.shut_down(Duration::ZERO);
    assert!(read(dir.path()).contains("ride-marker"));
}

/// The complete records of `dir`: a line that does not parse is skipped, never guessed at (the
/// rule of squelch, ADR-0003 §2.2).
fn complete_lines(dir: &Path) -> Vec<serde_json::Value> {
    vigil::rotate::read_since(dir, "logs", None)
        .expect("a partial last line is no read error")
        .iter()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// A quit that gives up on the flush, or a SIGKILL, can leave `logs.jsonl` ending in half a line
/// (ADR-0001 §7.1, #465): readers take the complete lines and skip that one.
#[test]
fn a_partial_last_line_costs_a_reader_only_that_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = LogPlan::new(None, dir.path().into());
    let logging = Logging::new(plan.install(None).expect("logging starts"));
    tracing::info!("whole-marker");
    logging.shut_down(Duration::from_secs(5));
    let before = complete_lines(dir.path());
    assert!(!before.is_empty());
    let mut live = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.path().join(LOG_FILE))
        .expect("live file");
    std::io::Write::write_all(&mut live, br#"{"resourceLogs":[{"scopeLo"#).expect("append");
    assert_eq!(
        complete_lines(dir.path()),
        before,
        "only the torn line is lost"
    );
    assert!(read(dir.path()).contains("whole-marker"));
}
