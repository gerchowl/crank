//! The command layer (ADR-0001 §4) on the simulator: a whole ride driven through the
//! Tauri command functions, with the managed `Core` and a window of the mock runtime,
//! and frames arriving on a real IPC `Channel`.
use crank_core::{Core, CoreConfig, DeviceMode, DeviceRole, ErrorCode, RideOutcome};
use crank_desktop::commands as cmd;
use crank_devices::sim::SimConfig;
use crank_engine::{MetricId, StreamFrame, StreamRideState, SubscribeOpts, VirtualClock};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    ipc::{Channel, InvokeResponseBody},
    test::{MockRuntime, mock_builder, mock_context, noop_assets},
};

const SENSOR_CHECK: &str = "builtin-sensor-check";

type Frames = Arc<Mutex<Vec<StreamFrame>>>;

fn app(dir: &std::path::Path) -> tauri::App<MockRuntime> {
    let core = Core::open(CoreConfig {
        data_dir: dir.into(),
        devices: DeviceMode::Sim(SimConfig::default()),
        clock: Some(Arc::new(VirtualClock::new(1))),
    })
    .expect("core opens");
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    app.manage(core);
    for label in ["main", "other"] {
        WebviewWindowBuilder::new(&app, label, WebviewUrl::default())
            .build()
            .expect("window");
    }
    app
}
fn win(app: &tauri::App<MockRuntime>, label: &str) -> tauri::Window<MockRuntime> {
    app.get_webview_window(label)
        .expect("window")
        .as_ref()
        .window()
}
/// A channel that decodes what the core sends, as the page would.
fn channel(frames: &Frames) -> Channel<StreamFrame> {
    let frames = frames.clone();
    Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("frames are JSON");
        };
        frames
            .lock()
            .expect("test mutex")
            .push(serde_json::from_str(&json).expect("a StreamFrame"));
        Ok(())
    })
}

#[tokio::test(start_paused = true)]
async fn a_whole_ride_through_the_commands_streams_frames_and_lands_in_history() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let core = app.state::<Core>();
    let window = win(&app, "main");

    let info = cmd::core_ready(core.clone()).await.unwrap();
    assert!(info.sim);
    let found = cmd::devices_scan(core.clone(), None).await.unwrap();
    let trainer = found.iter().find(|d| d.trainer).unwrap().id.clone();
    let strap = found.iter().find(|d| d.heart_rate).unwrap().id.clone();
    cmd::device_pair(core.clone(), trainer, DeviceRole::Trainer)
        .await
        .unwrap();
    cmd::device_pair(core.clone(), strap, DeviceRole::HeartRate)
        .await
        .unwrap();
    assert_eq!(cmd::device_links(core.clone()).await.unwrap().len(), 2);
    let library = cmd::workouts_list(core.clone()).await.unwrap();
    assert!(library.iter().any(|w| w.id == SENSOR_CHECK));
    cmd::workout_get(core.clone(), SENSOR_CHECK.into())
        .await
        .unwrap();

    cmd::ride_load(core.clone(), Some(SENSOR_CHECK.into()))
        .await
        .unwrap();
    let frames: Frames = Arc::default();
    let stream = cmd::open_stream(core.clone(), window.clone(), channel(&frames))
        .await
        .unwrap();
    let sub = cmd::stream_subscribe(
        core.clone(),
        window.clone(),
        stream,
        MetricId::Power,
        SubscribeOpts {
            rate_hz: 1.0,
            filters: Vec::new(),
        },
    )
    .await
    .unwrap();
    cmd::ride_start(core.clone()).await.unwrap();

    // Ride, acknowledging as the page does.
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let last = frames.lock().unwrap().last().map(|f| f.seq);
        if let Some(seq) = last {
            cmd::stream_ack(core.clone(), window.clone(), stream, seq)
                .await
                .unwrap();
        }
    }
    {
        let frames = frames.lock().unwrap();
        assert!(frames.len() >= 10, "{} frames", frames.len());
        assert!(
            frames
                .iter()
                .flat_map(|f| &f.samples)
                .any(|s| s.sub == sub && s.v.is_some_and(|w| w > 0.0)),
            "power samples arrive"
        );
    }
    let view = cmd::ride_status(core.clone()).await.unwrap();
    assert_eq!(view.status.state, StreamRideState::Riding);

    // A window may only use its own streams.
    let other = win(&app, "other");
    let e = cmd::stream_snapshot(core.clone(), other, stream)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    let snapshot = cmd::stream_snapshot(core.clone(), window.clone(), stream)
        .await
        .unwrap();
    assert!(snapshot.status.is_some());

    cmd::ride_pause(core.clone()).await.unwrap();
    cmd::ride_resume(core.clone()).await.unwrap();
    cmd::ride_skip(core.clone()).await.unwrap();
    cmd::stream_unsubscribe(core.clone(), window.clone(), stream, sub)
        .await
        .unwrap();
    let id = cmd::ride_end(core.clone()).await.unwrap().expect("a ride");

    let rides = cmd::rides_list(core.clone()).await.unwrap();
    assert_eq!(rides.len(), 1);
    assert_eq!(rides[0].outcome, RideOutcome::Finished);
    assert_eq!(cmd::ride_summary(core.clone(), id).await.unwrap(), rides[0]);
    cmd::device_forget(core.clone(), DeviceRole::HeartRate)
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_destroyed_window_loses_its_streams() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let core = app.state::<Core>();
    let window = win(&app, "main");
    for (id, role) in [("trainer", DeviceRole::Trainer)] {
        cmd::device_pair(core.clone(), id.into(), role)
            .await
            .unwrap();
    }
    cmd::ride_load(core.clone(), None).await.unwrap();
    let frames: Frames = Arc::default();
    let stream = cmd::open_stream(core.clone(), window.clone(), channel(&frames))
        .await
        .unwrap();
    core.close_window("main".into()).await;
    let e = cmd::stream_snapshot(core.clone(), window, stream)
        .await
        .unwrap_err();
    assert_eq!(e.code, ErrorCode::NotFound);
    cmd::ride_end(core.clone()).await.unwrap();
}
