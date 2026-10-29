//! Window and quit lifecycle (ADR-0001 §4.3, ADR-0002 §7.9) and the startup failure text.
use crank_core::{Core, CoreConfig, DeviceMode, DeviceRole, ErrorCode, RideOutcome, Shutdown};
use crank_desktop::{
    commands as cmd,
    lifecycle::{self, Decision, ExitApi, Quit, Runner, Wire},
    startup_failure,
};
use crank_devices::sim::SimConfig;
use crank_engine::{StreamFrame, VirtualClock};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicI32, Ordering},
};
use std::{sync::Arc, time::Duration};
use tauri::{
    AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder,
    ipc::Channel,
    menu::{Menu, MenuEvent},
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
/// An app with a core and a window `main`. (Not built through [`lifecycle::install`]: its
/// menu can only be built on the main thread on macOS; the handlers are called directly.)
fn app(dir: &std::path::Path) -> tauri::App<MockRuntime> {
    let app = mock_builder()
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
    assert_eq!(quit.shut_down(&core).await, Shutdown::Ended);
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

/// A mid-ride core: both devices paired, a ride started and ridden for 20 s.
async fn mid_ride(dir: &std::path::Path) -> Core {
    let core = Core::open(config(dir)).expect("core opens");
    for (id, role) in [
        ("trainer", DeviceRole::Trainer),
        ("strap", DeviceRole::HeartRate),
    ] {
        core.device_pair(id.into(), role).await.expect("step");
    }
    core.ride_load(Some("builtin-sensor-check".into()))
        .await
        .expect("step");
    core.ride_start().await.expect("step");
    tokio::time::sleep(Duration::from_secs(20)).await;
    core
}
async fn outcomes(core: &Core) -> Vec<RideOutcome> {
    core.rides_list()
        .await
        .expect("rides")
        .iter()
        .map(|r| r.outcome)
        .collect()
}

// ---- the exit-request decision (kills "no prevent_exit", N2) ----------------------

#[tokio::test(start_paused = true)]
async fn the_first_exit_request_is_held_back_the_ride_ended_and_only_then_the_exit_called() {
    let dir = tempfile::tempdir().unwrap();
    let core = mid_ride(dir.path()).await;
    let quit = Arc::new(Quit::default());
    let prevented = Arc::new(AtomicBool::new(false));
    let exited = Arc::new(AtomicI32::new(-1));
    let (p, e) = (prevented.clone(), exited.clone());
    lifecycle::on_exit_requested(
        &quit,
        &core,
        Some(7),
        move || p.store(true, Ordering::SeqCst),
        move |status| e.store(status, Ordering::SeqCst),
    );
    assert!(prevented.load(Ordering::SeqCst), "the exit is held back");
    assert_eq!(
        exited.load(Ordering::SeqCst),
        -1,
        "not before the ride ends"
    );
    // The shutdown task runs on Tauri's runtime (real time) while the core's timers follow
    // this test's paused clock: advance the clock and give the other runtime real time,
    // until the exit is called (bounded, 10 s of real time).
    for _ in 0..2000 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if exited.load(Ordering::SeqCst) != -1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(exited.load(Ordering::SeqCst), 7, "the requested status");
    assert_eq!(outcomes(&core).await, [RideOutcome::Finished]);
    // The exit the app then requests itself goes through, and the backstop has nothing left.
    let prevented = AtomicBool::new(false);
    lifecycle::on_exit_requested(
        &quit,
        &core,
        Some(7),
        || prevented.store(true, Ordering::SeqCst),
        |_| panic!("no second shutdown"),
    );
    assert!(!prevented.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn an_exit_request_during_the_shutdown_is_held_back_without_a_second_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let core = mid_ride(dir.path()).await;
    let quit = Arc::new(Quit::default());
    assert_eq!(quit.request(), Decision::Begin);
    let prevented = AtomicBool::new(false);
    lifecycle::on_exit_requested(
        &quit,
        &core,
        None,
        || prevented.store(true, Ordering::SeqCst),
        |_| panic!("the shutdown is already under way"),
    );
    assert!(prevented.load(Ordering::SeqCst));
}

/// An `ExitRequested` api that records `prevent_exit` (only the event loop can make Tauri's).
#[derive(Default)]
struct FakeApi(AtomicBool);
impl ExitApi for FakeApi {
    fn prevent_exit(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// The run-event path holds the exit back and ends the ride on the async path, calling the
/// exit only after (kills `|| api.prevent_exit()` -> `|| {}`, #305: the exit would go
/// through and `Exit` would then shut down blocking the main thread).
#[tokio::test(start_paused = true)]
async fn the_run_event_path_prevents_the_exit_and_shuts_down_asynchronously() {
    let dir = tempfile::tempdir().unwrap();
    let core = mid_ride(dir.path()).await;
    let quit = Arc::new(Quit::default());
    let api = FakeApi::default();
    let exited = Arc::new(AtomicI32::new(-1));
    let e = exited.clone();
    lifecycle::handle_exit_requested(&quit, &core, Some(3), &api, move |status| {
        e.store(status, Ordering::SeqCst);
    });
    assert!(api.0.load(Ordering::SeqCst), "the exit is held back");
    assert_eq!(
        exited.load(Ordering::SeqCst),
        -1,
        "not before the ride ends"
    );
    for _ in 0..2000 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if exited.load(Ordering::SeqCst) != -1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(exited.load(Ordering::SeqCst), 3);
    assert_eq!(outcomes(&core).await, [RideOutcome::Finished]);
}

// ---- the backstop (`RunEvent::Exit` without a request) ----------------------------

#[tokio::test(start_paused = true)]
async fn an_exit_that_never_asked_ends_the_ride_synchronously() {
    let dir = tempfile::tempdir().unwrap();
    let core = mid_ride(dir.path()).await;
    let quit = Quit::default();
    assert!(quit.pending());
    // `Exit` is delivered synchronously from `applicationWillTerminate:`: nothing may be
    // left to run on a task after it returns.
    let (c, q) = (core.clone(), Arc::new(quit));
    let q2 = q.clone();
    tokio::task::spawn_blocking(move || lifecycle::on_exit(&q2, &c))
        .await
        .unwrap();
    assert_eq!(outcomes(&core).await, [RideOutcome::Finished]);
    assert!(!q.pending());
}

#[tokio::test(start_paused = true)]
async fn an_exit_after_the_shutdown_is_done_does_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let core = mid_ride(dir.path()).await;
    let quit = Arc::new(Quit::default());
    // Pretend the shutdown ran (on another, empty core): the ride here must be left alone.
    let other = Core::open(config(tempfile::tempdir().unwrap().path())).unwrap();
    quit.shut_down(&other).await;
    let (c, q) = (core.clone(), quit.clone());
    tokio::task::spawn_blocking(move || lifecycle::on_exit(&q, &c))
        .await
        .unwrap();
    assert!(
        core.ride_status().await.unwrap().status.loaded,
        "still riding"
    );
}

// ---- registration (kills "install() is a no-op", N1) ------------------------------

type MenuHandler = Box<dyn Fn(&AppHandle<MockRuntime>, MenuEvent) + Send + Sync>;

/// A builder that records which handlers were registered, and keeps the menu-event handler so
/// a test can call what `install` registered. (The menu builder itself can only run on the
/// main thread on macOS: `examples/quit_check.rs` calls it there.)
#[derive(Default)]
struct Recorder {
    names: Vec<&'static str>,
    menu_event: Option<MenuHandler>,
}
impl Wire<MockRuntime> for Recorder {
    fn on_page_load<
        F: Fn(&tauri::Webview<MockRuntime>, &tauri::webview::PageLoadPayload<'_>)
            + Send
            + Sync
            + 'static,
    >(
        mut self,
        _: F,
    ) -> Self {
        self.names.push("page_load");
        self
    }
    fn on_window_event<
        F: Fn(&tauri::Window<MockRuntime>, &tauri::WindowEvent) + Send + Sync + 'static,
    >(
        mut self,
        _: F,
    ) -> Self {
        self.names.push("window_event");
        self
    }
    fn menu<
        F: FnOnce(&AppHandle<MockRuntime>) -> tauri::Result<Menu<MockRuntime>> + Send + 'static,
    >(
        mut self,
        _: F,
    ) -> Self {
        self.names.push("menu");
        self
    }
    fn on_menu_event<F: Fn(&AppHandle<MockRuntime>, MenuEvent) + Send + Sync + 'static>(
        mut self,
        f: F,
    ) -> Self {
        self.names.push("menu_event");
        self.menu_event = Some(Box::new(f));
        self
    }
}

#[test]
fn install_registers_the_page_load_window_event_and_menu_handlers() {
    let r = lifecycle::install::<MockRuntime, _>(Recorder::default());
    assert_eq!(r.names, ["page_load", "window_event", "menu", "menu_event"]);
}

fn bare_app() -> tauri::App<MockRuntime> {
    mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app")
}

/// Cmd+Q is the menu item `install` registers with the Quit id; choosing it, through the
/// handler `install` registered, starts the exit with status 0 (kills a no-op
/// `on_menu_event`, #305).
#[test]
fn the_menu_event_handler_install_registers_exits_on_the_quit_item_only() {
    let app = bare_app();
    let exits = Arc::new(Mutex::new(Vec::new()));
    let log = exits.clone();
    let r = lifecycle::install_with::<MockRuntime, _>(Recorder::default(), move |_, code| {
        log.lock().expect("lock").push(code);
    });
    let handler = r.menu_event.expect("a menu event handler");
    handler(
        app.handle(),
        MenuEvent {
            id: "something-else".into(),
        },
    );
    assert!(exits.lock().unwrap().is_empty(), "other items do not exit");
    handler(
        app.handle(),
        MenuEvent {
            id: lifecycle::QUIT_ITEM_ID.into(),
        },
    );
    assert_eq!(*exits.lock().unwrap(), [0]);
}

/// An app that delivers one `Exit` to the callback, as `applicationWillTerminate:` does.
struct ExitOnly(AppHandle<MockRuntime>, Arc<Mutex<bool>>);
impl Runner<MockRuntime> for ExitOnly {
    fn run<F: FnMut(&AppHandle<MockRuntime>, RunEvent) + 'static>(self, mut callback: F) {
        callback(&self.0, RunEvent::Exit);
        *self.1.lock().expect("lock") = true;
    }
}

#[tokio::test(start_paused = true)]
async fn run_registers_the_run_event_handler_that_backstops_an_exit() {
    let dir = tempfile::tempdir().unwrap();
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    let core = mid_ride(dir.path()).await;
    app.manage(core.clone());
    let delivered = Arc::new(Mutex::new(false));
    let runner = ExitOnly(app.handle().clone(), delivered.clone());
    tokio::task::spawn_blocking(move || lifecycle::run(runner))
        .await
        .unwrap();
    assert!(*delivered.lock().unwrap());
    assert_eq!(outcomes(&core).await, [RideOutcome::Finished]);
}

// ---- the Quit menu item (the replacement itself is checked by `examples/quit_check.rs`) ----

#[test]
fn the_quit_menu_item_exits_with_status_zero_and_others_do_not() {
    let mut exits = Vec::new();
    lifecycle::menu_event(&"crank-quit".into(), |c| exits.push(c));
    lifecycle::menu_event(&"something-else".into(), |c| exits.push(c));
    assert_eq!(exits, [0]);
}
