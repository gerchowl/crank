//! Window and quit lifecycle (ADR-0001 §4.3, ADR-0002 §7.9) and the startup failure text.
use crank_core::{Core, CoreConfig, DeviceMode, DeviceRole, ErrorCode, RideOutcome, Shutdown};
use crank_desktop::{
    commands as cmd,
    lifecycle::{self, Decision, Quit},
    startup_failure,
};
use crank_devices::sim::SimConfig;
use crank_engine::{StreamFrame, VirtualClock};
use std::{sync::Arc, time::Duration};
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    ipc::Channel,
    test::{MockRuntime, mock_builder, mock_context, noop_assets},
    webview::PageLoadEvent,
};

fn config(dir: &std::path::Path) -> CoreConfig {
    CoreConfig {
        data_dir: dir.into(),
        devices: DeviceMode::Sim(SimConfig::default()),
        clock: Some(Arc::new(VirtualClock::new(1))),
    }
}
/// An app built through [`lifecycle::install`], with a core and a window `main`.
fn app(dir: &std::path::Path) -> tauri::App<MockRuntime> {
    let app = lifecycle::install(mock_builder())
        .build(mock_context(noop_assets()))
        .expect("mock app");
    app.manage(Core::open(config(dir)).expect("core opens"));
    // Linux defaults the webview data dir under $HOME, which a build sandbox does not own.
    WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .data_directory(dir.join("webview"))
        .build()
        .expect("window");
    app
}
fn win(app: &tauri::App<MockRuntime>) -> tauri::Window<MockRuntime> {
    app.get_webview_window("main")
        .expect("window")
        .as_ref()
        .window()
}
/// Pair, load and open a stream for window `main`.
async fn streaming(app: &tauri::App<MockRuntime>) -> crank_engine::StreamId {
    let core = app.state::<Core>();
    cmd::device_pair(core.clone(), "trainer".into(), DeviceRole::Trainer)
        .await
        .expect("command");
    cmd::ride_load(core.clone(), None).await.expect("command");
    cmd::open_stream(core, win(app), Channel::<StreamFrame>::new(|_| Ok(())))
        .await
        .expect("command")
}
/// Whether `main` still owns `stream`, waiting for the (async) handlers to have run.
async fn owns(app: &tauri::App<MockRuntime>, stream: crank_engine::StreamId) -> bool {
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let r = cmd::stream_snapshot(app.state::<Core>(), win(app), stream).await;
        if r.is_err_and(|e| e.code == ErrorCode::NotFound) {
            return false;
        }
    }
    true
}

#[tokio::test]
async fn a_destroyed_window_loses_its_streams() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let stream = streaming(&app).await;
    assert!(owns_now(&app, stream).await);
    // The mock runtime emits no `Destroyed`; the handler the builder registers is called
    // with the real window, as the event loop does.
    lifecycle::window_event(&win(&app), &tauri::WindowEvent::Focused(true));
    assert!(owns_now(&app, stream).await, "other events keep the stream");
    lifecycle::window_event(&win(&app), &tauri::WindowEvent::Destroyed);
    assert!(
        !owns(&app, stream).await,
        "the Destroyed handler dropped the stream"
    );
}
async fn owns_now(app: &tauri::App<MockRuntime>, stream: crank_engine::StreamId) -> bool {
    cmd::stream_snapshot(app.state::<Core>(), win(app), stream)
        .await
        .is_ok()
}

#[tokio::test]
async fn a_page_load_drops_the_windows_streams_a_finished_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path());
    let stream = streaming(&app).await;
    lifecycle::page_load(app.handle(), "main", PageLoadEvent::Finished);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(owns_now(&app, stream).await, "Finished keeps the stream");
    lifecycle::page_load(app.handle(), "other", PageLoadEvent::Started);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        owns_now(&app, stream).await,
        "another window's reload keeps it"
    );
    lifecycle::page_load(app.handle(), "main", PageLoadEvent::Started);
    assert!(!owns(&app, stream).await, "a reload of the window drops it");
}

#[test]
fn the_first_exit_request_begins_the_shutdown_and_later_ones_wait_until_it_is_done() {
    let quit = Quit::default();
    assert_eq!(quit.request(), Decision::Begin);
    assert_eq!(quit.request(), Decision::Wait);
    assert_eq!(quit.request(), Decision::Wait);
}

#[tokio::test(start_paused = true)]
async fn quitting_mid_ride_ends_and_records_the_ride_before_the_exit_is_let_through() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path())).unwrap();
    for (id, role) in [
        ("trainer", DeviceRole::Trainer),
        ("strap", DeviceRole::HeartRate),
    ] {
        core.device_pair(id.into(), role).await.unwrap();
    }
    core.ride_load(Some("builtin-sensor-check".into()))
        .await
        .unwrap();
    core.ride_start().await.unwrap();
    tokio::time::sleep(Duration::from_secs(20)).await;

    let quit = Quit::default();
    assert_eq!(quit.request(), Decision::Begin);
    assert_eq!(quit.shut_down(&core).await, Shutdown::Released);
    assert_eq!(quit.request(), Decision::Proceed, "the app may exit now");
    // The ride was ended and its devices are free again.
    let rides = core.rides_list().await.unwrap();
    assert_eq!(rides.len(), 1);
    assert_eq!(rides[0].outcome, RideOutcome::Finished);
    core.device_forget(DeviceRole::Trainer).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn quitting_with_no_ride_does_not_wait() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path())).unwrap();
    let quit = Quit::default();
    assert_eq!(quit.shut_down(&core).await, Shutdown::Idle);
    assert_eq!(quit.request(), Decision::Proceed);
}

#[tokio::test]
async fn a_second_instance_is_told_so_instead_of_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let _first = Core::open(config(dir.path())).unwrap();
    let e = Core::open(config(dir.path()))
        .err()
        .expect("the lock is held");
    assert_eq!(e.code, ErrorCode::Conflict);
    assert_eq!(startup_failure(&e), "crank is already running.");
    let other = crank_core::ApiError::new(ErrorCode::Storage, "disk full");
    assert_eq!(startup_failure(&other), "crank could not start: disk full");
}
