//! Facade acceptance (#34, ADR-0001 §4) on the simulator and a virtual clock: paused
//! tokio time, so whole rides take no wall time.
use crank_core::{
    ApiError, Core, CoreConfig, DeviceMode, DeviceRole, ErrorCode, RideOutcome, SinkClosed,
};
use crank_devices::sim::{SimConfig, SimFault};
use crank_engine::stream::types::StreamPauseReason;
use crank_engine::{
    LinkChange, LinkDevice, LinkStatus, MetricId, StreamFrame, StreamId, StreamRideState,
    SubscribeOpts, VirtualClock,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::sleep;

const SENSOR_CHECK: &str = "builtin-sensor-check";

fn config(dir: &Path, faults: Vec<SimFault>) -> CoreConfig {
    CoreConfig {
        data_dir: dir.into(),
        devices: DeviceMode::Sim(SimConfig {
            faults,
            ..SimConfig::default()
        }),
        clock: Some(Arc::new(VirtualClock::new(1))),
    }
}
type Frames = Arc<Mutex<Vec<StreamFrame>>>;
fn sink(frames: &Frames) -> impl Fn(StreamFrame) -> Result<(), SinkClosed> + Send + 'static {
    let frames = frames.clone();
    move |f| {
        frames.lock().expect("test mutex").push(f);
        Ok(())
    }
}
fn opts() -> SubscribeOpts {
    SubscribeOpts {
        rate_hz: 1.0,
        filters: Vec::new(),
    }
}
async fn pair_both(core: &Core) {
    let found = core.devices_scan(None).await.expect("scan");
    let trainer = found.iter().find(|d| d.trainer).expect("sim trainer");
    let strap = found.iter().find(|d| d.heart_rate).expect("sim strap");
    core.device_pair(trainer.id.clone(), DeviceRole::Trainer)
        .await
        .expect("pair trainer");
    core.device_pair(strap.id.clone(), DeviceRole::HeartRate)
        .await
        .expect("pair strap");
}
/// Ack what has arrived, as the UI does, so the flow-control window never stalls.
async fn ack_all(core: &Core, stream: StreamId, frames: &Frames) {
    let last = frames.lock().expect("test mutex").last().map(|f| f.seq);
    if let Some(seq) = last {
        core.stream_ack(stream, seq).await.expect("ack");
    }
}
async fn run_for(core: &Core, stream: StreamId, frames: &Frames, seconds: u64) {
    for _ in 0..seconds {
        sleep(Duration::from_secs(1)).await;
        ack_all(core, stream, frames).await;
    }
}
fn code(e: &ApiError) -> ErrorCode {
    e.code
}

#[tokio::test(start_paused = true)]
async fn scan_pair_load_ride_pause_resume_skip_end_then_history() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path(), vec![])).unwrap();
    assert!(core.core_ready().await.unwrap().sim);

    // Discover and pair; pairing releases the devices, so a second scan still works.
    let found = core.devices_scan(None).await.unwrap();
    assert_eq!(found.len(), 2);
    assert!(found.iter().all(|d| !d.paired));
    pair_both(&core).await;
    assert!(
        core.devices_scan(None)
            .await
            .unwrap()
            .iter()
            .all(|d| d.paired)
    );
    let links = core.device_links().await.unwrap();
    assert!(links.iter().all(|l| l.device.is_some() && l.link.is_none()));

    // Workouts.
    let list = core.workouts_list().await.unwrap();
    assert!(list.iter().any(|w| w.id == SENSOR_CHECK));
    let detail = core.workout_get(SENSOR_CHECK.into()).await.unwrap();
    assert_eq!(detail.compiled.blocks.len(), 3);

    // Load, stream, start.
    let loaded = core.ride_load(Some(SENSOR_CHECK.into())).await.unwrap();
    assert!(loaded.hr_linked);
    let frames: Frames = Arc::default();
    let stream = core
        .open_stream(Some("main".into()), sink(&frames))
        .await
        .unwrap();
    core.subscribe(stream, MetricId::Power, opts())
        .await
        .unwrap();
    core.ride_start().await.unwrap();
    run_for(&core, stream, &frames, 20).await;
    let status = core.ride_status().await.unwrap();
    assert_eq!(status.state, StreamRideState::Riding);
    assert_eq!(status.block_index, Some(0));
    let links = core.device_links().await.unwrap();
    assert!(links.iter().all(|l| l.link == Some(LinkStatus::Ready)));

    // Pause and resume: workout time freezes, session time runs.
    core.ride_pause().await.unwrap();
    run_for(&core, stream, &frames, 1).await;
    let paused = core.ride_status().await.unwrap();
    assert_eq!(
        paused.state,
        StreamRideState::Paused {
            reason: StreamPauseReason::User
        }
    );
    run_for(&core, stream, &frames, 5).await;
    let later = core.ride_status().await.unwrap();
    assert!((later.workout_s - paused.workout_s).abs() < f64::EPSILON);
    assert!(later.session_s > paused.session_s);
    core.ride_resume().await.unwrap();
    run_for(&core, stream, &frames, 1).await;

    // Skip, then end.
    core.ride_skip().await.unwrap();
    run_for(&core, stream, &frames, 3).await;
    assert_eq!(core.ride_status().await.unwrap().block_index, Some(1));
    let snapshot = core.stream_snapshot(stream).await.unwrap();
    assert!(snapshot.status.is_some());
    let ride_id = core.ride_end().await.unwrap().expect("a finished ride");
    assert_eq!(
        core.ride_status().await.unwrap().state,
        StreamRideState::Finished
    );

    // History.
    let rides = core.rides_list().await.unwrap();
    assert_eq!(rides.len(), 1);
    let one = core.ride_summary(ride_id.clone()).await.unwrap();
    assert_eq!(one, rides[0]);
    assert_eq!(one.outcome, RideOutcome::Finished);
    assert_eq!(one.workout_id.as_deref(), Some(SENSOR_CHECK));
    let summary = one.summary.expect("summary written at finish");
    assert!(summary.duration_s >= 25.0, "{summary:?}");
    assert!(summary.avg_power_w.unwrap() > 50.0);
    assert!(summary.avg_hr_bpm.unwrap() > 40.0);
    assert!(summary.kj > 0.0);
    assert_eq!(summary.blocks.len(), 3);
    assert!(summary.blocks[0].actual_w.is_some());
    assert_eq!(
        core.ride_summary("nope".into()).await.unwrap_err().code,
        ErrorCode::NotFound
    );

    // The devices were released: a new ride can be loaded straight away.
    core.ride_load(None).await.unwrap();
    assert_eq!(
        core.ride_end().await.unwrap(),
        None,
        "never started: unloaded"
    );
}

#[tokio::test(start_paused = true)]
async fn a_whole_workout_finishes_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path(), vec![])).unwrap();
    pair_both(&core).await;
    core.ride_load(Some(SENSOR_CHECK.into())).await.unwrap();
    core.ride_start().await.unwrap();
    sleep(Duration::from_secs(185)).await;
    assert_eq!(
        core.ride_status().await.unwrap().state,
        StreamRideState::Finished
    );
    let id = core.ride_end().await.unwrap().expect("finished");
    let s = core.ride_summary(id).await.unwrap().summary.unwrap();
    assert!((s.duration_s - 180.0).abs() <= 2.0, "{s:?}");
    assert!(s.blocks.iter().all(|b| b.actual_w.is_some()));
}

#[tokio::test(start_paused = true)]
async fn a_dropout_shows_loss_then_reconnect_on_the_stream() {
    let dir = tempfile::tempdir().unwrap();
    let faults = vec![SimFault::Dropout {
        at: Duration::from_secs(20),
        down: Duration::from_secs(8),
    }];
    let core = Core::open(config(dir.path(), faults)).unwrap();
    pair_both(&core).await;
    core.ride_load(Some("builtin-endurance-45".into()))
        .await
        .unwrap();
    let frames: Frames = Arc::default();
    let stream = core.open_stream(None, sink(&frames)).await.unwrap();
    core.ride_start().await.unwrap();
    run_for(&core, stream, &frames, 60).await;

    let seen = frames.lock().expect("test mutex");
    let trainer_links: Vec<LinkStatus> = seen
        .iter()
        .flat_map(|f| &f.links)
        .filter(|l: &&LinkChange| l.device == LinkDevice::Trainer)
        .map(|l| l.state)
        .collect();
    let lost = trainer_links.iter().position(|s| *s == LinkStatus::Lost);
    let ready_after = lost.and_then(|i| {
        trainer_links[i..]
            .iter()
            .position(|s| *s == LinkStatus::Ready)
    });
    assert!(lost.is_some() && ready_after.is_some(), "{trainer_links:?}");
    let states: Vec<_> = seen
        .iter()
        .filter_map(|f| f.status.as_ref())
        .map(|s| s.state)
        .collect();
    assert!(states.contains(&StreamRideState::Paused {
        reason: StreamPauseReason::TrainerLost
    }));
    assert_eq!(states.last(), Some(&StreamRideState::Riding));
}

#[tokio::test(start_paused = true)]
async fn commands_in_the_wrong_state_are_typed_errors() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path(), vec![])).unwrap();
    // No ride loaded.
    for e in [
        core.ride_start().await.unwrap_err(),
        core.ride_pause().await.unwrap_err(),
        core.ride_end().await.unwrap_err(),
    ] {
        assert_eq!(code(&e), ErrorCode::Conflict);
    }
    let e = core.open_stream(None, |_| Ok(())).await.unwrap_err();
    assert_eq!(code(&e), ErrorCode::Conflict);
    // No trainer paired.
    assert_eq!(
        code(&core.ride_load(None).await.unwrap_err()),
        ErrorCode::DeviceUnavailable
    );
    assert_eq!(
        code(&core.device_forget(DeviceRole::Trainer).await.unwrap_err()),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(
            &core
                .device_pair(String::new(), DeviceRole::Trainer)
                .await
                .unwrap_err()
        ),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        code(&core.workout_get("nope".into()).await.unwrap_err()),
        ErrorCode::NotFound
    );
    pair_both(&core).await;
    assert_eq!(
        code(&core.ride_load(Some("nope".into())).await.unwrap_err()),
        ErrorCode::NotFound
    );
    // A loaded ride blocks pairing and scanning; a ride in progress blocks loading.
    core.ride_load(Some(SENSOR_CHECK.into())).await.unwrap();
    assert_eq!(
        code(&core.devices_scan(None).await.unwrap_err()),
        ErrorCode::Conflict
    );
    assert_eq!(
        code(&core.device_forget(DeviceRole::HeartRate).await.unwrap_err()),
        ErrorCode::Conflict
    );
    core.ride_start().await.unwrap();
    assert_eq!(
        code(&core.ride_start().await.unwrap_err()),
        ErrorCode::Conflict
    );
    assert_eq!(
        code(&core.ride_load(None).await.unwrap_err()),
        ErrorCode::Conflict
    );
    // Forgetting is per role, and a new pairing replaces the old one.
    core.ride_end().await.unwrap();
    core.device_forget(DeviceRole::HeartRate).await.unwrap();
    let links = core.device_links().await.unwrap();
    assert!(links[0].device.is_some() && links[1].device.is_none());
    // Without a strap the ride still loads, and says so.
    assert!(!core.ride_load(None).await.unwrap().hr_linked);
}

#[tokio::test(start_paused = true)]
async fn streams_belong_to_a_window_and_die_with_their_sink() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path(), vec![])).unwrap();
    pair_both(&core).await;
    core.ride_load(Some("builtin-endurance-45".into()))
        .await
        .unwrap();
    core.ride_start().await.unwrap();

    // A reload of the window drops its earlier stream.
    let (a, b): (Frames, Frames) = Default::default();
    let first = core
        .open_stream(Some("main".into()), sink(&a))
        .await
        .unwrap();
    let second = core
        .open_stream(Some("main".into()), sink(&b))
        .await
        .unwrap();
    assert_eq!(
        code(&core.stream_snapshot(first).await.unwrap_err()),
        ErrorCode::NotFound
    );
    assert!(core.stream_snapshot(second).await.is_ok());

    // Destroying the window drops its streams.
    core.close_window("main".into()).await;
    assert_eq!(
        code(&core.stream_snapshot(second).await.unwrap_err()),
        ErrorCode::NotFound
    );

    // A failed send drops the stream.
    let dead = core
        .open_stream(Some("aux".into()), |_| Err(SinkClosed))
        .await
        .unwrap();
    sleep(Duration::from_secs(2)).await;
    assert_eq!(
        code(&core.stream_snapshot(dead).await.unwrap_err()),
        ErrorCode::NotFound
    );
    // Bad subscription input is InvalidInput.
    let live = core.open_stream(None, sink(&a)).await.unwrap();
    let bad = SubscribeOpts {
        rate_hz: 99.0,
        filters: Vec::new(),
    };
    assert_eq!(
        code(
            &core
                .subscribe(live, MetricId::Power, bad)
                .await
                .unwrap_err()
        ),
        ErrorCode::InvalidInput
    );
}

#[test]
fn a_second_core_on_the_same_data_dir_gets_a_typed_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let _first = Core::open(config(dir.path(), vec![])).unwrap();
    let err = Core::open(config(dir.path(), vec![]))
        .err()
        .expect("lock is held");
    assert_eq!(err.code, ErrorCode::Conflict);
    assert!(err.message.contains("ride.lock"), "{}", err.message);
}

/// Child half of the two-process test: only runs when the parent sets the data dir.
#[test]
fn lock_contender_child() {
    let Ok(dir) = std::env::var("CRANK_LOCK_CHILD_DIR") else {
        return;
    };
    let err = Core::open(config(Path::new(&dir), vec![]))
        .err()
        .expect("lock is held");
    assert_eq!(err.code, ErrorCode::Conflict);
}
#[test]
fn another_process_cannot_take_ride_lock() {
    let dir = tempfile::tempdir().unwrap();
    let _owner = Core::open(config(dir.path(), vec![])).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_contender_child", "--nocapture"])
        .env("CRANK_LOCK_CHILD_DIR", dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "child: {status}");
}

#[test]
fn api_error_wire_shape_is_code_and_message() {
    let e = ApiError::new(ErrorCode::DeviceUnavailable, "no trainer paired");
    assert_eq!(
        serde_json::to_value(&e).unwrap(),
        serde_json::json!({"code": "device_unavailable", "message": "no trainer paired"})
    );
}
